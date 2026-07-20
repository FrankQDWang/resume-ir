//! Immutable, generation-addressed vector snapshots.
//!
//! The metadata store owns which generation is active. This crate never
//! chooses a "latest" generation and never falls back to an older snapshot.

mod ann;
mod codec;
mod manifest_inspection;
mod model;
mod model_contract;
mod private_storage;
mod publish_control;
mod purge_artifact;
mod snapshot_gc;
mod snapshot_gc_candidates;
mod snapshot_identity;
mod snapshot_model;
mod snapshot_root;
mod store;

pub use model::{QueryVector, VectorDocument, VectorDocumentIdentity, VectorHit, VectorIndexError};
pub use model_contract::{VectorModelContract, MAX_VECTOR_DIMENSION};
pub use publish_control::VectorSnapshotPublishControl;
pub use purge_artifact::{classify_purge_artifact, VectorPurgeArtifactClass};
pub use snapshot_gc::{
    commit_snapshot_gc, commit_snapshot_gc_with_cancel_check, try_retire_unpublished_generation,
    PreparedVectorSnapshotGc, VectorGcPartialFailure, VectorGcSummary, VectorGenerationRetirement,
    VectorGenerationRetirementSummary, VectorSnapshotGcAcquisition, VectorSnapshotGcCommitReport,
    VectorSnapshotGcFailureClass, VectorSnapshotGcFailurePhase, VectorSnapshotGcPreparation,
};
pub use snapshot_model::{
    VectorSnapshotManifestMetadata, VectorSnapshotSchema, VectorSnapshotSummary,
    VectorSnapshotUpdate, VECTOR_SNAPSHOT_SCHEMA_V4,
};
pub use snapshot_root::{
    VectorGenerationInspection, VectorGenerationState, VectorSnapshotReadLease,
    VectorSnapshotReader, VectorSnapshotRoot,
};
pub use store::VectorSnapshotStore;

pub fn crate_name() -> &'static str {
    "index-vector"
}
