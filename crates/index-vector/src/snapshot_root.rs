use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use core_domain::ActiveSearchProjection;

use crate::ann::AnnIndex;
use crate::codec::{read_snapshot, KEY_FILE};
use crate::model::{validate_dimension, QueryVector, VectorDocument, VectorHit, VectorIndexError};
use crate::model_contract::VectorModelContract;
use crate::private_storage::PinnedPrivateDirectory;
use crate::snapshot_model::VectorSnapshotSummary;
use crate::store::{
    canonical_index_root, generation_pin_path, open_lock_file, require_regular_snapshot_directory,
    validate_generation, GENERATION_PINS_DIR, READER_LOCK_FILE, SNAPSHOTS_DIR,
};

/// Dimension-independent read and reclamation boundary for vector snapshots.
///
/// The exact model contract must come from the metadata publication head;
/// this type never discovers it from filesystem artifacts.
#[derive(Clone)]
pub struct VectorSnapshotRoot {
    pub(crate) root: PathBuf,
    pub(crate) root_identity: Arc<PinnedPrivateDirectory>,
}

impl VectorSnapshotRoot {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, VectorIndexError> {
        let root = canonical_index_root(root.as_ref())?;
        let root_identity = Arc::new(PinnedPrivateDirectory::acquire(&root)?);
        Ok(Self {
            root,
            root_identity,
        })
    }

    pub fn acquire_read_lease(&self) -> Result<VectorSnapshotReadLease, VectorIndexError> {
        self.root_identity.validate_current()?;
        let root_identity = PinnedPrivateDirectory::acquire(&self.root)?;
        if !self.root_identity.same_identity(&root_identity) {
            return Err(VectorIndexError::StorageLayoutInvalid);
        }
        let file = open_lock_file(&self.root.join(READER_LOCK_FILE), false)?;
        let snapshots_identity = PinnedPrivateDirectory::acquire(&self.root.join(SNAPSHOTS_DIR))?;
        let pins_identity = PinnedPrivateDirectory::acquire(&self.root.join(GENERATION_PINS_DIR))?;
        file.lock_shared().map_err(|_| VectorIndexError::Storage)?;
        let lease = VectorSnapshotReadLease {
            root: self.root.clone(),
            root_identity,
            snapshots_identity,
            pins_identity,
            file,
        };
        lease.validate_for(self)?;
        Ok(lease)
    }

    pub fn open_generation_with_lease(
        &self,
        generation: &str,
        expected_model_contract: &VectorModelContract,
        lease: VectorSnapshotReadLease,
    ) -> Result<VectorSnapshotReader, VectorIndexError> {
        validate_generation(generation)?;
        expected_model_contract.validate()?;
        lease.validate_for(self)?;
        let snapshot_dir = self.root.join(SNAPSHOTS_DIR).join(generation);
        require_regular_snapshot_directory(&snapshot_dir)?;
        let generation_identity = PinnedPrivateDirectory::acquire(&snapshot_dir)?;
        let generation_pin = open_lock_file(&generation_pin_path(&self.root, generation), false)
            .map_err(|error| match error {
                VectorIndexError::GenerationNotFound => VectorIndexError::StorageLayoutInvalid,
                other => other,
            })?;
        generation_pin
            .lock_shared()
            .map_err(|_| VectorIndexError::Storage)?;
        let generation_lease = VectorGenerationReadLease {
            root: self.root.clone(),
            _generation: generation.to_string(),
            file: generation_pin,
        };
        lease.validate_for(self)?;
        generation_identity.validate_current()?;
        require_regular_snapshot_directory(&snapshot_dir)?;
        let (projection, documents, summary) = read_snapshot(
            &snapshot_dir,
            &snapshot_dir.join(KEY_FILE),
            generation,
            expected_model_contract,
        )?;
        lease.validate_for(self)?;
        generation_identity.validate_current()?;
        drop(lease);
        let ann = AnnIndex::build(&documents);
        Ok(VectorSnapshotReader {
            _generation_lease: generation_lease,
            summary,
            projection,
            documents,
            ann,
        })
    }

