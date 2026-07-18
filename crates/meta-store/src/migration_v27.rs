use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use fs4::fs_std::FileExt;
use rusqlite::{backup::Backup, Connection};

use super::{
    apply_sqlcipher_key, encode_hex, remove_sqlite_sidecars, restrict_private_file_permissions,
    schema_v27, MetaStore, MetaStoreError, MetadataEncryptionState, Result,
    METADATA_ENCRYPTION_KEY_LEN, METADATA_STORE_FILE,
};

const MIGRATION_LOCK_FILE: &str = "metadata-migration.lock";

mod allowlist;
mod cleanup;
mod manifest;
mod store_validation;

use allowlist::{copy_allowed_legacy_state, validate_allowlist_inventory};
use cleanup::{
    complete_legacy_cleanup, discard_unpublished_cleanup, remove_owner_file_if_exists,
    write_legacy_cleanup_receipt, LEGACY_CLEANUP_RECEIPT_FILE,
};
use manifest::{
    publish_manifest, random_store_id_digest, read_manifest, ActiveStoreManifest, MANIFEST_FILE,
};
use store_validation::{
    open_encrypted_connection, source_schema_version, store_identity, validate_active_store,
    validate_staging_store,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationFailpoint {
    None,
    AfterBackup,
    AfterMigration,
    BeforeManifest,
    AfterManifestRename,
    AfterManifest,
    AfterLegacyMainDelete,
    AfterLegacySidecarDelete,
}

pub(super) fn active_store_path(data_dir: &Path) -> Result<PathBuf> {
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if !owner_regular_file_exists(&manifest_path)? {
        return Ok(data_dir.join(METADATA_STORE_FILE));
    }
    Ok(data_dir.join(read_manifest(&manifest_path)?.file_name))
}

#[cfg(test)]
pub(super) fn ensure_active_v27_store(data_dir: &Path, key: &[u8]) -> Result<PathBuf> {
    ensure_active_v27_store_with_failpoint(data_dir, key, MigrationFailpoint::None)
}

pub(super) fn prepare_active_v27_store(
    data_dir: &Path,
) -> Result<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN])> {
    with_migration_lock(data_dir, || {
        let key = super::load_or_create_metadata_encryption_key(data_dir)?;
        let path = ensure_active_v27_store_locked(data_dir, &key, MigrationFailpoint::None)?;
        Ok((path, key))
    })
}

#[cfg(test)]
fn ensure_active_v27_store_with_failpoint(
    data_dir: &Path,
    key: &[u8],
    failpoint: MigrationFailpoint,
) -> Result<PathBuf> {
    with_migration_lock(data_dir, || {
        ensure_active_v27_store_locked(data_dir, key, failpoint)
    })
}

fn with_migration_lock<T>(data_dir: &Path, operation: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock = acquire_migration_lock(data_dir)?;
    let result = operation();
    let unlock = FileExt::unlock(&lock).map_err(MetaStoreError::io_storage);
    match result {
        Ok(value) => {
            unlock?;
            Ok(value)
        }
        Err(error) => {
            let _ = unlock;
            Err(error)
        }
    }
}

