use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{Connection, TransactionBehavior};

use crate::data_directory_owner::DataDirectoryOwnerGuard;
#[cfg(test)]
use crate::DataDirectoryOwnerLease;
use crate::{
    active_store_manifest::{
        owner_regular_file_exists, publish_new_active_store, random_store_id_digest, read_manifest,
        replace_active_store, ActiveStoreManifest, MANIFEST_FILE,
    },
    apply_sqlcipher_key,
    migration_v27::{
        complete_pending_v27_legacy_cleanup, create_private_empty_file,
        discard_pending_v27_legacy_cleanup, legacy_owner_regular_file_exists,
        open_encrypted_connection, source_schema_version, store_identity, sync_validated_store,
        validate_active_store as validate_active_v27_store, with_migration_lock,
    },
    restrict_private_file_permissions, schema_v27, schema_v28, MetaStoreError,
    MetadataEncryptionState, OwnedMetaStore, Result, METADATA_ENCRYPTION_KEY_LEN,
    METADATA_STORE_FILE,
};

mod allowlist;
mod cleanup;
mod predecessor_fence;
mod validation;

use allowlist::copy_allowed_source_state;
use cleanup::{
    pending_migration_attempt, published_previous_store, recover_migration_attempt,
    write_migration_attempt, MigrationAttempt, PreviousStore,
};
use predecessor_fence::{install_predecessor_write_fence, read_predecessor_write_fence};
use validation::{validate_active_v28_store, validate_staging_v28_store};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationFailpoint {
    None,
    AfterAttemptWriteCrash,
    AfterTargetCreateCrash,
    AfterSourceCopyCrash,
    AfterTargetValidationCrash,
    BeforeManifest,
    AfterPredecessorFence,
    AfterManifestRename,
    AfterManifest,
}

struct UpgradeSource {
    connection: Connection,
    schema_version: u32,
    previous: Option<PreviousStore>,
    current_manifest: Option<ActiveStoreManifest>,
}

pub(super) fn validate_current_v28_store(
    path: &Path,
    key: &[u8],
    store_id_digest: &str,
) -> Result<()> {
    validate_active_v28_store(path, key, store_id_digest)
}

pub(super) fn prepare_active_v28_store(
    owner: &Arc<DataDirectoryOwnerGuard>,
) -> Result<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN])> {
    let data_dir = owner.canonical_data_dir();
    with_migration_lock(data_dir, || {
        let key = crate::load_or_create_metadata_encryption_key(data_dir)?;
        let path = ensure_active_v28_store_locked(owner, &key, MigrationFailpoint::None)?;
        Ok((path, key))
    })
}

#[cfg(test)]
fn ensure_active_v28_store(
    data_dir: &Path,
    key: &[u8],
    failpoint: MigrationFailpoint,
) -> Result<PathBuf> {
    let lease = match DataDirectoryOwnerLease::try_acquire(data_dir)
        .map_err(|_| MetaStoreError::storage_invariant())?
    {
        crate::DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        crate::DataDirectoryOwnerAcquisition::Contended => {
            return Err(MetaStoreError::migration_ownership_required());
        }
    };
    let owner = lease.shared_guard();
    with_migration_lock(owner.canonical_data_dir(), || {
        ensure_active_v28_store_locked(&owner, key, failpoint)
    })
}

