//! Exact current-v29 active-store boundary.
//!
//! Production opens validate an already-published v29 store without repair, or
//! initialize v29 from an authority-free directory. Older schemas and partial
//! authorities are rejected without entering the test-only migration code.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use rusqlite::{Connection, Transaction};

use crate::data_directory_owner::DataDirectoryOwnerGuard;
use crate::{
    active_store_manifest::{
        owner_regular_file_exists, publish_new_active_store, random_store_id_digest, read_manifest,
        read_manifest_schema_version, sync_parent_directory, validate_owner_directory_metadata,
        validate_owner_regular_metadata, ActiveStoreManifest, MANIFEST_FILE,
    },
    migration_v27::{
        open_encrypted_read_connection, source_schema_version, store_identity, sync_validated_store,
    },
    schema_v29, schema_v29_publication_retirement, MetaStoreError, MetadataEncryptionState,
    OwnedMetaStore, Result, METADATA_ENCRYPTION_KEY_LEN, METADATA_STORE_FILE,
};

#[path = "migration_v29_descriptor_validation.rs"]
mod descriptor_validation;
#[cfg(feature = "migration-test-support")]
#[path = "migration_v29_fixture_support.rs"]
pub(crate) mod fixture_support;
#[path = "migration_v29_projection_validation.rs"]
mod projection_validation;

const LEGACY_CLEANUP_RECEIPT_FILE: &str = "metadata-legacy-cleanup.v1";
const V28_MIGRATION_ATTEMPT_FILE: &str = "metadata-v28-migration-attempt.v1";
const V28_MIGRATION_ATTEMPT_TEMP_PREFIX: &str = ".metadata-v28-migration-attempt.v1.tmp-";
const MANIFEST_TEMP_PREFIX: &str = ".metadata-active.v1.tmp-";
const FRESH_V29_TEMP_PREFIX: &str = ".metadata-v29-init-";
const SQLITE_AUTHORITY_SUFFIXES: [&str; 4] = [
    ".sqlite3",
    ".sqlite3-journal",
    ".sqlite3-wal",
    ".sqlite3-shm",
];

pub(super) fn active_store_path(data_dir: &Path) -> Result<PathBuf> {
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if !owner_regular_file_exists(&manifest_path)? {
        return Err(MetaStoreError::migration_ownership_required());
    }
    require_current_manifest_version(&manifest_path)?;
    Ok(data_dir.join(read_manifest(&manifest_path)?.file_name))
}

pub(super) fn open_current_v29_store(
    data_dir: &Path,
) -> Result<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN], String)> {
    open_optional_current_v29_store(data_dir)?
        .ok_or_else(MetaStoreError::migration_ownership_required)
}

pub(super) fn open_optional_current_v29_store(
    data_dir: &Path,
) -> Result<Option<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN], String)>> {
    match fs::symlink_metadata(data_dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(MetaStoreError::io_storage(error)),
    }
    let data_dir = fs::canonicalize(data_dir).map_err(MetaStoreError::io_storage)?;
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if !owner_regular_file_exists(&manifest_path)? {
        reject_legacy_or_partial_authority(&data_dir)?;
        return Ok(None);
    }
    require_current_manifest_version(&manifest_path)?;
    let manifest = read_manifest(&manifest_path)?;
    let key = crate::read_metadata_encryption_key_without_repair(
        &crate::metadata_encryption_key_path(&data_dir),
    )?;
    Ok(Some((
        data_dir.join(&manifest.file_name),
        key,
        manifest.store_id_digest,
    )))
}

pub(super) fn validate_current_v29_connection(
    connection: &Connection,
    store_id_digest: &str,
) -> Result<()> {
    validate_active_v29_connection(connection, store_id_digest)
}

pub(super) fn prepare_active_v29_store(
    owner: &Arc<DataDirectoryOwnerGuard>,
) -> Result<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN])> {
    let data_dir = owner.canonical_data_dir();
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if owner_regular_file_exists(&manifest_path)? {
        require_current_manifest_version(&manifest_path)?;
        let manifest = read_manifest(&manifest_path)?;
        let key = crate::read_metadata_encryption_key_without_repair(
            &crate::metadata_encryption_key_path(data_dir),
        )?;
        let path = data_dir.join(&manifest.file_name);
        validate_active_v29_store(&path, &key, &manifest.store_id_digest)?;
        return Ok((path, key));
    }

    reject_legacy_or_partial_authority(data_dir)?;
    create_fresh_v29_store(owner)
}

