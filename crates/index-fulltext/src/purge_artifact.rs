//! Exact classification of full-text control-plane artifacts for purge scans.

use std::fs;
use std::path::{Component, Path};

use crate::{
    validate_private_lock_metadata, validate_snapshot_directory, validate_snapshot_name,
    FullTextError, Result, GENERATION_PINS_DIR, SNAPSHOT_PUBLICATION_LOCK_FILE,
    SNAPSHOT_READER_LOCK_FILE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Whether one full-text artifact contains data or is a validated control.
pub enum FullTextPurgeArtifactClass {
    /// An ordinary artifact whose bytes must remain in the residual scan.
    Data,
    /// A validated control directory whose entries must still be classified.
    ControlPlaneDirectory,
    /// A validated empty lock file that must not be opened by the scanner.
    ControlPlaneFile,
}

/// Classifies one existing artifact under an exact canonical full-text root.
pub fn classify_purge_artifact(
    canonical_index_root: &Path,
    path: &Path,
) -> Result<FullTextPurgeArtifactClass> {
    require_canonical_root(canonical_index_root)?;
    let relative = strict_relative_path(canonical_index_root, path)?;
    if matches!(
        relative.to_str(),
        Some(SNAPSHOT_READER_LOCK_FILE | SNAPSHOT_PUBLICATION_LOCK_FILE)
    ) {
        return validate_control_file(path);
    }
    if relative == Path::new(GENERATION_PINS_DIR) {
        validate_snapshot_directory(path)?;
        return Ok(FullTextPurgeArtifactClass::ControlPlaneDirectory);
    }

    let mut components = relative.components();
    if components.next() == Some(Component::Normal(GENERATION_PINS_DIR.as_ref()))
        && components.clone().count() == 1
    {
        let Component::Normal(name) = components.next().expect("one component counted") else {
            return Err(FullTextError::internal(
                "full-text purge artifact path invalid",
            ));
        };
        let Some(name) = name.to_str() else {
            return Ok(FullTextPurgeArtifactClass::Data);
        };
        let Some(snapshot_name) = name.strip_suffix(".lock") else {
            return Ok(FullTextPurgeArtifactClass::Data);
        };
        validate_snapshot_name(snapshot_name)?;
        return validate_control_file(path);
    }

    Ok(FullTextPurgeArtifactClass::Data)
}

fn require_canonical_root(root: &Path) -> Result<()> {
    let canonical = fs::canonicalize(root).map_err(FullTextError::io)?;
    if canonical != root {
        return Err(FullTextError::internal(
            "full-text purge root is not canonical",
        ));
    }
    validate_snapshot_directory(root)
}

fn strict_relative_path<'a>(root: &Path, path: &'a Path) -> Result<&'a Path> {
    let relative = path.strip_prefix(root).map_err(|_| {
        FullTextError::internal("full-text purge artifact is outside the index root")
    })?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(FullTextError::internal(
            "full-text purge artifact path invalid",
        ));
    }
    Ok(relative)
}

fn validate_control_file(path: &Path) -> Result<FullTextPurgeArtifactClass> {
    let metadata = fs::symlink_metadata(path).map_err(FullTextError::io)?;
    validate_private_lock_metadata(&metadata)?;
    if metadata.len() != 0 {
        return Err(FullTextError::internal(
            "full-text purge control artifact is not empty",
        ));
    }
    Ok(FullTextPurgeArtifactClass::ControlPlaneFile)
}
