use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    active_store_manifest::{
        owner_regular_file_exists, remove_owner_file_if_exists, sync_parent_directory,
        validate_owner_regular_metadata, validate_store_file_name, validate_store_id_digest,
        ActiveStoreManifest, MANIFEST_MAX_BYTES,
    },
    MetaStoreError, Result, METADATA_STORE_FILE,
};

const LEGACY_CLEANUP_RECEIPT_FILE: &str = "metadata-legacy-cleanup.v1";
const LEGACY_CLEANUP_RECEIPT_SCHEMA: &str = "resume-ir.metadata-legacy-cleanup.v1";

struct LegacyCleanupReceipt {
    active_file_name: String,
    active_store_id_digest: String,
}

pub(super) fn discard_unpublished_cleanup(data_dir: &Path) -> Result<()> {
    let receipt_path = data_dir.join(LEGACY_CLEANUP_RECEIPT_FILE);
    if !owner_regular_file_exists(&receipt_path)? {
        return Ok(());
    }
    let receipt = read_receipt(&receipt_path)?;
    remove_store_artifacts(data_dir, &receipt.active_file_name)?;
    remove_owner_file_if_exists(&receipt_path)?;
    sync_parent_directory(data_dir)
}

pub(super) fn complete_legacy_cleanup(
    data_dir: &Path,
    manifest: &ActiveStoreManifest,
) -> Result<()> {
    let receipt_path = data_dir.join(LEGACY_CLEANUP_RECEIPT_FILE);
    if !owner_regular_file_exists(&receipt_path)? {
        return Ok(());
    }
    let receipt = read_receipt(&receipt_path)?;
    if receipt.active_file_name != manifest.file_name
        || receipt.active_store_id_digest != manifest.store_id_digest
        || manifest.file_name == METADATA_STORE_FILE
    {
        return Err(MetaStoreError::storage_invariant());
    }
    remove_store_artifacts(data_dir, METADATA_STORE_FILE)?;
    sync_parent_directory(data_dir)?;
    remove_owner_file_if_exists(&receipt_path)?;
    sync_parent_directory(data_dir)
}

fn read_receipt(path: &Path) -> Result<LegacyCleanupReceipt> {
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
    let active_file_name = required_value(lines.next(), "active")?.to_string();
    let active_store_id_digest = required_value(lines.next(), "digest")?.to_string();
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

fn required_value<'a>(line: Option<&'a str>, key: &str) -> Result<&'a str> {
    line.and_then(|line| line.strip_prefix(key))
        .and_then(|value| value.strip_prefix('='))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| MetaStoreError::invalid_value("metadata.legacy_cleanup_receipt"))
}

fn remove_store_artifacts(data_dir: &Path, file_name: &str) -> Result<()> {
    validate_store_file_name(file_name)?;
    let main = data_dir.join(file_name);
    for path in [
        main.clone(),
        sidecar_path(&main, "-journal"),
        sidecar_path(&main, "-wal"),
        sidecar_path(&main, "-shm"),
    ] {
        remove_owner_file_if_exists(&path)?;
    }
    Ok(())
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_owned();
    value.push(suffix);
    PathBuf::from(value)
}