fn create_fresh_v29_store(
    owner: &Arc<DataDirectoryOwnerGuard>,
) -> Result<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN])> {
    let data_dir = owner.canonical_data_dir();
    let key = crate::random_metadata_encryption_key()?;
    let staging_token = random_store_id_digest()?;
    let staging_path = data_dir.join(format!(
        "{FRESH_V29_TEMP_PREFIX}{}.sqlite3",
        &staging_token[..16]
    ));
    let staging = create_owned_private_empty_file(&staging_path)?;
    let mut publication = FreshV29Publication::new(data_dir, staging);

    let connection = rusqlite::Connection::open(&staging_path).map_err(MetaStoreError::storage)?;
    crate::apply_sqlcipher_key(&connection, &key)?;
    crate::verify_sqlcipher_key(&connection)?;
    let store = OwnedMetaStore::from_owned_connection(
        connection,
        MetadataEncryptionState::SqlCipher,
        Arc::clone(owner),
    )?;
    let store_id_digest = initialize_current_v29_from_empty(&store)?;
    drop(store);
    sync_validated_store(&staging_path)?;
    validate_active_v29_store(&staging_path, &key, &store_id_digest)?;

    let target = ActiveStoreManifest {
        file_name: format!("metadata-v29-{}.sqlite3", &store_id_digest[..16]),
        schema_version: schema_v29::VERSION,
        store_id_digest,
    };
    let target_path = data_dir.join(&target.file_name);
    let target_guard = link_owned_regular_file(&publication.staging, &target_path)?;
    publication.target = Some(target_guard);
    publication.staging.delete_owned()?;
    sync_parent_directory(data_dir)
        .and_then(|()| publish_fresh_key(data_dir, &key, &mut publication))?;

    let publish_result = publish_new_active_store(data_dir, &target, || Ok(()));
    let manifest_is_current =
        read_manifest(&data_dir.join(MANIFEST_FILE)).is_ok_and(|published| published == target);
    if publish_result.is_ok() || manifest_is_current {
        publication.commit();
    }
    publish_result?;
    Ok((target_path, key))
}

fn initialize_current_v29_from_empty(store: &OwnedMetaStore) -> Result<String> {
    let report = store.initialize_current_v29_schema()?;
    if report
        .applied_versions()
        .iter()
        .copied()
        .ne(1..=schema_v29::VERSION)
    {
        return Err(MetaStoreError::storage_invariant());
    }
    let connection = store.connection.borrow();
    if crate::schema_version_in_connection(&connection)? != schema_v29::VERSION {
        return Err(MetaStoreError::storage_invariant());
    }
    store_identity(&connection)
}

fn publish_fresh_key(
    data_dir: &Path,
    key: &[u8; METADATA_ENCRYPTION_KEY_LEN],
    publication: &mut FreshV29Publication,
) -> Result<()> {
    let key_path = crate::metadata_encryption_key_path(data_dir);
    let key_directory = key_path
        .parent()
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.encryption_key_path"))?;
    publication.key_directory = Some(create_owned_private_directory(key_directory)?);
    publication.key = Some(create_owned_private_file(
        &key_path,
        crate::encode_hex(key).as_bytes(),
    )?);
    sync_parent_directory(key_directory)
}

fn require_current_manifest_version(manifest_path: &Path) -> Result<()> {
    if read_manifest_schema_version(manifest_path)? != schema_v29::VERSION {
        return Err(MetaStoreError::unsupported_store_schema());
    }
    Ok(())
}

fn reject_legacy_or_partial_authority(data_dir: &Path) -> Result<()> {
    for entry in fs::read_dir(data_dir).map_err(MetaStoreError::io_storage)? {
        let entry = entry.map_err(MetaStoreError::io_storage)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_reserved_store_authority(&name) {
            return Err(MetaStoreError::unsupported_store_schema());
        }
    }
    Ok(())
}

