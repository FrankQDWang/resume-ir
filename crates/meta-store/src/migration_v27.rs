#[cfg(any(test, feature = "migration-test-support"))]
use std::fs::{self, File};
use std::{fs::OpenOptions, path::Path};

#[cfg(any(test, feature = "migration-test-support"))]
use fs4::fs_std::FileExt;

#[cfg(any(test, feature = "migration-test-support"))]
use crate::active_store_manifest::{validate_owner_regular_metadata, ActiveStoreManifest};
#[cfg(any(test, feature = "migration-test-support"))]
use crate::restrict_private_file_permissions;
use crate::{MetaStoreError, Result};

#[cfg(any(test, feature = "migration-test-support"))]
mod cleanup;
mod store_validation;

#[cfg(any(test, feature = "migration-test-support"))]
use cleanup::{complete_legacy_cleanup, discard_unpublished_cleanup};
#[cfg(any(test, feature = "migration-test-support"))]
pub(super) use store_validation::{open_encrypted_connection, validate_active_store};
pub(super) use store_validation::{
    open_encrypted_read_connection, source_schema_version, store_identity,
};

pub(crate) const MIGRATION_LOCK_FILE: &str = "metadata-migration.lock";

#[cfg(any(test, feature = "migration-test-support"))]
pub(super) fn with_migration_lock<T>(
    data_dir: &Path,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
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

#[cfg(any(test, feature = "migration-test-support"))]
pub(super) fn discard_pending_v27_legacy_cleanup(data_dir: &Path) -> Result<()> {
    discard_unpublished_cleanup(data_dir)
}

#[cfg(any(test, feature = "migration-test-support"))]
pub(super) fn complete_pending_v27_legacy_cleanup(
    data_dir: &Path,
    manifest: &ActiveStoreManifest,
) -> Result<()> {
    complete_legacy_cleanup(data_dir, manifest)
}

pub(super) fn sync_validated_store(path: &Path) -> Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(MetaStoreError::io_storage)?
        .sync_all()
        .map_err(MetaStoreError::io_storage)
}

#[cfg(any(test, feature = "migration-test-support"))]
pub(super) fn create_private_empty_file(path: &Path) -> Result<()> {
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

#[cfg(any(test, feature = "migration-test-support"))]
pub(super) fn legacy_owner_regular_file_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err(MetaStoreError::invalid_value("metadata.owner_file"));
            }
            restrict_private_file_permissions(path)?;
            let restricted = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
            validate_owner_regular_metadata(&restricted)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(MetaStoreError::io_storage(error)),
    }
}

#[cfg(any(test, feature = "migration-test-support"))]
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
