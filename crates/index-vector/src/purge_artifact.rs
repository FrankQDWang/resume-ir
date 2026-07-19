//! Exact classification of vector-index control-plane artifacts for purge scans.

use std::fs;
use std::path::{Component, Path};

use crate::store::{
    canonical_index_root, require_regular_directory, validate_generation,
    validate_private_lock_metadata, GENERATION_PINS_DIR, PUBLICATION_LOCK_FILE, READER_LOCK_FILE,
};
use crate::VectorIndexError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Whether one vector artifact contains data or is a validated control.
pub enum VectorPurgeArtifactClass {
    /// An ordinary artifact whose bytes must remain in the residual scan.
    Data,
    /// A validated control directory whose entries must still be classified.
    ControlPlaneDirectory,
    /// A validated empty lock file that must not be opened by the scanner.
    ControlPlaneFile,
}

/// Classifies one existing artifact under an exact canonical vector root.
pub fn classify_purge_artifact(
    canonical_root: &Path,
    path: &Path,
) -> Result<VectorPurgeArtifactClass, VectorIndexError> {
    if canonical_index_root(canonical_root)? != canonical_root {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    let relative = strict_relative_path(canonical_root, path)?;
    if matches!(
        relative.to_str(),
        Some(READER_LOCK_FILE | PUBLICATION_LOCK_FILE)
    ) {
        return validate_control_file(path);
    }
    if relative == Path::new(GENERATION_PINS_DIR) {
        require_regular_directory(path)?;
        return Ok(VectorPurgeArtifactClass::ControlPlaneDirectory);
    }

    let mut components = relative.components();
    if components.next() == Some(Component::Normal(GENERATION_PINS_DIR.as_ref()))
        && components.clone().count() == 1
    {
        let Component::Normal(name) = components.next().expect("one component counted") else {
            return Err(VectorIndexError::StorageLayoutInvalid);
        };
        let Some(name) = name.to_str() else {
            return Ok(VectorPurgeArtifactClass::Data);
        };
        let Some(generation) = name.strip_suffix(".lock") else {
            return Ok(VectorPurgeArtifactClass::Data);
        };
        validate_generation(generation)?;
        return validate_control_file(path);
    }

    Ok(VectorPurgeArtifactClass::Data)
}

fn strict_relative_path<'a>(root: &Path, path: &'a Path) -> Result<&'a Path, VectorIndexError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| VectorIndexError::LeaseRootMismatch)?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    Ok(relative)
}

fn validate_control_file(path: &Path) -> Result<VectorPurgeArtifactClass, VectorIndexError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| VectorIndexError::Storage)?;
    validate_private_lock_metadata(&metadata)?;
    if metadata.len() != 0 {
        return Err(VectorIndexError::StorageLayoutInvalid);
    }
    Ok(VectorPurgeArtifactClass::ControlPlaneFile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::private_storage::create_private_directory;
    use crate::store::open_lock_file;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn classifier_excludes_only_exact_empty_vector_controls() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("resume-ir-vector-purge-{unique}"));
        create_private_directory(&root).unwrap();
        let canonical = fs::canonicalize(&root).unwrap();
        create_private_directory(&canonical.join(GENERATION_PINS_DIR)).unwrap();
        for path in [
            canonical.join(READER_LOCK_FILE),
            canonical.join(PUBLICATION_LOCK_FILE),
            canonical
                .join(GENERATION_PINS_DIR)
                .join("generation-one.lock"),
        ] {
            drop(open_lock_file(&path, true).unwrap());
            assert_eq!(
                classify_purge_artifact(&canonical, &path).unwrap(),
                VectorPurgeArtifactClass::ControlPlaneFile
            );
        }
        assert_eq!(
            classify_purge_artifact(&canonical, &canonical.join(GENERATION_PINS_DIR)).unwrap(),
            VectorPurgeArtifactClass::ControlPlaneDirectory
        );
        let similar = canonical.join("snapshot-readers.lock.backup");
        fs::write(&similar, b"ordinary data").unwrap();
        assert_eq!(
            classify_purge_artifact(&canonical, &similar).unwrap(),
            VectorPurgeArtifactClass::Data
        );
        fs::write(canonical.join(PUBLICATION_LOCK_FILE), b"contaminated").unwrap();
        assert!(
            classify_purge_artifact(&canonical, &canonical.join(PUBLICATION_LOCK_FILE)).is_err()
        );
        assert!(
            classify_purge_artifact(&canonical, &canonical.parent().unwrap().join("outside"))
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
