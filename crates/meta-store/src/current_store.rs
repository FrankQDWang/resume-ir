//! Current store publication and contiguous encrypted COW forward migration.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::{
    active_store_manifest::{
        owner_regular_file_exists, publish_new_active_store, random_store_id_digest, read_manifest,
        read_manifest_format_version, read_manifest_schema_version, replace_active_store,
        sync_parent_directory, validate_owner_directory_metadata, validate_owner_regular_metadata,
        ActiveStoreManifest, MANIFEST_FILE,
    },
    data_directory_owner::DataDirectoryOwnerGuard,
    forward_migration,
    migration_v27::{open_encrypted_read_connection, store_identity, sync_validated_store},
    migration_v29, schema_v29, schema_v30, schema_v31, schema_v32, schema_v33, MetaStoreError,
    MetadataEncryptionState, OwnedMetaStore, Result, METADATA_ENCRYPTION_KEY_LEN,
};
use rusqlite::{backup::Backup, types::ValueRef, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

#[path = "current_store_initialization_receipt.rs"]
mod initialization_receipt;
#[path = "forward_migration_receipt.rs"]
mod receipt;

use initialization_receipt::{InitializationPhase, InitializationReceipt};
use receipt::{MigrationReceipt, ReceiptPhase};

const STAGING_PREFIX: &str = ".metadata-forward-stage-";

pub(super) fn active_store_path(data_dir: &Path) -> Result<PathBuf> {
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if !owner_regular_file_exists(&manifest_path)? {
        return Err(MetaStoreError::migration_ownership_required());
    }
    require_current_manifest(&manifest_path)?;
    Ok(data_dir.join(read_manifest(&manifest_path)?.file_name))
}

pub(super) fn migration_required(data_dir: &Path) -> Result<bool> {
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if !owner_regular_file_exists(&manifest_path)? {
        return Ok(false);
    }
    match read_manifest_schema_version(&manifest_path)? {
        schema_v29::VERSION if read_manifest_format_version(&manifest_path)? == 1 => Ok(true),
        schema_v30::VERSION if read_manifest_format_version(&manifest_path)? == 2 => Ok(true),
        schema_v31::VERSION | schema_v32::VERSION
            if read_manifest_format_version(&manifest_path)? == 2 =>
        {
            Ok(true)
        }
        schema_v33::VERSION if read_manifest_format_version(&manifest_path)? == 2 => Ok(false),
        _ => Err(MetaStoreError::unsupported_store_schema()),
    }
}

pub(super) fn open_current_store(
    data_dir: &Path,
) -> Result<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN], String)> {
    open_optional_current_store(data_dir)?.ok_or_else(MetaStoreError::migration_ownership_required)
}

pub(super) fn open_optional_current_store(
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
        migration_v29::reject_legacy_or_partial_authority(&data_dir)?;
        return Ok(None);
    }
    require_current_manifest(&manifest_path)?;
    let manifest = read_manifest(&manifest_path)?;
    validate_receipt_for_current_store(&data_dir, &manifest)?;
    let key = read_key(&data_dir)?;
    validate_current_store(
        &data_dir.join(&manifest.file_name),
        &key,
        &manifest.store_id_digest,
    )?;
    Ok(Some((
        data_dir.join(&manifest.file_name),
        key,
        manifest.store_id_digest,
    )))
}

pub(super) fn prepare_active_store(
    owner: &Arc<DataDirectoryOwnerGuard>,
) -> Result<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN])> {
    let data_dir = owner.canonical_data_dir();
    reconcile_initialization_receipt(data_dir)?;
    reconcile_receipt(data_dir)?;
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if !owner_regular_file_exists(&manifest_path)? {
        migration_v29::reject_legacy_or_partial_authority(data_dir)?;
        return create_fresh_store(owner);
    }

    let authority = (
        read_manifest_format_version(&manifest_path)?,
        read_manifest_schema_version(&manifest_path)?,
    );
    match authority {
        (2, schema_v33::VERSION) => {
            let manifest = read_manifest(&manifest_path)?;
            let key = read_key(data_dir)?;
            let path = data_dir.join(&manifest.file_name);
            validate_current_store(&path, &key, &manifest.store_id_digest)?;
            Ok((path, key))
        }
        (1, schema_v29::VERSION)
        | (2, schema_v30::VERSION)
        | (2, schema_v31::VERSION)
        | (2, schema_v32::VERSION) => {
            let manifest = read_manifest(&manifest_path)?;
            let key = read_key(data_dir)?;
            migrate_prior(owner, manifest, key)
        }
        _ => Err(MetaStoreError::unsupported_store_schema()),
    }
}