fn ensure_active_v27_store_locked(
    data_dir: &Path,
    key: &[u8],
    failpoint: MigrationFailpoint,
) -> Result<PathBuf> {
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if owner_regular_file_exists(&manifest_path)? {
        let manifest = read_manifest(&manifest_path)?;
        let active_path = data_dir.join(&manifest.file_name);
        validate_active_store(&active_path, key, &manifest.store_id_digest)?;
        complete_legacy_cleanup(data_dir, &manifest, failpoint)?;
        return Ok(active_path);
    }
    discard_unpublished_cleanup(data_dir)?;

    let legacy_path = data_dir.join(METADATA_STORE_FILE);
    let legacy_exists = owner_regular_file_exists(&legacy_path)?;
    let source_is_plaintext =
        legacy_exists && super::metadata_store_has_plaintext_header(&legacy_path)?;
    let mut source = if legacy_exists {
        if source_is_plaintext {
            Connection::open(&legacy_path).map_err(MetaStoreError::storage)?
        } else {
            open_encrypted_connection(&legacy_path, key)?
        }
    } else {
        Connection::open_in_memory().map_err(MetaStoreError::storage)?
    };
    let version = source_schema_version(&source)?;
    if version == schema_v27::VERSION && !source_is_plaintext {
        let store_id_digest = store_identity(&source)?;
        drop(source);
        validate_active_store(&legacy_path, key, &store_id_digest)?;
        sync_validated_store(&legacy_path)?;
        publish_manifest(data_dir, METADATA_STORE_FILE, &store_id_digest, failpoint)?;
        return Ok(legacy_path);
    }
    if version > schema_v27::VERSION {
        return Err(MetaStoreError::invalid_value("metadata.schema_version"));
    }

    let store_id_digest = if version == schema_v27::VERSION {
        store_identity(&source)?
    } else {
        random_store_id_digest()?
    };
    let target_file_name = format!("metadata-v27-{}.sqlite3", &store_id_digest[..16]);
    let target_path = data_dir.join(&target_file_name);
    if owner_regular_file_exists(&target_path)? {
        return Err(MetaStoreError::storage_invariant());
    }

    let migration = if version == 0 {
        create_fresh_v27_store(&target_path, key, &store_id_digest, failpoint)
    } else if version == schema_v27::VERSION {
        copy_v27_store(
            &source,
            &target_path,
            key,
            &store_id_digest,
            source_is_plaintext,
            failpoint,
        )
    } else {
        migrate_allowlisted(&mut source, &target_path, key, &store_id_digest, failpoint)
    };
    drop(source);
    if let Err(error) = migration {
        let _ = fs::remove_file(&target_path);
        remove_sqlite_sidecars(&target_path);
        return Err(error);
    }
    if let Err(error) = sync_validated_store(&target_path) {
        let _ = fs::remove_file(&target_path);
        remove_sqlite_sidecars(&target_path);
        return Err(error);
    }
    let cleanup_receipt = legacy_exists && target_path != legacy_path;
    if cleanup_receipt {
        write_legacy_cleanup_receipt(data_dir, &target_file_name, &store_id_digest)?;
    }
    if failpoint == MigrationFailpoint::BeforeManifest {
        let _ = fs::remove_file(&target_path);
        remove_sqlite_sidecars(&target_path);
        if cleanup_receipt {
            let _ = fs::remove_file(data_dir.join(LEGACY_CLEANUP_RECEIPT_FILE));
        }
        return Err(MetaStoreError::storage_invariant());
    }
    if let Err(error) = publish_manifest(data_dir, &target_file_name, &store_id_digest, failpoint) {
        // A rename is the commit point. Permission tightening or directory
        // synchronization can fail after the pointer has become visible. In
        // that uncertain-commit state the target and cleanup receipt must stay
        // intact so the next locked opener can validate and finish cleanup.
        if fs::symlink_metadata(&manifest_path).is_err() {
            let _ = fs::remove_file(&target_path);
            remove_sqlite_sidecars(&target_path);
            if cleanup_receipt {
                let _ = remove_owner_file_if_exists(&data_dir.join(LEGACY_CLEANUP_RECEIPT_FILE));
                let _ = sync_parent_directory(data_dir);
            }
        }
        return Err(error);
    }
    if failpoint == MigrationFailpoint::AfterManifest {
        return Err(MetaStoreError::storage_invariant());
    }
    complete_legacy_cleanup(
        data_dir,
        &ActiveStoreManifest {
            file_name: target_file_name,
            store_id_digest,
        },
        failpoint,
    )?;
    Ok(target_path)
}

fn sync_validated_store(path: &Path) -> Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(MetaStoreError::io_storage)?
        .sync_all()
        .map_err(MetaStoreError::io_storage)
}

