use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    manifest::{
        validate_store_file_name, validate_store_id_digest, ActiveStoreManifest, MANIFEST_MAX_BYTES,
    },
    owner_regular_file_exists, sync_parent_directory, validate_owner_regular_metadata,
    MigrationFailpoint,
};
use crate::{MetaStoreError, Result, METADATA_STORE_FILE};

pub(super) const LEGACY_CLEANUP_RECEIPT_FILE: &str = "metadata-legacy-cleanup.v1";
const LEGACY_CLEANUP_RECEIPT_SCHEMA: &str = "resume-ir.metadata-legacy-cleanup.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
struct LegacyCleanupReceipt {
    active_file_name: String,
    active_store_id_digest: String,
}

/// Persists cleanup intent before the manifest commit so a post-rename crash
/// can retire the exact legacy store without guessing which target won.
pub(super) fn write_legacy_cleanup_receipt(
    data_dir: &Path,
    active_file_name: &str,
    active_store_id_digest: &str,
) -> Result<()> {
    validate_store_file_name(active_file_name)?;
    validate_store_id_digest(active_store_id_digest)?;
    if active_file_name == METADATA_STORE_FILE {
        return Err(MetaStoreError::storage_invariant());
    }
    let path = data_dir.join(LEGACY_CLEANUP_RECEIPT_FILE);
    let bytes = format!(
        "{LEGACY_CLEANUP_RECEIPT_SCHEMA}\nactive={active_file_name}\ndigest={active_store_id_digest}"
    );
    crate::write_new_private_file(&path, bytes.as_bytes()).map_err(MetaStoreError::io_storage)?;
    sync_parent_directory(data_dir)
}

fn read_legacy_cleanup_receipt(path: &Path) -> Result<LegacyCleanupReceipt> {
    let metadata = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
    validate_owner_regular_metadata(&metadata)?;
    if metadata.len() > MANIFEST_MAX_BYTES {
        return Err(MetaStoreError::invalid_value(
            "metadata.legacy_cleanup_receipt",
        ));
    }
    let value = fs::read_to_string(path).map_err(MetaStoreError::io_storage)?;
    let mut lines = value.lines();
    if lines.next() != Some(LEGACY_CLEANUP_RECEIPT_SCHEMA) {
        return Err(MetaStoreError::invalid_value(
            "metadata.legacy_cleanup_receipt",
        ));
    }
    let active_file_name = lines
        .next()
        .and_then(|line| line.strip_prefix("active="))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.legacy_cleanup_receipt"))?
        .to_string();
    let active_store_id_digest = lines
        .next()
        .and_then(|line| line.strip_prefix("digest="))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.legacy_cleanup_receipt"))?
        .to_string();
    if lines.next().is_some() || active_file_name == METADATA_STORE_FILE {
        return Err(MetaStoreError::invalid_value(
            "metadata.legacy_cleanup_receipt",
        ));
    }
    validate_store_file_name(&active_file_name)?;
    validate_store_id_digest(&active_store_id_digest)?;
    Ok(LegacyCleanupReceipt {
        active_file_name,
        active_store_id_digest,
    })
}

pub(super) fn discard_unpublished_cleanup(data_dir: &Path) -> Result<()> {
    let receipt_path = data_dir.join(LEGACY_CLEANUP_RECEIPT_FILE);
    if !owner_regular_file_exists(&receipt_path)? {
        return Ok(());
    }
    let receipt = read_legacy_cleanup_receipt(&receipt_path)?;
    let unpublished_target = data_dir.join(receipt.active_file_name);
    remove_owner_file_if_exists(&unpublished_target)?;
    remove_owner_file_if_exists(&PathBuf::from(format!(
        "{}-wal",
        unpublished_target.display()
    )))?;
    remove_owner_file_if_exists(&PathBuf::from(format!(
        "{}-shm",
        unpublished_target.display()
    )))?;
    remove_owner_file_if_exists(&receipt_path)?;
    sync_parent_directory(data_dir)
}

pub(super) fn complete_legacy_cleanup(
    data_dir: &Path,
    manifest: &ActiveStoreManifest,
    failpoint: MigrationFailpoint,
) -> Result<()> {
    let receipt_path = data_dir.join(LEGACY_CLEANUP_RECEIPT_FILE);
    if !owner_regular_file_exists(&receipt_path)? {
        return Ok(());
    }
    let receipt = read_legacy_cleanup_receipt(&receipt_path)?;
    if receipt.active_file_name != manifest.file_name
        || receipt.active_store_id_digest != manifest.store_id_digest
    {
        return Err(MetaStoreError::storage_invariant());
    }
    let legacy_path = data_dir.join(METADATA_STORE_FILE);
    if data_dir.join(&manifest.file_name) == legacy_path {
        return Err(MetaStoreError::storage_invariant());
    }
    remove_owner_file_if_exists(&legacy_path)?;
    sync_parent_directory(data_dir)?;
    if failpoint == MigrationFailpoint::AfterLegacyMainDelete {
        return Err(MetaStoreError::storage_invariant());
    }
    remove_owner_file_if_exists(&PathBuf::from(format!("{}-wal", legacy_path.display())))?;
    remove_owner_file_if_exists(&PathBuf::from(format!("{}-shm", legacy_path.display())))?;
    sync_parent_directory(data_dir)?;
    if failpoint == MigrationFailpoint::AfterLegacySidecarDelete {
        return Err(MetaStoreError::storage_invariant());
    }
    remove_owner_file_if_exists(&receipt_path)?;
    sync_parent_directory(data_dir)
}

pub(super) fn remove_owner_file_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_owner_regular_metadata(&metadata)?;
            fs::remove_file(path).map_err(MetaStoreError::io_storage)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(MetaStoreError::io_storage(error)),
    }
}