fn ensure_active_v28_store_locked(
    owner: &Arc<DataDirectoryOwnerGuard>,
    key: &[u8],
    failpoint: MigrationFailpoint,
) -> Result<PathBuf> {
    let data_dir = owner.canonical_data_dir();
    let manifest_path = data_dir.join(MANIFEST_FILE);
    let current_manifest = owner_regular_file_exists(&manifest_path)?
        .then(|| read_manifest(&manifest_path))
        .transpose()?;
    if let Some(manifest) = current_manifest.as_ref() {
        let active_path = data_dir.join(&manifest.file_name);
        match manifest.schema_version {
            schema_v28::VERSION => {
                validate_active_v28_store(&active_path, key, &manifest.store_id_digest)?;
                recover_published_v28_attempt(owner, key, manifest)?;
                return Ok(active_path);
            }
            schema_v27::VERSION => {}
            _ => return Err(MetaStoreError::invalid_value("metadata.active_manifest")),
        }
    }

    if let Some(manifest) = current_manifest {
        let source_path = data_dir.join(&manifest.file_name);
        let source = open_encrypted_connection(&source_path, key)?;
        return upgrade_source(
            owner,
            key,
            UpgradeSource {
                connection: source,
                schema_version: manifest.schema_version,
                previous: Some(PreviousStore {
                    file_name: manifest.file_name.clone(),
                    schema_version: manifest.schema_version,
                    store_id_digest: Some(manifest.store_id_digest.clone()),
                }),
                current_manifest: Some(manifest),
            },
            failpoint,
        );
    }

    let legacy_path = data_dir.join(METADATA_STORE_FILE);
    if !legacy_owner_regular_file_exists(&legacy_path)? {
        let empty = Connection::open_in_memory().map_err(MetaStoreError::storage)?;
        return upgrade_source(
            owner,
            key,
            UpgradeSource {
                connection: empty,
                schema_version: 0,
                previous: None,
                current_manifest: None,
            },
            failpoint,
        );
    }
    let source_is_plaintext = crate::metadata_store_has_plaintext_header(&legacy_path)?;
    let source = if source_is_plaintext {
        Connection::open(&legacy_path).map_err(MetaStoreError::storage)?
    } else {
        open_encrypted_connection(&legacy_path, key)?
    };
    let observed_version = source_schema_version(&source)?;
    let version = read_predecessor_write_fence(&source)?
        .map_or(observed_version, |fence| fence.source_schema_version);
    if version > schema_v28::VERSION {
        return Err(MetaStoreError::invalid_value("metadata.schema_version"));
    }
    if version == schema_v28::VERSION {
        return Err(MetaStoreError::migration_ownership_required());
    }
    if !matches!(version, 0 | 26 | schema_v27::VERSION) {
        return Err(MetaStoreError::invalid_value("metadata.schema_version"));
    }
    let source_digest = (version == schema_v27::VERSION)
        .then(|| store_identity(&source))
        .transpose()?;
    upgrade_source(
        owner,
        key,
        UpgradeSource {
            connection: source,
            schema_version: version,
            previous: Some(PreviousStore {
                file_name: METADATA_STORE_FILE.to_string(),
                schema_version: version,
                store_id_digest: source_digest,
            }),
            current_manifest: None,
        },
        failpoint,
    )
}

fn upgrade_source(
    owner: &Arc<DataDirectoryOwnerGuard>,
    key: &[u8],
    source: UpgradeSource,
    failpoint: MigrationFailpoint,
) -> Result<PathBuf> {
    if read_predecessor_write_fence(&source.connection)?.is_some() {
        return recover_fenced_prepublication(owner, key, source, failpoint);
    }
    if let Some(manifest) = source.current_manifest.as_ref() {
        if source.schema_version != schema_v27::VERSION
            || manifest.schema_version != source.schema_version
        {
            return Err(MetaStoreError::storage_invariant());
        }
        validate_active_v27_store(
            &owner.canonical_data_dir().join(&manifest.file_name),
            key,
            &manifest.store_id_digest,
        )?;
    }
    rebuild_from_source(owner, key, source, failpoint)
}

fn recover_fenced_prepublication(
    owner: &Arc<DataDirectoryOwnerGuard>,
    key: &[u8],
    source: UpgradeSource,
    failpoint: MigrationFailpoint,
) -> Result<PathBuf> {
    let data_dir = owner.canonical_data_dir();
    let _publication_guard = owner.try_acquire_search_publication_ownership()?;
    let UpgradeSource {
        mut connection,
        schema_version,
        previous,
        current_manifest,
    } = source;
    let source_snapshot = connection
        .transaction_with_behavior(TransactionBehavior::Exclusive)
        .map_err(MetaStoreError::storage)?;
    let fence = read_predecessor_write_fence(&source_snapshot)?
        .ok_or_else(MetaStoreError::storage_invariant)?;
    if fence.source_schema_version != schema_version {
        return Err(MetaStoreError::storage_invariant());
    }
    let task_ids = legacy_import_task_ids(&source_snapshot, schema_version)?;
    let legacy_task_locks =
        crate::data_directory_owner::acquire_legacy_task_locks(data_dir, task_ids)?;
    let attempt =
        pending_migration_attempt(data_dir)?.ok_or_else(MetaStoreError::storage_invariant)?;
    if attempt.expected_manifest != current_manifest
        || attempt.previous != previous
        || attempt.target != fence.target
    {
        return Err(MetaStoreError::storage_invariant());
    }
    let target_path = data_dir.join(&fence.target.file_name);
    validate_staging_v28_store(
        &target_path,
        key,
        &fence.target.store_id_digest,
        &fence.inventory,
    )?;
    sync_validated_store(&target_path)?;
    source_snapshot.commit().map_err(MetaStoreError::storage)?;
    drop(connection);

    if failpoint == MigrationFailpoint::AfterPredecessorFence {
        return Err(MetaStoreError::storage_invariant());
    }
    publish_manifest(
        data_dir,
        current_manifest.as_ref(),
        &fence.target,
        failpoint,
    )?;
    if failpoint == MigrationFailpoint::AfterManifest {
        return Err(MetaStoreError::storage_invariant());
    }
    if let Some(predecessor) = current_manifest.as_ref().filter(|predecessor| {
        predecessor.schema_version == schema_v27::VERSION
            && predecessor.file_name != METADATA_STORE_FILE
    }) {
        complete_pending_v27_legacy_cleanup(data_dir, predecessor)?;
    }
    recover_migration_attempt(data_dir, Some(&fence.target))?;
    drop(legacy_task_locks);
    Ok(target_path)
}