fn copy_v27_store(
    source: &Connection,
    target_path: &Path,
    key: &[u8],
    store_id_digest: &str,
    source_is_plaintext: bool,
    failpoint: MigrationFailpoint,
) -> Result<()> {
    copy_to_encrypted_target(source, target_path, key, source_is_plaintext)?;
    if failpoint == MigrationFailpoint::AfterBackup {
        return Err(MetaStoreError::storage_invariant());
    }
    let connection = open_encrypted_connection(target_path, key)?;
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(MetaStoreError::storage)?;
    drop(connection);
    restrict_private_file_permissions(target_path)?;
    validate_active_store(target_path, key, store_id_digest)?;
    if failpoint == MigrationFailpoint::AfterMigration {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn create_fresh_v27_store(
    target_path: &Path,
    key: &[u8],
    store_id_digest: &str,
    failpoint: MigrationFailpoint,
) -> Result<()> {
    if failpoint == MigrationFailpoint::AfterBackup {
        return Err(MetaStoreError::storage_invariant());
    }
    create_empty_v27_target(target_path, key, store_id_digest)?;
    if failpoint == MigrationFailpoint::AfterMigration {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn create_empty_v27_target(target_path: &Path, key: &[u8], store_id_digest: &str) -> Result<()> {
    create_private_empty_file(target_path)?;
    let connection = Connection::open(target_path).map_err(MetaStoreError::storage)?;
    apply_sqlcipher_key(&connection, key)?;
    let store = MetaStore::from_connection(connection, true, MetadataEncryptionState::SqlCipher)?;
    store.migrate_staging_store_to_v27(store_id_digest)?;
    store
        .connection
        .borrow()
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(MetaStoreError::storage)?;
    validate_staging_store(&store, store_id_digest)?;
    drop(store);
    restrict_private_file_permissions(target_path)?;
    Ok(())
}

fn migrate_allowlisted(
    source: &mut Connection,
    target_path: &Path,
    key: &[u8],
    store_id_digest: &str,
    failpoint: MigrationFailpoint,
) -> Result<()> {
    create_empty_v27_target(target_path, key, store_id_digest)?;
    if failpoint == MigrationFailpoint::AfterBackup {
        return Err(MetaStoreError::storage_invariant());
    }
    let expected_inventory = copy_allowed_legacy_state(source, target_path, key)?;
    let connection = open_encrypted_connection(target_path, key)?;
    let store = MetaStore::from_connection(connection, true, MetadataEncryptionState::SqlCipher)?;
    validate_staging_store(&store, store_id_digest)?;
    validate_allowlist_inventory(&store.connection.borrow(), &expected_inventory)?;
    drop(store);
    if failpoint == MigrationFailpoint::AfterMigration {
        return Err(MetaStoreError::storage_invariant());
    }
    restrict_private_file_permissions(target_path)?;
    Ok(())
}

fn copy_to_encrypted_target(
    source: &Connection,
    target_path: &Path,
    key: &[u8],
    source_is_plaintext: bool,
) -> Result<()> {
    create_private_empty_file(target_path)?;
    if source_is_plaintext {
        return export_plaintext_to_encrypted_target(source, target_path, key);
    }
    let mut target = Connection::open(target_path).map_err(MetaStoreError::storage)?;
    apply_sqlcipher_key(&target, key)?;
    {
        let backup = Backup::new(source, &mut target).map_err(MetaStoreError::storage)?;
        backup
            .run_to_completion(128, Duration::from_millis(1), None)
            .map_err(MetaStoreError::storage)?;
    }
    drop(target);
    Ok(())
}

fn create_private_empty_file(path: &Path) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(MetaStoreError::io_storage)?;
    file.sync_all().map_err(MetaStoreError::io_storage)?;
    restrict_private_file_permissions(path)
}

fn export_plaintext_to_encrypted_target(
    source: &Connection,
    target_path: &Path,
    key: &[u8],
) -> Result<()> {
    let target_literal = sql_string_literal(target_path)?;
    let key_hex = encode_hex(key);
    source
        .execute_batch(&format!(
            "\
            ATTACH DATABASE {target_literal} AS encrypted KEY \"x'{key_hex}'\";
            SELECT sqlcipher_export('encrypted');
            DETACH DATABASE encrypted;
            "
        ))
        .map_err(MetaStoreError::storage)
}

fn sql_string_literal(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.store_path"))?;
    Ok(format!("'{}'", value.replace('\'', "''")))
}

fn acquire_migration_lock(data_dir: &Path) -> Result<File> {
    let path = data_dir.join(MIGRATION_LOCK_FILE);
    if path.try_exists().map_err(MetaStoreError::io_storage)? {
        let metadata = fs::symlink_metadata(&path).map_err(MetaStoreError::io_storage)?;
        validate_owner_regular_metadata(&metadata)?;
    }

    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options.open(&path).map_err(MetaStoreError::io_storage)?;
    FileExt::lock_exclusive(&lock).map_err(MetaStoreError::io_storage)?;
    restrict_private_file_permissions(&path)?;
    let metadata = fs::symlink_metadata(&path).map_err(MetaStoreError::io_storage)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        let _ = FileExt::unlock(&lock);
        return Err(MetaStoreError::invalid_value("metadata.migration_lock"));
    }
    Ok(lock)
}

fn owner_regular_file_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_owner_regular_metadata(&metadata)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(MetaStoreError::io_storage(error)),
    }
}

