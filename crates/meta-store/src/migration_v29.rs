use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use rusqlite::{Connection, Transaction};

use crate::data_directory_owner::DataDirectoryOwnerGuard;
use crate::{
    active_store_manifest::{
        owner_regular_file_exists, read_manifest, remove_owner_file_if_exists,
        replace_active_store, sync_parent_directory, ActiveStoreManifest, MANIFEST_FILE,
    },
    migration_v27::{
        open_encrypted_connection, open_encrypted_read_connection, source_schema_version,
        store_identity, sync_validated_store, with_migration_lock,
    },
    migration_v28, restrict_private_file_permissions, schema_v28, schema_v29,
    schema_v29_publication_retirement, MetaStoreError, Result, METADATA_ENCRYPTION_KEY_LEN,
};

#[path = "migration_v29_descriptor_validation.rs"]
mod descriptor_validation;
#[cfg(feature = "migration-test-support")]
#[path = "migration_v29_fixture_support.rs"]
pub(crate) mod fixture_support;
#[path = "migration_v29_projection_validation.rs"]
mod projection_validation;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationFailpoint {
    None,
    AfterTargetValidation,
    AfterManifest,
}

pub(super) fn active_store_path(data_dir: &Path) -> Result<PathBuf> {
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if !owner_regular_file_exists(&manifest_path)? {
        return Err(MetaStoreError::migration_ownership_required());
    }
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
        return Ok(None);
    }
    let manifest = read_manifest(&manifest_path)?;
    if manifest.schema_version != schema_v29::VERSION {
        return Err(MetaStoreError::migration_ownership_required());
    }
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
    if let Some(current) = with_migration_lock(data_dir, || {
        let manifest_path = data_dir.join(MANIFEST_FILE);
        if !owner_regular_file_exists(&manifest_path)? {
            return Ok(None);
        }
        let manifest = read_manifest(&manifest_path)?;
        if manifest.schema_version != schema_v29::VERSION {
            return Ok(None);
        }
        let key = crate::load_or_create_metadata_encryption_key(data_dir)?;
        let path = data_dir.join(&manifest.file_name);
        validate_active_v29_store(&path, &key, &manifest.store_id_digest)?;
        cleanup_v28_predecessor(data_dir, &manifest)?;
        Ok(Some((path, key)))
    })? {
        return Ok(current);
    }

    let (_, key) = migration_v28::prepare_active_v28_store(owner)?;
    with_migration_lock(data_dir, || {
        ensure_active_v29_store_locked(owner, &key, MigrationFailpoint::None)
            .map(|path| (path, key))
    })
}

fn ensure_active_v29_store_locked(
    owner: &Arc<DataDirectoryOwnerGuard>,
    key: &[u8],
    failpoint: MigrationFailpoint,
) -> Result<PathBuf> {
    let data_dir = owner.canonical_data_dir();
    let _publication_guard = owner.try_acquire_search_publication_ownership()?;
    let manifest = read_manifest(&data_dir.join(MANIFEST_FILE))?;
    if manifest.schema_version == schema_v29::VERSION {
        let path = data_dir.join(&manifest.file_name);
        validate_active_v29_store(&path, key, &manifest.store_id_digest)?;
        cleanup_v28_predecessor(data_dir, &manifest)?;
        return Ok(path);
    }
    if manifest.schema_version != schema_v28::VERSION {
        return Err(MetaStoreError::invalid_value("metadata.active_manifest"));
    }

    let source_path = data_dir.join(&manifest.file_name);
    migration_v28::validate_current_v28_store(&source_path, key, &manifest.store_id_digest)?;
    let target = ActiveStoreManifest {
        file_name: format!("metadata-v29-{}.sqlite3", &manifest.store_id_digest[..16]),
        schema_version: schema_v29::VERSION,
        store_id_digest: manifest.store_id_digest.clone(),
    };
    let target_path = data_dir.join(&target.file_name);
    // The v28 manifest is the only committed authority at this point. A v29
    // file left behind before the manifest swap is merely staging, even when
    // it validates in isolation: the active v28 store may have advanced after
    // the crash. Reusing that orphan would promote a stale snapshot and then
    // retire the newer source.
    if owner_regular_file_exists(&target_path)? {
        remove_inactive_store_artifacts(data_dir, &target_path)?;
    }
    fs::copy(&source_path, &target_path).map_err(MetaStoreError::io_storage)?;
    restrict_private_file_permissions(&target_path)?;
    let mut connection = open_encrypted_connection(&target_path, key)?;
    crate::apply_v29_target_schema(&mut connection)?;
    drop(connection);
    sync_validated_store(&target_path)?;
    validate_active_v29_store(&target_path, key, &target.store_id_digest)?;
    if failpoint == MigrationFailpoint::AfterTargetValidation {
        return Err(MetaStoreError::storage_invariant());
    }
    replace_active_store(data_dir, &manifest, &target, || Ok(()))?;
    if failpoint == MigrationFailpoint::AfterManifest {
        return Err(MetaStoreError::storage_invariant());
    }
    cleanup_v28_predecessor(data_dir, &target)?;
    Ok(target_path)
}

fn cleanup_v28_predecessor(data_dir: &Path, manifest: &ActiveStoreManifest) -> Result<()> {
    if manifest.schema_version != schema_v29::VERSION {
        return Err(MetaStoreError::storage_invariant());
    }
    let predecessor = data_dir.join(format!(
        "metadata-v28-{}.sqlite3",
        &manifest.store_id_digest[..16]
    ));
    if predecessor != data_dir.join(&manifest.file_name) {
        remove_inactive_store_artifacts(data_dir, &predecessor)?;
    }
    Ok(())
}

fn remove_inactive_store_artifacts(data_dir: &Path, store_path: &Path) -> Result<()> {
    for suffix in [None, Some("-journal"), Some("-wal"), Some("-shm")] {
        let path = if let Some(suffix) = suffix {
            let mut value = store_path.as_os_str().to_owned();
            value.push(suffix);
            PathBuf::from(value)
        } else {
            store_path.to_path_buf()
        };
        remove_owner_file_if_exists(&path)?;
    }
    sync_parent_directory(data_dir)
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
    descriptor_validation::validate_persisted_repair_context(connection)?;
    schema_v29_publication_retirement::validate(connection)
}

pub(super) fn apply_v29_contract_migration(transaction: &Transaction<'_>) -> Result<()> {
    descriptor_validation::apply_contract_migration(transaction)
}

#[cfg(test)]
use descriptor_validation::descriptor_contract;

#[cfg(test)]
#[path = "migration_v29_tests.rs"]
mod tests;