fn is_reserved_store_authority(name: &str) -> bool {
    name == METADATA_STORE_FILE
        || name.starts_with("metadata.sqlite3-")
        || name == "metadata-secrets"
        || name == crate::migration_v27::MIGRATION_LOCK_FILE
        || name == LEGACY_CLEANUP_RECEIPT_FILE
        || name.starts_with("metadata-legacy-cleanup.")
        || name == V28_MIGRATION_ATTEMPT_FILE
        || name.starts_with("metadata-v28-migration-attempt.")
        || name.starts_with(V28_MIGRATION_ATTEMPT_TEMP_PREFIX)
        || name.starts_with(".metadata-v28-migration-attempt.")
        || name.starts_with("metadata-active.")
        || name.starts_with(".metadata-active.")
        || name.starts_with(MANIFEST_TEMP_PREFIX)
        || name.starts_with(FRESH_V29_TEMP_PREFIX)
        || ((name.starts_with("metadata-v") || name.starts_with(".metadata-v"))
            && SQLITE_AUTHORITY_SUFFIXES
                .iter()
                .any(|suffix| name.ends_with(suffix)))
}

#[cfg(unix)]
fn restrict_private_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(MetaStoreError::io_storage)
}

#[cfg(not(unix))]
fn restrict_private_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

struct FreshV29Publication {
    data_dir: PathBuf,
    key: Option<CreatedPath>,
    target: Option<CreatedPath>,
    staging: CreatedPath,
    key_directory: Option<CreatedPath>,
    committed: bool,
}

impl FreshV29Publication {
    fn new(data_dir: &Path, staging: CreatedPath) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            key: None,
            target: None,
            staging,
            key_directory: None,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
        self.key.iter_mut().for_each(CreatedPath::disarm);
        self.target.iter_mut().for_each(CreatedPath::disarm);
        self.staging.disarm();
        self.key_directory.iter_mut().for_each(CreatedPath::disarm);
    }
}

impl Drop for FreshV29Publication {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.key
            .iter_mut()
            .for_each(CreatedPath::delete_best_effort);
        self.target
            .iter_mut()
            .for_each(CreatedPath::delete_best_effort);
        self.staging.delete_best_effort();
        self.key_directory
            .iter_mut()
            .for_each(CreatedPath::delete_best_effort);
        let _ = sync_parent_directory(&self.data_dir);
    }
}

#[derive(Clone, Copy)]
enum CreatedPathKind {
    RegularFile,
    Directory,
}

struct CreatedPath {
    path: PathBuf,
    identity: Option<same_file::Handle>,
    kind: CreatedPathKind,
}

impl CreatedPath {
    fn capture_open_regular_file(path: &Path, file: &File) -> Result<Self> {
        let opened = file.metadata().map_err(MetaStoreError::io_storage)?;
        validate_owner_regular_metadata(&opened)?;
        let identity =
            same_file::Handle::from_file(file.try_clone().map_err(MetaStoreError::io_storage)?)
                .map_err(MetaStoreError::io_storage)?;
        validate_current_path(path, CreatedPathKind::RegularFile)?;
        let current = same_file::Handle::from_path(path).map_err(MetaStoreError::io_storage)?;
        if identity != current {
            return Err(MetaStoreError::storage_invariant());
        }
        Ok(Self {
            path: path.to_path_buf(),
            identity: Some(identity),
            kind: CreatedPathKind::RegularFile,
        })
    }

    fn capture_regular_file(path: &Path) -> Result<Self> {
        validate_current_path(path, CreatedPathKind::RegularFile)?;
        let file = OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(MetaStoreError::io_storage)?;
        Self::capture_open_regular_file(path, &file)
    }

    fn capture_directory(path: &Path) -> Result<Self> {
        validate_current_path(path, CreatedPathKind::Directory)?;
        let identity = same_file::Handle::from_path(path).map_err(MetaStoreError::io_storage)?;
        validate_current_path(path, CreatedPathKind::Directory)?;
        Ok(Self {
            path: path.to_path_buf(),
            identity: Some(identity),
            kind: CreatedPathKind::Directory,
        })
    }

    fn same_identity(&self, other: &Self) -> bool {
        self.identity
            .as_ref()
            .zip(other.identity.as_ref())
            .is_some_and(|(left, right)| left == right)
    }

    fn disarm(&mut self) {
        self.identity = None;
    }

    fn delete_owned(&mut self) -> Result<()> {
        let Some(expected) = self.identity.as_ref() else {
            return Ok(());
        };
        match fs::symlink_metadata(&self.path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.disarm();
                return Ok(());
            }
            Err(error) => return Err(MetaStoreError::io_storage(error)),
        }
        validate_current_path(&self.path, self.kind)?;
        let current =
            same_file::Handle::from_path(&self.path).map_err(MetaStoreError::io_storage)?;
        validate_current_path(&self.path, self.kind)?;
        if expected != &current {
            return Err(MetaStoreError::storage_invariant());
        }
        drop(current);
        match self.kind {
            CreatedPathKind::RegularFile => {
                fs::remove_file(&self.path).map_err(MetaStoreError::io_storage)?
            }
            CreatedPathKind::Directory => {
                fs::remove_dir(&self.path).map_err(MetaStoreError::io_storage)?
            }
        }
        self.disarm();
        if let Some(parent) = self.path.parent() {
            sync_parent_directory(parent)?;
        }
        Ok(())
    }

    fn delete_best_effort(&mut self) {
        let _ = self.delete_owned();
    }
}