    pub fn inspect_generation_with_lease(
        &self,
        generation: &str,
        expected_model_contract: &VectorModelContract,
        lease: &VectorSnapshotReadLease,
    ) -> VectorGenerationInspection {
        if validate_generation(generation).is_err() || expected_model_contract.validate().is_err() {
            return VectorGenerationInspection::new(VectorGenerationState::Invalid, None);
        }
        if lease.validate_for(self).is_err() {
            return VectorGenerationInspection::new(VectorGenerationState::Unreadable, None);
        }
        let snapshot_dir = self.root.join(SNAPSHOTS_DIR).join(generation);
        match require_regular_snapshot_directory(&snapshot_dir).and_then(|_| {
            read_snapshot(
                &snapshot_dir,
                &snapshot_dir.join(KEY_FILE),
                generation,
                expected_model_contract,
            )
        }) {
            Ok((_, _, summary)) => {
                VectorGenerationInspection::new(VectorGenerationState::Ready, Some(summary))
            }
            Err(VectorIndexError::GenerationNotFound) => {
                VectorGenerationInspection::new(VectorGenerationState::Missing, None)
            }
            Err(VectorIndexError::SchemaMismatch) => {
                VectorGenerationInspection::new(VectorGenerationState::Incompatible, None)
            }
            Err(VectorIndexError::Storage) => {
                VectorGenerationInspection::new(VectorGenerationState::Unreadable, None)
            }
            Err(_) => VectorGenerationInspection::new(VectorGenerationState::Corrupt, None),
        }
    }
}

impl fmt::Debug for VectorSnapshotRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorSnapshotRoot")
            .field("root", &"<redacted>")
            .finish()
    }
}

/// Root-wide acquisition fence held across metadata selection and exact open.
/// Exact open consumes it only after acquiring the selected generation pin.
pub struct VectorSnapshotReadLease {
    root: PathBuf,
    root_identity: PinnedPrivateDirectory,
    snapshots_identity: PinnedPrivateDirectory,
    pins_identity: PinnedPrivateDirectory,
    file: File,
}

impl VectorSnapshotReadLease {
    pub(crate) fn validate_for(&self, owner: &VectorSnapshotRoot) -> Result<(), VectorIndexError> {
        if self.root != owner.root {
            return Err(VectorIndexError::LeaseRootMismatch);
        }
        owner.root_identity.validate_current()?;
        self.root_identity.validate_current()?;
        self.snapshots_identity.validate_current()?;
        self.pins_identity.validate_current()?;
        if !owner.root_identity.same_identity(&self.root_identity) {
            return Err(VectorIndexError::StorageLayoutInvalid);
        }
        Ok(())
    }
}

struct VectorGenerationReadLease {
    root: PathBuf,
    _generation: String,
    file: File,
}

impl Drop for VectorGenerationReadLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl Drop for VectorSnapshotReadLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub struct VectorSnapshotReader {
    _generation_lease: VectorGenerationReadLease,
    summary: VectorSnapshotSummary,
    projection: Vec<ActiveSearchProjection>,
    documents: Vec<VectorDocument>,
    ann: AnnIndex,
}

impl VectorSnapshotReader {
    pub fn generation(&self) -> &str {
        self.summary.generation()
    }

    pub fn summary(&self) -> &VectorSnapshotSummary {
        &self.summary
    }

    /// Clones the complete exact-generation document set for construction of
    /// another immutable publication. The returned values remain version-bound;
    /// callers must not use this API as a query response surface.
    pub fn documents_for_republication(&self) -> Vec<VectorDocument> {
        self.documents.clone()
    }

    /// Returns the validated, document-sorted exact projection without
    /// allocating or cloning identity strings.
    pub fn exact_projection(&self) -> &[ActiveSearchProjection] {
        &self.projection
    }

    pub fn knn(&self, query: QueryVector, k: usize) -> Result<Vec<VectorHit>, VectorIndexError> {
        let Some(dimension) = self.summary.model_contract().dimension() else {
            return Err(VectorIndexError::SemanticUnavailable);
        };
        validate_dimension(dimension, query.values())?;
        self.ann.knn(query, k, None)
    }

    pub(crate) fn belongs_to(&self, root: &Path) -> bool {
        self._generation_lease.root == root
    }

    pub(crate) fn documents(&self) -> &[VectorDocument] {
        &self.documents
    }
}

impl fmt::Debug for VectorSnapshotReader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorSnapshotReader")
            .field("generation", &self.summary.generation())
            .field("model_contract", &self.summary.model_contract())
            .field("vector_count", &self.summary.vector_count())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorGenerationInspection {
    state: VectorGenerationState,
    summary: Option<VectorSnapshotSummary>,
}

impl VectorGenerationInspection {
    fn new(state: VectorGenerationState, summary: Option<VectorSnapshotSummary>) -> Self {
        Self { state, summary }
    }

    pub fn state(&self) -> VectorGenerationState {
        self.state
    }

    pub fn summary(&self) -> Option<&VectorSnapshotSummary> {
        self.summary.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorGenerationState {
    Missing,
    Ready,
    Incompatible,
    Corrupt,
    Unreadable,
    Invalid,
}