pub(super) fn validate_current_connection(
    connection: &Connection,
    store_id_digest: &str,
) -> Result<()> {
    migration_v29::validate_active_connection(connection, schema_v33::VERSION, store_id_digest)?;
    forward_migration::validate_chain(connection, schema_v29::VERSION, schema_v33::VERSION)
}

fn migrate_prior(
    owner: &Arc<DataDirectoryOwnerGuard>,
    source_manifest: ActiveStoreManifest,
    key: [u8; METADATA_ENCRYPTION_KEY_LEN],
) -> Result<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN])> {
    let data_dir = owner.canonical_data_dir();
    let source_path = data_dir.join(&source_manifest.file_name);
    let source = open_encrypted_read_connection(&source_path, &key)?;
    validate_source_store(&source, &source_manifest)?;
    let source_witness = PreservationWitness::capture(&source)?;
    // A store migrated by an earlier product version may still retain its own
    // predecessor and published receipt. Validate the active source first,
    // then retire that older recovery point before creating this generation's
    // receipt so there is always exactly one recoverable predecessor.
    destroy_retained_predecessor(data_dir)?;

    let migration_id = random_store_id_digest()?;
    let staging_file = format!("{STAGING_PREFIX}{}.sqlite3", &migration_id[..16]);
    let target_file = format!("metadata-v33-{}.sqlite3", &migration_id[..16]);
    let target_manifest = ActiveStoreManifest {
        file_name: target_file.clone(),
        schema_version: schema_v33::VERSION,
        store_id_digest: source_manifest.store_id_digest.clone(),
    };
    let mut receipt = MigrationReceipt {
        phase: ReceiptPhase::Preparing,
        migration_id,
        source: source_manifest.clone(),
        staging_file,
        target: target_manifest.clone(),
    };
    receipt::persist(data_dir, &receipt)?;

    let staging_path = data_dir.join(&receipt.staging_file);
    let target_path = data_dir.join(&target_file);
    let migration_result = (|| {
        copy_encrypted_store(&source, &staging_path, &key)?;
        let mut staging = open_existing_encrypted_writer(&staging_path, &key)?;
        forward_migration::apply_current_schema(&mut staging, source_manifest.schema_version)?;
        validate_current_connection(&staging, &target_manifest.store_id_digest)?;
        if !source_witness.matches(&staging)? {
            return Err(MetaStoreError::storage_invariant());
        }
        drop(staging);
        sync_validated_store(&staging_path)?;
        fs::rename(&staging_path, &target_path).map_err(MetaStoreError::io_storage)?;
        sync_parent_directory(data_dir)?;
        validate_current_store(&target_path, &key, &target_manifest.store_id_digest)?;
        receipt.phase = ReceiptPhase::Ready;
        receipt::persist(data_dir, &receipt)?;
        replace_active_store(data_dir, &source_manifest, &target_manifest, || Ok(()))?;
        receipt.phase = ReceiptPhase::Published;
        receipt::persist(data_dir, &receipt)?;
        Ok(())
    })();
    if let Err(error) = migration_result {
        let published = read_manifest(&data_dir.join(MANIFEST_FILE))
            .is_ok_and(|manifest| manifest == target_manifest);
        if published
            && validate_current_store(&target_path, &key, &target_manifest.store_id_digest).is_ok()
        {
            receipt.phase = ReceiptPhase::Published;
            receipt::persist(data_dir, &receipt)?;
        } else {
            let _ = cleanup_unpublished_files(data_dir, &receipt);
            return Err(error);
        }
    }
    Ok((target_path, key))
}