fn recover_published_v28_attempt(
    owner: &Arc<DataDirectoryOwnerGuard>,
    key: &[u8],
    manifest: &ActiveStoreManifest,
) -> Result<()> {
    let data_dir = owner.canonical_data_dir();
    let Some(previous) = published_previous_store(data_dir, manifest)? else {
        recover_migration_attempt(data_dir, Some(manifest))?;
        return Ok(());
    };
    let _publication_guard = owner.try_acquire_search_publication_ownership()?;
    let previous_path = data_dir.join(&previous.file_name);
    let legacy_task_locks = if owner_regular_file_exists(&previous_path)? {
        let plaintext = crate::metadata_store_has_plaintext_header(&previous_path)?;
        let mut source = if plaintext {
            Connection::open(&previous_path).map_err(MetaStoreError::storage)?
        } else {
            open_encrypted_connection(&previous_path, key)?
        };
        let snapshot = source
            .transaction_with_behavior(TransactionBehavior::Exclusive)
            .map_err(MetaStoreError::storage)?;
        let fence = read_predecessor_write_fence(&snapshot)?
            .ok_or_else(MetaStoreError::storage_invariant)?;
        if fence.source_schema_version != previous.schema_version || fence.target != *manifest {
            return Err(MetaStoreError::storage_invariant());
        }
        if previous.schema_version == schema_v27::VERSION
            && previous.store_id_digest.as_deref() != Some(store_identity(&snapshot)?.as_str())
        {
            return Err(MetaStoreError::storage_invariant());
        }
        validate_staging_v28_store(
            &data_dir.join(&manifest.file_name),
            key,
            &manifest.store_id_digest,
            &fence.inventory,
        )?;
        let task_ids = legacy_import_task_ids(&snapshot, fence.source_schema_version)?;
        let locks = crate::data_directory_owner::acquire_legacy_task_locks(data_dir, task_ids)?;
        snapshot.commit().map_err(MetaStoreError::storage)?;
        drop(source);
        locks
    } else {
        crate::data_directory_owner::acquire_legacy_task_locks(data_dir, Vec::new())?
    };

    if previous.schema_version == schema_v27::VERSION && previous.file_name != METADATA_STORE_FILE {
        let predecessor_manifest = ActiveStoreManifest {
            file_name: previous.file_name.clone(),
            schema_version: previous.schema_version,
            store_id_digest: previous
                .store_id_digest
                .clone()
                .ok_or_else(MetaStoreError::storage_invariant)?,
        };
        complete_pending_v27_legacy_cleanup(data_dir, &predecessor_manifest)?;
    }
    recover_migration_attempt(data_dir, Some(manifest))?;
    drop(legacy_task_locks);
    Ok(())
}

