//! Exact classification of metadata control-plane artifacts for purge scans.

use std::fs;
use std::path::{Component, Path};

use super::task_lock::{
    validate_legacy_task_id, validate_task_lock_directory_metadata, validate_task_lock_metadata,
    IMPORT_TASK_OWNER_LOCKS_DIR,
};
use super::{
    validate_legacy_publication_lock_metadata, DataDirectoryOwnerLease,
    DATA_DIRECTORY_OWNER_LOCK_FILE, LEGACY_DAEMON_OWNER_LOCK_FILE, SEARCH_PUBLICATION_LOCK_FILE,
};
use crate::migration_v27::MIGRATION_LOCK_FILE;
use crate::{MetaStoreError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Whether one metadata artifact contains data or is a validated control.
pub enum MetaStorePurgeArtifactClass {
    /// An ordinary artifact whose bytes must remain in the residual scan.
    Data,
    /// A validated control directory whose entries must still be classified.
    ControlPlaneDirectory,
    /// A validated empty lock file that must not be opened by the scanner.
    ControlPlaneFile,
}

impl DataDirectoryOwnerLease {
    /// Classifies one existing artifact under this lease's canonical data root.
    ///
    /// Only exact, empty lock artifacts owned by metadata storage are exempt
    /// from content scanning. Malformed known controls and paths outside the
    /// retained owner root fail closed.
    pub fn classify_purge_artifact(&self, path: &Path) -> Result<MetaStorePurgeArtifactClass> {
        let relative = strict_relative_path(self.canonical_data_dir(), path)?;
        if relative == Path::new(DATA_DIRECTORY_OWNER_LOCK_FILE) {
            return validate_held_owner_lock(path, &self.guard.data_directory_lock);
        }
        if relative == Path::new(LEGACY_DAEMON_OWNER_LOCK_FILE) {
            return validate_held_owner_lock(path, &self.guard.legacy_daemon_lock);
        }
        if relative == Path::new(SEARCH_PUBLICATION_LOCK_FILE) {
            let metadata = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
            validate_legacy_publication_lock_metadata(&metadata)?;
            return require_empty_control_file(&metadata);
        }
        if relative == Path::new(MIGRATION_LOCK_FILE) {
            let metadata = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
            crate::active_store_manifest::validate_owner_regular_metadata(&metadata)?;
            return require_empty_control_file(&metadata);
        }
        if relative == Path::new(IMPORT_TASK_OWNER_LOCKS_DIR) {
            let metadata = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
            validate_task_lock_directory_metadata(&metadata).map_err(MetaStoreError::io_storage)?;
            return Ok(MetaStorePurgeArtifactClass::ControlPlaneDirectory);
        }

        let mut components = relative.components();
        if components.next() == Some(Component::Normal(IMPORT_TASK_OWNER_LOCKS_DIR.as_ref()))
            && components.clone().count() == 1
        {
            let Component::Normal(name) = components.next().expect("one component counted") else {
                return Err(MetaStoreError::invalid_value("purge.artifact_path"));
            };
            let Some(name) = name.to_str() else {
                return Ok(MetaStorePurgeArtifactClass::Data);
            };
            let Some(task_id) = name.strip_suffix(".lock") else {
                return Ok(MetaStorePurgeArtifactClass::Data);
            };
            validate_legacy_task_id(task_id)?;
            let metadata = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
            validate_task_lock_metadata(&metadata).map_err(MetaStoreError::io_storage)?;
            return require_empty_control_file(&metadata);
        }

        Ok(MetaStorePurgeArtifactClass::Data)
    }
}

fn strict_relative_path<'a>(root: &Path, path: &'a Path) -> Result<&'a Path> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| MetaStoreError::invalid_value("purge.artifact_path"))?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(MetaStoreError::invalid_value("purge.artifact_path"));
    }
    Ok(relative)
}

fn validate_held_owner_lock(path: &Path, held: &fs::File) -> Result<MetaStorePurgeArtifactClass> {
    super::validate_open_owner_lock_file(path, held)
        .map_err(|_| MetaStoreError::invalid_value("purge.control_artifact"))?;
    let metadata = held.metadata().map_err(MetaStoreError::io_storage)?;
    require_empty_control_file(&metadata)
}

fn require_empty_control_file(metadata: &fs::Metadata) -> Result<MetaStorePurgeArtifactClass> {
    if metadata.len() != 0 {
        return Err(MetaStoreError::invalid_value("purge.control_artifact"));
    }
    Ok(MetaStorePurgeArtifactClass::ControlPlaneFile)
}