fn create_fresh_store(
    owner: &Arc<DataDirectoryOwnerGuard>,
) -> Result<(PathBuf, [u8; METADATA_ENCRYPTION_KEY_LEN])> {
    let data_dir = owner.canonical_data_dir();
    let initialization_id = random_store_id_digest()?;
    let mut receipt = InitializationReceipt::new(initialization_id);
    initialization_receipt::persist(data_dir, &receipt)?;
    let key = crate::random_metadata_encryption_key()?;
    let staging_path = data_dir.join(receipt.staging_file());
    let target_path = data_dir.join(receipt.target_file());
    let connection = create_encrypted_writer(&staging_path, &key)?;
    let store = OwnedMetaStore::from_owned_connection(
        connection,
        MetadataEncryptionState::SqlCipher,
        Arc::clone(owner),
    )?;
    let report = store.initialize_current_schema()?;
    if report
        .applied_versions()
        .iter()
        .copied()
        .ne(1..=schema_v33::VERSION)
    {
        return Err(MetaStoreError::storage_invariant());
    }
    let store_id_digest = store_identity(&store.connection.borrow())?;
    drop(store);
    sync_validated_store(&staging_path)?;
    fs::rename(&staging_path, &target_path).map_err(MetaStoreError::io_storage)?;
    sync_parent_directory(data_dir)?;
    let manifest = ActiveStoreManifest {
        file_name: receipt.target_file().to_string(),
        schema_version: schema_v33::VERSION,
        store_id_digest: store_id_digest.clone(),
    };
    validate_current_store(&target_path, &key, &store_id_digest)?;
    receipt.mark_ready(store_id_digest);
    initialization_receipt::persist(data_dir, &receipt)?;
    publish_key(data_dir, &key)?;
    publish_new_active_store(data_dir, &manifest, || Ok(()))?;
    initialization_receipt::remove(data_dir)?;
    Ok((target_path, key))
}

fn copy_encrypted_store(source: &Connection, staging_path: &Path, key: &[u8]) -> Result<()> {
    let mut destination = create_encrypted_writer(staging_path, key)?;
    let backup = Backup::new(source, &mut destination).map_err(MetaStoreError::storage)?;
    backup
        .run_to_completion(256, Duration::from_millis(1), None)
        .map_err(MetaStoreError::storage)?;
    drop(backup);
    destination
        .execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(MetaStoreError::storage)
}

fn create_encrypted_writer(path: &Path, key: &[u8]) -> Result<Connection> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(MetaStoreError::io_storage)?;
    crate::restrict_private_file_permissions(path)?;
    open_existing_encrypted_writer(path, key)
}

fn open_existing_encrypted_writer(path: &Path, key: &[u8]) -> Result<Connection> {
    let connection = Connection::open(path).map_err(MetaStoreError::storage)?;
    crate::apply_sqlcipher_key(&connection, key)?;
    crate::verify_sqlcipher_key(&connection)?;
    Ok(connection)
}

fn validate_current_store(path: &Path, key: &[u8], store_id_digest: &str) -> Result<()> {
    if !owner_regular_file_exists(path)? {
        return Err(MetaStoreError::storage_invariant());
    }
    let connection = open_encrypted_read_connection(path, key)?;
    validate_current_connection(&connection, store_id_digest)
}

fn validate_store_for_manifest(
    data_dir: &Path,
    key: &[u8],
    manifest: &ActiveStoreManifest,
) -> Result<()> {
    let path = data_dir.join(&manifest.file_name);
    if manifest.schema_version == schema_v33::VERSION {
        validate_current_store(&path, key, &manifest.store_id_digest)
    } else {
        if !owner_regular_file_exists(&path)? {
            return Err(MetaStoreError::storage_invariant());
        }
        let connection = open_encrypted_read_connection(&path, key)?;
        validate_source_store(&connection, manifest)
    }
}

fn require_current_manifest(path: &Path) -> Result<()> {
    if read_manifest_format_version(path)? != 2
        || read_manifest_schema_version(path)? != schema_v33::VERSION
    {
        return Err(MetaStoreError::unsupported_store_schema());
    }
    Ok(())
}

fn validate_source_store(connection: &Connection, manifest: &ActiveStoreManifest) -> Result<()> {
    match manifest.schema_version {
        schema_v29::VERSION => {
            migration_v29::validate_current_v29_connection(connection, &manifest.store_id_digest)
        }
        schema_v30::VERSION | schema_v31::VERSION | schema_v32::VERSION => {
            migration_v29::validate_active_connection(
                connection,
                manifest.schema_version,
                &manifest.store_id_digest,
            )?;
            forward_migration::validate_chain(
                connection,
                schema_v29::VERSION,
                manifest.schema_version,
            )
        }
        _ => Err(MetaStoreError::unsupported_store_schema()),
    }
}