fn rebuild_from_source(
    owner: &Arc<DataDirectoryOwnerGuard>,
    key: &[u8],
    source: UpgradeSource,
    failpoint: MigrationFailpoint,
) -> Result<PathBuf> {
    let data_dir = owner.canonical_data_dir();
    let _publication_guard = owner.try_acquire_search_publication_ownership()?;
    let UpgradeSource {
        mut connection,
        schema_version,
        previous,
        current_manifest,
    } = source;
    if schema_version >= schema_v28::VERSION {
        return Err(MetaStoreError::storage_invariant());
    }
    let source_snapshot = connection
        .transaction_with_behavior(TransactionBehavior::Exclusive)
        .map_err(MetaStoreError::storage)?;
    let task_ids = legacy_import_task_ids(&source_snapshot, schema_version)?;
    let legacy_task_locks =
        crate::data_directory_owner::acquire_legacy_task_locks(data_dir, task_ids)?;
    if current_manifest.is_none() {
        discard_pending_v27_legacy_cleanup(data_dir)?;
    }
    recover_migration_attempt(data_dir, current_manifest.as_ref())?;

    let store_id_digest = random_store_id_digest()?;
    let target_file_name = format!("metadata-v28-{}.sqlite3", &store_id_digest[..16]);
    let target_path = data_dir.join(&target_file_name);
    if owner_regular_file_exists(&target_path)? {
        return Err(MetaStoreError::storage_invariant());
    }
    let desired = ActiveStoreManifest {
        file_name: target_file_name,
        schema_version: schema_v28::VERSION,
        store_id_digest,
    };
    write_migration_attempt(
        data_dir,
        &MigrationAttempt {
            expected_manifest: current_manifest.clone(),
            previous: previous.clone(),
            target: desired.clone(),
        },
    )?;
    if failpoint == MigrationFailpoint::AfterAttemptWriteCrash {
        return Err(MetaStoreError::storage_invariant());
    }

    if let Err(error) = create_empty_v28_target(owner, &target_path, key, &desired.store_id_digest)
    {
        recover_migration_attempt(data_dir, current_manifest.as_ref())?;
        return Err(error);
    }
    if failpoint == MigrationFailpoint::AfterTargetCreateCrash {
        return Err(MetaStoreError::storage_invariant());
    }

    let inventory =
        match copy_allowed_source_state(&source_snapshot, schema_version, &target_path, key) {
            Ok(inventory) => inventory,
            Err(error) => {
                drop(source_snapshot);
                drop(legacy_task_locks);
                recover_migration_attempt(data_dir, current_manifest.as_ref())?;
                return Err(error);
            }
        };
    if failpoint == MigrationFailpoint::AfterSourceCopyCrash {
        return Err(MetaStoreError::storage_invariant());
    }

    let validation =
        validate_staging_v28_store(&target_path, key, &desired.store_id_digest, &inventory)
            .and_then(|()| sync_validated_store(&target_path));
    if let Err(error) = validation {
        recover_migration_attempt(data_dir, current_manifest.as_ref())?;
        return Err(error);
    }
    if failpoint == MigrationFailpoint::AfterTargetValidationCrash {
        return Err(MetaStoreError::storage_invariant());
    }
    if failpoint == MigrationFailpoint::BeforeManifest {
        drop(source_snapshot);
        drop(legacy_task_locks);
        recover_migration_attempt(data_dir, current_manifest.as_ref())?;
        return Err(MetaStoreError::storage_invariant());
    }
    if previous.is_some() {
        install_predecessor_write_fence(&source_snapshot, schema_version, &desired, &inventory)?;
    }
    source_snapshot.commit().map_err(MetaStoreError::storage)?;
    drop(connection);
    if failpoint == MigrationFailpoint::AfterPredecessorFence {
        return Err(MetaStoreError::storage_invariant());
    }
    publish_manifest(data_dir, current_manifest.as_ref(), &desired, failpoint)?;
    if failpoint == MigrationFailpoint::AfterManifest {
        return Err(MetaStoreError::storage_invariant());
    }
    if let Some(predecessor) = current_manifest.as_ref().filter(|predecessor| {
        predecessor.schema_version == schema_v27::VERSION
            && predecessor.file_name != METADATA_STORE_FILE
    }) {
        complete_pending_v27_legacy_cleanup(data_dir, predecessor)?;
    }
    recover_migration_attempt(data_dir, Some(&desired))?;
    drop(legacy_task_locks);
    Ok(target_path)
}

fn legacy_import_task_ids(source: &Connection, source_version: u32) -> Result<Vec<String>> {
    if source_version == 0 {
        return Ok(Vec::new());
    }
    let mut statement = source
        .prepare("SELECT id FROM import_task ORDER BY id")
        .map_err(MetaStoreError::storage)?;
    let mut rows = statement.query([]).map_err(MetaStoreError::storage)?;
    let mut task_ids = Vec::new();
    while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
        task_ids.push(row.get::<_, String>(0).map_err(MetaStoreError::storage)?);
    }
    Ok(task_ids)
}

fn create_empty_v28_target(
    owner: &Arc<DataDirectoryOwnerGuard>,
    target_path: &Path,
    key: &[u8],
    store_id_digest: &str,
) -> Result<()> {
    create_private_empty_file(target_path)?;
    let connection = Connection::open(target_path).map_err(MetaStoreError::storage)?;
    apply_sqlcipher_key(&connection, key)?;
    let store = OwnedMetaStore::from_owned_connection(
        connection,
        MetadataEncryptionState::SqlCipher,
        Arc::clone(owner),
    )?;
    store.migrate_staging_store_to_v28(store_id_digest)?;
    drop(store);
    restrict_private_file_permissions(target_path)
}

fn publish_manifest(
    data_dir: &Path,
    current: Option<&ActiveStoreManifest>,
    desired: &ActiveStoreManifest,
    failpoint: MigrationFailpoint,
) -> Result<()> {
    let after_rename = || {
        if failpoint == MigrationFailpoint::AfterManifestRename {
            Err(MetaStoreError::storage_invariant())
        } else {
            Ok(())
        }
    };
    if let Some(current) = current {
        replace_active_store(data_dir, current, desired, after_rename)
    } else {
        publish_new_active_store(data_dir, desired, after_rename)
    }
}

#[cfg(test)]
#[path = "migration_v28_tests.rs"]
mod tests;
