#[cfg(any(test, feature = "migration-test-support"))]
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use super::lock_ops::{self, ExclusiveLockAttempt};
use super::{private_lock_options, same_file_identity};
use crate::{ImportTaskId, MetaStoreError, Result as StoreResult};

pub(super) const IMPORT_TASK_OWNER_LOCKS_DIR: &str = "import-task-locks";

pub fn import_task_owner_lock_path(data_dir: &Path, task_id: &ImportTaskId) -> PathBuf {
    data_dir
        .join(IMPORT_TASK_OWNER_LOCKS_DIR)
        .join(format!("{task_id}.lock"))
}

/// Legacy per-task exclusion retained at the storage boundary so a v28 owner
/// can detect and fence older writers before discarding their task rows.
pub struct ImportTaskOwnerLock {
    file: File,
}

impl ImportTaskOwnerLock {
    pub fn acquire(data_dir: &Path, task_id: &ImportTaskId) -> io::Result<Self> {
        let path = import_task_owner_lock_path(data_dir, task_id);
        let file = open_task_lock_file(&path)?;
        lock_ops::lock_exclusive(&file)?;
        validate_open_task_lock_file(&path, &file)?;
        Ok(Self { file })
    }

    pub fn try_acquire(data_dir: &Path, task_id: &ImportTaskId) -> io::Result<Option<Self>> {
        let path = import_task_owner_lock_path(data_dir, task_id);
        let file = open_task_lock_file(&path)?;
        match lock_ops::try_exclusive(&file) {
            Ok(ExclusiveLockAttempt::Acquired) => {
                validate_open_task_lock_file(&path, &file)?;
                Ok(Some(Self { file }))
            }
            Ok(ExclusiveLockAttempt::Contended) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl Drop for ImportTaskOwnerLock {
    fn drop(&mut self) {
        let _ = lock_ops::unlock(&self.file);
    }
}

#[cfg(any(test, feature = "migration-test-support"))]
pub(crate) fn acquire_legacy_task_locks(
    data_dir: &Path,
    task_ids: impl IntoIterator<Item = String>,
) -> StoreResult<Vec<ImportTaskOwnerLock>> {
    let mut task_ids = task_ids
        .into_iter()
        .map(|task_id| (task_id.clone(), task_id))
        .collect::<BTreeMap<_, _>>();
    let locks_dir = data_dir.join(IMPORT_TASK_OWNER_LOCKS_DIR);
    match fs::symlink_metadata(&locks_dir) {
        Ok(metadata) => {
            if validate_task_lock_directory_metadata(&metadata).is_err() {
                return Err(MetaStoreError::invalid_value("import_task.owner_locks_dir"));
            }
            for entry in fs::read_dir(&locks_dir).map_err(MetaStoreError::io_storage)? {
                let entry = entry.map_err(MetaStoreError::io_storage)?;
                let metadata = entry.metadata().map_err(MetaStoreError::io_storage)?;
                validate_task_lock_metadata(&metadata).map_err(MetaStoreError::io_storage)?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| MetaStoreError::invalid_value("import_task.owner_lock"))?;
                let value = name
                    .strip_suffix(".lock")
                    .ok_or_else(|| MetaStoreError::invalid_value("import_task.owner_lock"))?;
                validate_legacy_task_id(value)?;
                task_ids
                    .entry(value.to_string())
                    .or_insert_with(|| value.to_string());
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(MetaStoreError::io_storage(error)),
    }
    let mut locks = Vec::new();
    for task_id in task_ids.into_values() {
        validate_legacy_task_id(&task_id)?;
        let path = data_dir
            .join(IMPORT_TASK_OWNER_LOCKS_DIR)
            .join(format!("{task_id}.lock"));
        let file = open_task_lock_file(&path).map_err(MetaStoreError::io_storage)?;
        let lock = match lock_ops::try_exclusive(&file) {
            Ok(ExclusiveLockAttempt::Acquired) => ImportTaskOwnerLock { file },
            Ok(ExclusiveLockAttempt::Contended) => {
                return Err(MetaStoreError::migration_ownership_required());
            }
            Err(error) => {
                return Err(MetaStoreError::io_storage(error));
            }
        };
        validate_open_task_lock_file(&path, &lock.file).map_err(MetaStoreError::io_storage)?;
        locks.push(lock);
    }
    Ok(locks)
}

pub(super) fn validate_legacy_task_id(value: &str) -> StoreResult<()> {
    if value.is_empty()
        || value.len() > 256
        || matches!(value, "." | "..")
        || value.bytes().any(|byte| matches!(byte, b'/' | b'\\' | 0))
    {
        return Err(MetaStoreError::invalid_value("import_task.id"));
    }
    Ok(())
}

fn open_task_lock_file(path: &Path) -> io::Result<File> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "invalid import task owner lock"))?;
    fs::create_dir_all(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path) {
        validate_task_lock_metadata(&metadata)?;
    }
    let file = private_lock_options().open(path)?;
    validate_open_task_lock_file(path, &file)?;
    Ok(file)
}

fn validate_open_task_lock_file(path: &Path, file: &File) -> io::Result<()> {
    let opened = file.metadata()?;
    validate_task_lock_metadata(&opened)?;
    let current = fs::symlink_metadata(path)?;
    validate_task_lock_metadata(&current)?;
    if !same_file_identity(file, path, &opened, &current)? {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "import task owner lock identity changed",
        ));
    }
    Ok(())
}

pub(super) fn validate_task_lock_metadata(metadata: &fs::Metadata) -> io::Result<()> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "import task owner lock is not a regular file",
        ));
    }
    Ok(())
}

pub(super) fn validate_task_lock_directory_metadata(metadata: &fs::Metadata) -> io::Result<()> {
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "import task owner lock directory is not a regular directory",
        ));
    }
    Ok(())
}