fn read_key(data_dir: &Path) -> Result<[u8; METADATA_ENCRYPTION_KEY_LEN]> {
    crate::read_metadata_encryption_key_without_repair(&crate::metadata_encryption_key_path(
        data_dir,
    ))
}

fn publish_key(data_dir: &Path, key: &[u8]) -> Result<()> {
    let key_path = crate::metadata_encryption_key_path(data_dir);
    let key_directory = key_path
        .parent()
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.encryption_key_path"))?;
    fs::create_dir(key_directory).map_err(MetaStoreError::io_storage)?;
    restrict_private_directory_permissions(key_directory)?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&key_path)
        .map_err(MetaStoreError::io_storage)?;
    file.write_all(crate::encode_hex(key).as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(MetaStoreError::io_storage)?;
    crate::restrict_private_file_permissions(&key_path)?;
    sync_parent_directory(key_directory)
}

fn remove_fresh_key(data_dir: &Path) -> Result<()> {
    let key_path = crate::metadata_encryption_key_path(data_dir);
    let key_directory = key_path
        .parent()
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.encryption_key_path"))?;
    match fs::symlink_metadata(&key_path) {
        Ok(metadata) => {
            validate_owner_regular_metadata(&metadata)?;
            fs::remove_file(&key_path).map_err(MetaStoreError::io_storage)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(MetaStoreError::io_storage(error)),
    }
    match fs::symlink_metadata(key_directory) {
        Ok(metadata) => {
            validate_owner_directory_metadata(&metadata)?;
            fs::remove_dir(key_directory).map_err(MetaStoreError::io_storage)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(MetaStoreError::io_storage(error)),
    }
    sync_parent_directory(data_dir)
}

fn reconcile_initialization_receipt(data_dir: &Path) -> Result<()> {
    let receipt_path = initialization_receipt::path(data_dir);
    if !owner_regular_file_exists(&receipt_path)? {
        return Ok(());
    }
    let receipt = initialization_receipt::read(&receipt_path)?;
    let manifest_path = data_dir.join(MANIFEST_FILE);
    if owner_regular_file_exists(&manifest_path)? {
        let manifest = read_manifest(&manifest_path)?;
        let target = receipt
            .target_manifest()
            .ok_or_else(MetaStoreError::storage_invariant)?;
        if manifest != target {
            return Err(MetaStoreError::storage_invariant());
        }
        validate_current_store(
            &data_dir.join(&manifest.file_name),
            &read_key(data_dir)?,
            &manifest.store_id_digest,
        )?;
        initialization_receipt::remove(data_dir)?;
        return Ok(());
    }

    match receipt.phase() {
        InitializationPhase::Preparing => cleanup_initialization(data_dir, &receipt),
        InitializationPhase::Ready => {
            let Some(target) = receipt.target_manifest() else {
                return Err(MetaStoreError::storage_invariant());
            };
            let key = match read_key(data_dir) {
                Ok(key) => key,
                Err(_) => return cleanup_initialization(data_dir, &receipt),
            };
            let target_path = data_dir.join(&target.file_name);
            if validate_current_store(&target_path, &key, &target.store_id_digest).is_err() {
                return cleanup_initialization(data_dir, &receipt);
            }
            publish_new_active_store(data_dir, &target, || Ok(()))?;
            initialization_receipt::remove(data_dir)
        }
    }
}

fn cleanup_initialization(data_dir: &Path, receipt: &InitializationReceipt) -> Result<()> {
    for file_name in [receipt.staging_file(), receipt.target_file()] {
        let path = data_dir.join(file_name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                validate_owner_regular_metadata(&metadata)?;
                fs::remove_file(path).map_err(MetaStoreError::io_storage)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(MetaStoreError::io_storage(error)),
        }
    }
    let key_directory = crate::metadata_encryption_key_path(data_dir)
        .parent()
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.encryption_key_path"))?
        .to_path_buf();
    match fs::symlink_metadata(&key_directory) {
        Ok(_) => remove_fresh_key(data_dir)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(MetaStoreError::io_storage(error)),
    }
    initialization_receipt::remove(data_dir)
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

fn reconcile_receipt(data_dir: &Path) -> Result<()> {
    let path = receipt::path(data_dir);
    if !owner_regular_file_exists(&path)? {
        return Ok(());
    }
    let mut receipt = receipt::read(&path)?;
    let manifest = read_manifest(&data_dir.join(MANIFEST_FILE))?;
    if manifest == receipt.target {
        validate_store_for_manifest(data_dir, &read_key(data_dir)?, &receipt.target)?;
        if receipt.phase != ReceiptPhase::Published {
            receipt.phase = ReceiptPhase::Published;
            receipt::persist(data_dir, &receipt)?;
        }
        return Ok(());
    }
    if manifest != receipt.source {
        return Err(MetaStoreError::storage_invariant());
    }
    match receipt.phase {
        ReceiptPhase::Preparing => {
            cleanup_unpublished_files(data_dir, &receipt)?;
            fs::remove_file(path).map_err(MetaStoreError::io_storage)?;
            sync_parent_directory(data_dir)
        }
        ReceiptPhase::Ready => {
            let key = read_key(data_dir)?;
            validate_store_for_manifest(data_dir, &key, &receipt.target)?;
            replace_active_store(data_dir, &receipt.source, &receipt.target, || Ok(()))?;
            receipt.phase = ReceiptPhase::Published;
            receipt::persist(data_dir, &receipt)
        }
        ReceiptPhase::Published => Err(MetaStoreError::storage_invariant()),
    }
}

fn validate_receipt_for_current_store(
    data_dir: &Path,
    manifest: &ActiveStoreManifest,
) -> Result<()> {
    let path = receipt::path(data_dir);
    if !owner_regular_file_exists(&path)? {
        return Ok(());
    }
    let receipt = receipt::read(&path)?;
    if receipt.target != *manifest
        || !matches!(receipt.phase, ReceiptPhase::Ready | ReceiptPhase::Published)
    {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}

fn cleanup_unpublished_files(data_dir: &Path, receipt: &MigrationReceipt) -> Result<()> {
    for file_name in [&receipt.staging_file, &receipt.target.file_name] {
        let path = data_dir.join(file_name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                validate_owner_regular_metadata(&metadata)?;
                fs::remove_file(path).map_err(MetaStoreError::io_storage)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(MetaStoreError::io_storage(error)),
        }
    }
    sync_parent_directory(data_dir)
}

pub(super) fn destroy_retained_predecessor(data_dir: &Path) -> Result<bool> {
    let receipt_path = receipt::path(data_dir);
    if !owner_regular_file_exists(&receipt_path)? {
        return Ok(false);
    }
    let receipt = receipt::read(&receipt_path)?;
    let manifest = read_manifest(&data_dir.join(MANIFEST_FILE))?;
    if receipt.phase != ReceiptPhase::Published || receipt.target != manifest {
        return Err(MetaStoreError::storage_invariant());
    }
    let predecessor = data_dir.join(&receipt.source.file_name);
    match fs::symlink_metadata(&predecessor) {
        Ok(metadata) => {
            validate_owner_regular_metadata(&metadata)?;
            fs::remove_file(predecessor).map_err(MetaStoreError::io_storage)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(MetaStoreError::io_storage(error)),
    }
    fs::remove_file(receipt_path).map_err(MetaStoreError::io_storage)?;
    sync_parent_directory(data_dir)?;
    Ok(true)
}

#[derive(Debug, Eq, PartialEq)]
struct PreservationWitness {
    source_tables: Vec<String>,
    logical_data_digest: String,
    documents: i64,
    versions: i64,
    projections: i64,
    head: Option<(Option<String>, i64)>,
    artifact: Option<(String, String, Option<String>, Option<String>)>,
}

impl PreservationWitness {
    fn capture(connection: &Connection) -> Result<Self> {
        let source_tables = preserved_source_tables(connection)?;
        let logical_data_digest = logical_data_digest(connection, &source_tables)?;
        let count = |table: &str| {
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(MetaStoreError::storage)
        };
        let head = connection
            .query_row(
                "SELECT generation, visible_epoch FROM search_projection_state
                 WHERE state_key = 'default'",
                [],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        let artifact = connection
            .query_row(
                "SELECT publication_fingerprint, projection_digest,
                        fulltext_logical_content_digest, vector_logical_content_digest
                 FROM search_publication_journal
                 WHERE generation = (
                     SELECT generation FROM search_projection_state
                     WHERE state_key = 'default'
                 )",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        Ok(Self {
            source_tables,
            logical_data_digest,
            documents: count("document")?,
            versions: count("resume_version")?,
            projections: count("active_search_projection")?,
            head,
            artifact,
        })
    }

    fn matches(&self, connection: &Connection) -> Result<bool> {
        let after = Self {
            source_tables: self.source_tables.clone(),
            logical_data_digest: logical_data_digest(connection, &self.source_tables)?,
            documents: connection
                .query_row("SELECT COUNT(*) FROM document", [], |row| row.get(0))
                .map_err(MetaStoreError::storage)?,
            versions: connection
                .query_row("SELECT COUNT(*) FROM resume_version", [], |row| row.get(0))
                .map_err(MetaStoreError::storage)?,
            projections: connection
                .query_row("SELECT COUNT(*) FROM active_search_projection", [], |row| {
                    row.get(0)
                })
                .map_err(MetaStoreError::storage)?,
            head: connection
                .query_row(
                    "SELECT generation, visible_epoch FROM search_projection_state
                     WHERE state_key = 'default'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(MetaStoreError::storage)?,
            artifact: connection
                .query_row(
                    "SELECT publication_fingerprint, projection_digest,
                            fulltext_logical_content_digest, vector_logical_content_digest
                     FROM search_publication_journal
                     WHERE generation = (
                         SELECT generation FROM search_projection_state
                         WHERE state_key = 'default'
                     )",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()
                .map_err(MetaStoreError::storage)?,
        };
        Ok(&after == self)
    }
}

fn preserved_source_tables(connection: &Connection) -> Result<Vec<String>> {
    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_schema
             WHERE type = 'table'
               AND name NOT LIKE 'sqlite_%'
               AND name NOT IN ('schema_migrations', 'forward_migration_history')
             ORDER BY name",
        )
        .map_err(MetaStoreError::storage)?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(MetaStoreError::storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::storage)?;
    Ok(tables)
}

fn logical_data_digest(connection: &Connection, tables: &[String]) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(b"resume-ir.metadata-logical-preservation.v1");
    for table in tables {
        update_digest_part(&mut digest, table.as_bytes());
        let quoted_table = quote_identifier(table);
        let mut column_statement = connection
            .prepare(&format!("PRAGMA table_info({quoted_table})"))
            .map_err(MetaStoreError::storage)?;
        let columns = column_statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        drop(column_statement);
        if columns.is_empty() {
            return Err(MetaStoreError::storage_invariant());
        }
        for column in &columns {
            update_digest_part(&mut digest, column.as_bytes());
        }
        let order = columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let mut row_statement = connection
            .prepare(&format!("SELECT * FROM {quoted_table} ORDER BY {order}"))
            .map_err(MetaStoreError::storage)?;
        let column_count = columns.len();
        let mut rows = row_statement.query([]).map_err(MetaStoreError::storage)?;
        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            digest.update(b"row");
            for index in 0..column_count {
                match row.get_ref(index).map_err(MetaStoreError::storage)? {
                    ValueRef::Null => digest.update(b"null"),
                    ValueRef::Integer(value) => {
                        digest.update(b"integer");
                        update_digest_part(&mut digest, &value.to_be_bytes());
                    }
                    ValueRef::Real(value) => {
                        digest.update(b"real");
                        update_digest_part(&mut digest, &value.to_bits().to_be_bytes());
                    }
                    ValueRef::Text(value) => {
                        digest.update(b"text");
                        update_digest_part(&mut digest, value);
                    }
                    ValueRef::Blob(value) => {
                        digest.update(b"blob");
                        update_digest_part(&mut digest, value);
                    }
                }
            }
        }
    }
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn update_digest_part(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

#[cfg(test)]
#[path = "current_store_tests.rs"]
mod tests;