fn validate_owner_regular_metadata(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(MetaStoreError::invalid_value("metadata.owner_file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(MetaStoreError::invalid_value(
                "metadata.owner_file_permissions",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(data_dir: &Path) -> Result<()> {
    let directory = fs::File::open(data_dir).map_err(MetaStoreError::io_storage)?;
    directory.sync_all().map_err(MetaStoreError::io_storage)
}

#[cfg(not(unix))]
fn sync_parent_directory(_data_dir: &Path) -> Result<()> {
    // The manifest file itself is flushed before the atomic rename. Rust's
    // standard library cannot open a Windows directory for FlushFileBuffers;
    // attempting `File::open` here would make every v27 publication fail.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{legacy_migrations, migration_applied};

    const LEGACY_PII_MARKER: &str = "SYNTHETIC_V26_PRIVATE_DERIVED_MARKER_7f4e";

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let mut random = [0_u8; 8];
            getrandom::getrandom(&mut random).unwrap();
            let path = std::env::temp_dir()
                .join(format!("resume-ir-s807-{label}-{}", encode_hex(&random)));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct LegacyFixture {
        directory: TestDirectory,
        path: PathBuf,
        key: [u8; 32],
    }

    impl LegacyFixture {
        fn create(label: &str) -> Self {
            Self::create_with_encryption(label, true)
        }

        fn create_plaintext(label: &str) -> Self {
            Self::create_with_encryption(label, false)
        }

        fn create_with_encryption(label: &str, encrypted: bool) -> Self {
            let directory = TestDirectory::new(label);
            let path = directory.path().join(METADATA_STORE_FILE);
            let mut key = [0_u8; 32];
            getrandom::getrandom(&mut key).unwrap();
            let mut connection = Connection::open(&path).unwrap();
            if encrypted {
                apply_sqlcipher_key(&connection, &key).unwrap();
            }
            connection
                .execute_batch(
                    "CREATE TABLE schema_migrations (
                        version INTEGER PRIMARY KEY,
                        applied_at_seconds INTEGER NOT NULL
                    );",
                )
                .unwrap();
            for (version, schema) in legacy_migrations() {
                if !migration_applied(&connection, version).unwrap() {
                    let transaction = connection.transaction().unwrap();
                    transaction.execute_batch(schema).unwrap();
                    transaction
                        .execute(
                            "INSERT INTO schema_migrations (version, applied_at_seconds)
                             VALUES (?1, 0)",
                            [i64::from(version)],
                        )
                        .unwrap();
                    transaction.commit().unwrap();
                }
            }
            connection
                .execute_batch(&format!(
                    "INSERT INTO document (
                        id, source_uri, normalized_path, file_name, extension, byte_size,
                        mtime_seconds, content_hash, text_hash, is_deleted,
                        created_at_seconds, updated_at_seconds, status
                     ) VALUES (
                        'doc_00000000000000000000000000000001', 'synthetic://legacy',
                        'synthetic/legacy.txt', 'legacy.txt', 'txt', 10, 1,
                        'sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                        'sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                        0, 1, 1, 'searchable'
                     );
                     INSERT INTO resume_version (
                        id, document_id, candidate_id, parse_version, schema_version,
                        language_set_json, page_count, raw_text, clean_text, quality_score,
                        visibility
                     ) VALUES (
                        'ver_00000000000000000000000000000001',
                        'doc_00000000000000000000000000000001', NULL,
                        'parser-v26', 'schema-v26', '[]', 1, '{LEGACY_PII_MARKER} raw',
                        '{LEGACY_PII_MARKER} clean', 0.8, 'searchable'
                     );
                     INSERT INTO entity_mention (
                        id, resume_version_id, section_id, entity_type, raw_value,
                        normalized_value, span_start, span_end, confidence, extractor
                     ) VALUES (
                        'ent_00000000000000000000000000000001',
                        'ver_00000000000000000000000000000001', NULL, 'skill',
                        '{LEGACY_PII_MARKER}', '{LEGACY_PII_MARKER}', 0, 6, 0.9, 'legacy-v26'
                     );
                     INSERT INTO ingest_job (
                        id, document_id, resume_version_id, kind, status, attempt_count,
                        max_attempts, queued_at_seconds, started_at_seconds,
                        finished_at_seconds, updated_at_seconds, failure_kind
                     ) VALUES (
                        'job_00000000000000000000000000000001',
                        'doc_00000000000000000000000000000001',
                        'ver_00000000000000000000000000000001', 'parse_document',
                        'completed', 1, 3, 1, 1, 1, 1, NULL
                     );
                     INSERT INTO candidate (
                        id, primary_name, phone_hash, email_hash, version_count
                     ) VALUES (
                        'cand_00000000000000000000000000000001', '{LEGACY_PII_MARKER}',
                        'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                        'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', 0
                     );
                     INSERT INTO ocr_page_cache (
                        file_content_hash, page_no, render_dpi, ocr_lang, ocr_profile,
                        text, confidence, engine_profile, duration_ms, status, error_kind,
                        updated_at_seconds, word_boxes_json
                     ) VALUES (
                        'sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc',
                        1, 144, 'eng', 'synthetic', '{LEGACY_PII_MARKER}', 0.9,
                        'synthetic', 1, 'succeeded', NULL, 1,
                        '[{LEGACY_PII_MARKER}]'
                     );
                     INSERT INTO worker_task_control (task_kind, paused, updated_at_seconds)
                     VALUES ('ocr', 1, 2);
                     INSERT INTO import_task (
                        id, root_path, status, queued_at_seconds, started_at_seconds,
                        finished_at_seconds, updated_at_seconds
                     ) VALUES (
                        'import_000000000000000000000000000001', '/synthetic/authorized',
                        'completed', 1, 1, 1, 1
                     );
                     INSERT INTO import_scan_scope (
                        import_task_id, root_kind, root_preset, scan_profile,
                        requested_root_path, canonical_root_path, updated_at_seconds,
                        scan_budget_kind, scan_budget_limit
                     ) VALUES (
                        'import_000000000000000000000000000001', 'explicit', NULL,
                        'explicit', '/synthetic/requested', '/synthetic/authorized', 3,
                        'files', 42
                     );
                     INSERT INTO import_root_control (
                        canonical_root_path, paused, updated_at_seconds
                     ) VALUES ('/synthetic/authorized', 1, 4);
                     INSERT INTO query_observation (
                        observed_at_seconds, mode, duration_ms, result_count
                     ) VALUES (1, 'fulltext', 1, 1);
                     INSERT INTO index_state (
                        state_key, manifest_version, snapshot_token, status,
                        updated_at_seconds, visible_epoch, manifest_document_count
                     ) VALUES ('default', 'legacy-v26', 'legacy-generation', 'ready', 1, 9, 1);"
                ))
                .unwrap();
            drop(connection);
            restrict_private_file_permissions(&path).unwrap();
            Self {
                directory,
                path,
                key,
            }
        }

        fn snapshot(&self) -> (Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>) {
            (
                fs::read(&self.path).unwrap(),
                read_optional_file(&PathBuf::from(format!("{}-wal", self.path.display()))),
                read_optional_file(&PathBuf::from(format!("{}-shm", self.path.display()))),
            )
        }
    }

    fn read_optional_file(path: &Path) -> Option<Vec<u8>> {
        fs::read(path).ok()
    }

    fn assert_legacy_artifacts_retired(fixture: &LegacyFixture) {
        for path in [
            fixture.path.clone(),
            PathBuf::from(format!("{}-wal", fixture.path.display())),
            PathBuf::from(format!("{}-shm", fixture.path.display())),
            fixture.directory.path().join(LEGACY_CLEANUP_RECEIPT_FILE),
        ] {
            assert!(
                !path.exists(),
                "legacy artifact remained: {}",
                path.display()
            );
        }
    }

    fn assert_no_retained_private_marker(directory: &Path) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                let bytes = fs::read(entry.path()).unwrap();
                assert!(
                    !bytes
                        .windows(LEGACY_PII_MARKER.len())
                        .any(|window| window == LEGACY_PII_MARKER.as_bytes()),
                    "derived marker remained in {}",
                    entry.path().display()
                );
            }
        }
    }

    #[test]
    fn copy_on_write_failpoints_leave_v26_bytes_and_pointer_unchanged() {
        for failpoint in [
            MigrationFailpoint::AfterBackup,
            MigrationFailpoint::AfterMigration,
            MigrationFailpoint::BeforeManifest,
        ] {
            let fixture = LegacyFixture::create(&format!("fail-{failpoint:?}"));
            let before = fixture.snapshot();
            let error = ensure_active_v27_store_with_failpoint(
                fixture.directory.path(),
                &fixture.key,
                failpoint,
            )
            .unwrap_err();
            assert_eq!(error.class(), crate::MetaStoreErrorClass::StorageInvariant);
            assert_eq!(fixture.snapshot(), before);
            assert!(!fixture.directory.path().join(MANIFEST_FILE).exists());
            let targets = fs::read_dir(fixture.directory.path())
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("metadata-v27-")
                })
                .count();
            assert_eq!(targets, 0);
        }
    }

    #[test]
    fn validated_pointer_switch_retires_legacy_and_preserves_only_allowlisted_identity() {
        let fixture = LegacyFixture::create("success");
        let active = ensure_active_v27_store(fixture.directory.path(), &fixture.key).unwrap();
        assert_ne!(active, fixture.path);
        assert_eq!(active_store_path(fixture.directory.path()).unwrap(), active);
        assert_legacy_artifacts_retired(&fixture);
        assert_no_retained_private_marker(fixture.directory.path());

        let connection = open_encrypted_connection(&active, &fixture.key).unwrap();
        assert_eq!(source_schema_version(&connection).unwrap(), 27);
        let counts = connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM ingest_job),
                    (SELECT COUNT(*) FROM resume_version),
                    (SELECT COUNT(*) FROM entity_mention),
                    (SELECT COUNT(*) FROM active_search_projection),
                    (SELECT COUNT(*) FROM candidate),
                    (SELECT COUNT(*) FROM ocr_page_cache),
                    (SELECT COUNT(*) FROM import_task),
                    (SELECT COUNT(*) FROM import_scan_scope),
                    (SELECT COUNT(*) FROM worker_task_control),
                    (SELECT COUNT(*) FROM query_observation)",
                [],
                |row| {
                    (0..10)
                        .map(|index| row.get::<_, i64>(index))
                        .collect::<std::result::Result<Vec<_>, _>>()
                },
            )
            .unwrap();
        assert_eq!(counts, vec![0; 10]);
        let document = connection
            .query_row(
                "SELECT status, content_hash, text_hash FROM document",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(document, ("discovered".to_string(), None, None));
        let authorized_root = connection
            .query_row(
                "SELECT requested_root_path, canonical_root_path, root_kind, scan_profile,
                        scan_budget_kind, scan_budget_limit, paused, updated_at_seconds
                 FROM authorized_import_root",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            authorized_root,
            (
                "/synthetic/requested".to_string(),
                "/synthetic/authorized".to_string(),
                "explicit".to_string(),
                "explicit".to_string(),
                Some("files".to_string()),
                Some(42),
                1,
                4,
            )
        );
        let state = connection
            .query_row(
                "SELECT service_state, generation, repair_reason
                 FROM search_projection_state",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            state,
            (
                "repairing".to_string(),
                None,
                Some("migration_rebuild".to_string())
            )
        );
    }

    #[test]
    fn plaintext_v26_is_rebuilt_into_encrypted_v27_and_physically_retired() {
        let fixture = LegacyFixture::create_plaintext("plaintext-cow");

        let active = ensure_active_v27_store(fixture.directory.path(), &fixture.key).unwrap();

        assert_ne!(active, fixture.path);
        assert_legacy_artifacts_retired(&fixture);
        assert_no_retained_private_marker(fixture.directory.path());
        assert!(!crate::metadata_store_has_plaintext_header(&active).unwrap());
        validate_active_store(
            &active,
            &fixture.key,
            &read_manifest(&fixture.directory.path().join(MANIFEST_FILE))
                .unwrap()
                .store_id_digest,
        )
        .unwrap();
    }

    #[test]
    fn committed_pointer_failpoints_resume_cleanup_without_rolling_back_active_store() {
        for failpoint in [
            MigrationFailpoint::AfterManifestRename,
            MigrationFailpoint::AfterManifest,
            MigrationFailpoint::AfterLegacyMainDelete,
            MigrationFailpoint::AfterLegacySidecarDelete,
        ] {
            let fixture = LegacyFixture::create_plaintext(&format!("cleanup-{failpoint:?}"));
            let error = ensure_active_v27_store_with_failpoint(
                fixture.directory.path(),
                &fixture.key,
                failpoint,
            )
            .unwrap_err();
            assert_eq!(error.class(), crate::MetaStoreErrorClass::StorageInvariant);

            let committed_active = active_store_path(fixture.directory.path()).unwrap();
            assert_ne!(committed_active, fixture.path);
            assert!(committed_active.is_file());
            for suffix in ["-wal", "-shm"] {
                let sidecar = PathBuf::from(format!("{}{suffix}", fixture.path.display()));
                fs::write(&sidecar, b"synthetic legacy sidecar").unwrap();
                restrict_private_file_permissions(&sidecar).unwrap();
            }

            let reopened = ensure_active_v27_store(fixture.directory.path(), &fixture.key).unwrap();
            assert_eq!(reopened, committed_active);
            assert_eq!(
                active_store_path(fixture.directory.path()).unwrap(),
                committed_active
            );
            assert_legacy_artifacts_retired(&fixture);
            assert_no_retained_private_marker(fixture.directory.path());
        }
    }

    #[test]
    fn concurrent_openers_publish_exactly_one_v27_store() {
        use std::sync::{Arc, Barrier};

        let fixture = LegacyFixture::create("concurrent");
        let barrier = Arc::new(Barrier::new(8));
        let mut openers = Vec::new();
        for _ in 0..8 {
            let data_dir = fixture.directory.path().to_path_buf();
            let key = fixture.key;
            let barrier = Arc::clone(&barrier);
            openers.push(std::thread::spawn(move || {
                barrier.wait();
                ensure_active_v27_store(&data_dir, &key)
            }));
        }

        let active_paths = openers
            .into_iter()
            .map(|opener| opener.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert!(active_paths.windows(2).all(|pair| pair[0] == pair[1]));
        assert_eq!(
            fs::read_dir(fixture.directory.path())
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("metadata-v27-")
                })
                .count(),
            1
        );
        assert_eq!(
            active_store_path(fixture.directory.path()).unwrap(),
            active_paths[0]
        );
    }

    #[test]
    fn empty_data_dir_is_initialized_behind_a_validated_manifest() {
        let directory = TestDirectory::new("fresh");
        let mut key = [0_u8; 32];
        getrandom::getrandom(&mut key).unwrap();

        let active = ensure_active_v27_store(directory.path(), &key).unwrap();

        assert_ne!(active, directory.path().join(METADATA_STORE_FILE));
        assert!(directory.path().join(MANIFEST_FILE).is_file());
        let manifest = read_manifest(&directory.path().join(MANIFEST_FILE)).unwrap();
        validate_active_store(&active, &key, &manifest.store_id_digest).unwrap();
    }

    #[test]
    fn concurrent_data_dir_openers_share_one_key_and_one_active_store() {
        use std::sync::{Arc, Barrier};

        let directory = TestDirectory::new("concurrent-open-data-dir");
        let barrier = Arc::new(Barrier::new(8));
        let mut openers = Vec::new();
        for _ in 0..8 {
            let data_dir = directory.path().to_path_buf();
            let barrier = Arc::clone(&barrier);
            openers.push(std::thread::spawn(move || {
                barrier.wait();
                let store = MetaStore::open_data_dir(&data_dir)?;
                store.schema_version()
            }));
        }

        for opener in openers {
            assert_eq!(opener.join().unwrap().unwrap(), schema_v27::VERSION);
        }
        assert_eq!(
            fs::read_dir(directory.path())
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("metadata-v27-")
                })
                .count(),
            1
        );
        let active = active_store_path(directory.path()).unwrap();
        let key = super::super::read_metadata_encryption_key(
            &super::super::metadata_encryption_key_path(directory.path()),
        )
        .unwrap();
        let manifest = read_manifest(&directory.path().join(MANIFEST_FILE)).unwrap();
        validate_active_store(&active, &key, &manifest.store_id_digest).unwrap();
    }

    #[test]
    fn corrupt_or_missing_published_store_fails_closed() {
        let fixture = LegacyFixture::create("fail-closed");
        let active = ensure_active_v27_store(fixture.directory.path(), &fixture.key).unwrap();
        fs::remove_file(&active).unwrap();
        assert!(ensure_active_v27_store(fixture.directory.path(), &fixture.key).is_err());
        assert!(!active.exists());

        let corrupt = LegacyFixture::create("corrupt-manifest");
        ensure_active_v27_store(corrupt.directory.path(), &corrupt.key).unwrap();
        let manifest = corrupt.directory.path().join(MANIFEST_FILE);
        fs::write(&manifest, b"not-a-valid-manifest").unwrap();
        restrict_private_file_permissions(&manifest).unwrap();
        assert!(ensure_active_v27_store(corrupt.directory.path(), &corrupt.key).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn manifest_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let fixture = LegacyFixture::create("manifest-symlink");
        let target = fixture.directory.path().join("manifest-target");
        fs::write(&target, b"synthetic").unwrap();
        restrict_private_file_permissions(&target).unwrap();
        symlink(&target, fixture.directory.path().join(MANIFEST_FILE)).unwrap();
        assert!(active_store_path(fixture.directory.path()).is_err());
    }
}
