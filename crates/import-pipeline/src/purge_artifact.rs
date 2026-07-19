//! One purge-scanner boundary over metadata, full-text, and vector controls.

use std::path::{Component, Path, PathBuf};

use index_fulltext::FullTextPurgeArtifactClass;
use index_vector::VectorPurgeArtifactClass;
use meta_store::{DataDirectoryOwnerLease, MetaStorePurgeArtifactClass};

const FULLTEXT_INDEX_ROOT: &str = "search-index";
const VECTOR_INDEX_ROOT: &str = "vector-index";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Unified purge-scanner classification across all storage owners.
pub enum PurgeArtifactClass {
    /// An ordinary artifact whose bytes must remain in the residual scan.
    Data,
    /// A validated control directory whose entries must still be classified.
    ControlPlaneDirectory,
    /// A validated empty lock file that must not be opened by the scanner.
    ControlPlaneFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Storage boundary that rejected purge-artifact classification.
pub enum PurgeArtifactClassificationError {
    /// Metadata ownership or its lock namespace failed validation.
    Metadata,
    /// The full-text layout or lock namespace failed validation.
    FullText,
    /// The vector layout or lock namespace failed validation.
    Vector,
}

/// Retains the mutation owner while classifying purge-scan artifacts.
pub struct PurgeArtifactClassifier<'a> {
    owner: &'a DataDirectoryOwnerLease,
    fulltext_root: PathBuf,
    vector_root: PathBuf,
}

impl<'a> PurgeArtifactClassifier<'a> {
    /// Binds classification to a retained data-directory mutation owner.
    pub fn new(owner: &'a DataDirectoryOwnerLease) -> Self {
        let data_dir = owner.canonical_data_dir();
        Self {
            owner,
            fulltext_root: data_dir.join(FULLTEXT_INDEX_ROOT),
            vector_root: data_dir.join(VECTOR_INDEX_ROOT),
        }
    }

    /// Returns the exact canonical root protected by this classifier.
    pub fn data_dir(&self) -> &Path {
        self.owner.canonical_data_dir()
    }

    /// Classifies one existing artifact without opening validated lock files.
    pub fn classify(
        &self,
        path: &Path,
    ) -> Result<PurgeArtifactClass, PurgeArtifactClassificationError> {
        match self
            .owner
            .classify_purge_artifact(path)
            .map_err(|_| PurgeArtifactClassificationError::Metadata)?
        {
            MetaStorePurgeArtifactClass::ControlPlaneDirectory => {
                return Ok(PurgeArtifactClass::ControlPlaneDirectory);
            }
            MetaStorePurgeArtifactClass::ControlPlaneFile => {
                return Ok(PurgeArtifactClass::ControlPlaneFile);
            }
            MetaStorePurgeArtifactClass::Data => {}
        }

        let relative = path
            .strip_prefix(self.data_dir())
            .map_err(|_| PurgeArtifactClassificationError::Metadata)?;
        match relative.components().next() {
            Some(Component::Normal(name)) if name == FULLTEXT_INDEX_ROOT => map_fulltext(
                index_fulltext::classify_purge_artifact(&self.fulltext_root, path),
            ),
            Some(Component::Normal(name)) if name == VECTOR_INDEX_ROOT => map_vector(
                index_vector::classify_purge_artifact(&self.vector_root, path),
            ),
            _ => Ok(PurgeArtifactClass::Data),
        }
    }
}

fn map_fulltext(
    result: index_fulltext::Result<FullTextPurgeArtifactClass>,
) -> Result<PurgeArtifactClass, PurgeArtifactClassificationError> {
    result
        .map(|class| match class {
            FullTextPurgeArtifactClass::Data => PurgeArtifactClass::Data,
            FullTextPurgeArtifactClass::ControlPlaneDirectory => {
                PurgeArtifactClass::ControlPlaneDirectory
            }
            FullTextPurgeArtifactClass::ControlPlaneFile => PurgeArtifactClass::ControlPlaneFile,
        })
        .map_err(|_| PurgeArtifactClassificationError::FullText)
}

fn map_vector(
    result: Result<VectorPurgeArtifactClass, index_vector::VectorIndexError>,
) -> Result<PurgeArtifactClass, PurgeArtifactClassificationError> {
    result
        .map(|class| match class {
            VectorPurgeArtifactClass::Data => PurgeArtifactClass::Data,
            VectorPurgeArtifactClass::ControlPlaneDirectory => {
                PurgeArtifactClass::ControlPlaneDirectory
            }
            VectorPurgeArtifactClass::ControlPlaneFile => PurgeArtifactClass::ControlPlaneFile,
        })
        .map_err(|_| PurgeArtifactClassificationError::Vector)
}