impl Drop for CreatedPath {
    fn drop(&mut self) {
        self.delete_best_effort();
    }
}

fn validate_current_path(path: &Path, kind: CreatedPathKind) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
    match kind {
        CreatedPathKind::RegularFile => validate_owner_regular_metadata(&metadata),
        CreatedPathKind::Directory => validate_owner_directory_metadata(&metadata),
    }
}

fn create_owned_private_empty_file(path: &Path) -> Result<CreatedPath> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(MetaStoreError::io_storage)?;
    let owned = CreatedPath::capture_open_regular_file(path, &file)?;
    file.sync_all().map_err(MetaStoreError::io_storage)?;
    crate::restrict_private_file_permissions(path)?;
    validate_current_path(path, CreatedPathKind::RegularFile)?;
    Ok(owned)
}

fn create_owned_private_file(path: &Path, bytes: &[u8]) -> Result<CreatedPath> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(MetaStoreError::io_storage)?;
    let owned = CreatedPath::capture_open_regular_file(path, &file)?;
    file.write_all(bytes)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(MetaStoreError::io_storage)?;
    crate::restrict_private_file_permissions(path)?;
    validate_current_path(path, CreatedPathKind::RegularFile)?;
    Ok(owned)
}

fn create_owned_private_directory(path: &Path) -> Result<CreatedPath> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).map_err(MetaStoreError::io_storage)?;
    let owned = CreatedPath::capture_directory(path)?;
    restrict_private_directory_permissions(path)?;
    validate_current_path(path, CreatedPathKind::Directory)?;
    Ok(owned)
}

fn link_owned_regular_file(staging: &CreatedPath, target: &Path) -> Result<CreatedPath> {
    fs::hard_link(&staging.path, target).map_err(MetaStoreError::io_storage)?;
    let target = CreatedPath::capture_regular_file(target)?;
    if !staging.same_identity(&target) {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(target)
}

fn validate_active_v29_store(path: &Path, key: &[u8], store_id_digest: &str) -> Result<()> {
    if !owner_regular_file_exists(path)? {
        return Err(MetaStoreError::storage_invariant());
    }
    let connection = open_encrypted_read_connection(path, key)?;
    validate_active_v29_connection(&connection, store_id_digest)
}

fn validate_active_v29_connection(connection: &Connection, store_id_digest: &str) -> Result<()> {
    let journal_mode = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
        .map_err(MetaStoreError::storage)?;
    let integrity = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(MetaStoreError::storage)?;
    let foreign_key_failures = connection
        .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(MetaStoreError::storage)?;
    let migration_count = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(MetaStoreError::storage)?;
    let trigger_count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'trigger' AND name IN (
                 'search_publication_payload_immutable_after_validation',
                 'search_publication_same_state_immutable',
                 'search_publication_transition',
                 'ready_search_publication_immutable_update',
                 'artifact_repair_context_insert_authority',
                 'artifact_repair_context_immutable_update',
                 'artifact_repair_attempt_insert_authority',
                 'artifact_repair_context_head_change_cleanup'
             )",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if !journal_mode.eq_ignore_ascii_case("delete")
        || integrity != "ok"
        || foreign_key_failures != 0
        || source_schema_version(connection)? != schema_v29::VERSION
        || migration_count != i64::from(schema_v29::VERSION)
        || store_identity(connection)? != store_id_digest
        || trigger_count != 8
    {
        return Err(MetaStoreError::storage_invariant());
    }
    descriptor_validation::validate_current_repair_authority_trigger(connection)?;
    descriptor_validation::validate_current_publication_authority(connection)?;
    descriptor_validation::validate_persisted_repair_context(connection)?;
    schema_v29_publication_retirement::validate(connection)
}

pub(super) fn apply_v29_contract_migration(transaction: &Transaction<'_>) -> Result<()> {
    descriptor_validation::apply_contract_migration(transaction)
}

#[cfg(test)]
#[path = "migration_v29_tests.rs"]
mod tests;
