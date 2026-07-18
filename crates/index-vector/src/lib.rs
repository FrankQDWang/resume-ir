//! Immutable, generation-addressed vector snapshots.
//!
//! The metadata store owns which generation is active. This crate never
//! chooses a "latest" generation and never falls back to an older snapshot.

mod ann;
mod codec;
mod model;
mod model_contract;
mod private_storage;
mod snapshot_gc;
mod snapshot_gc_candidates;
mod snapshot_identity;
mod snapshot_model;
mod snapshot_root;
mod store;

pub use model::{QueryVector, VectorDocument, VectorDocumentIdentity, VectorHit, VectorIndexError};
pub use model_contract::{VectorModelContract, MAX_VECTOR_DIMENSION};
pub use snapshot_gc::{
    commit_snapshot_gc, PreparedVectorSnapshotGc, VectorGcPartialFailure, VectorGcSummary,
    VectorSnapshotGcAcquisition, VectorSnapshotGcCommitReport, VectorSnapshotGcFailureClass,
    VectorSnapshotGcFailurePhase, VectorSnapshotGcPreparation,
};
pub use snapshot_model::{
    VectorSnapshotSchema, VectorSnapshotSummary, VectorSnapshotUpdate, VECTOR_SNAPSHOT_SCHEMA_V3,
};
pub use snapshot_root::{
    VectorGenerationInspection, VectorGenerationState, VectorSnapshotReadLease,
    VectorSnapshotReader, VectorSnapshotRoot,
};
pub use store::VectorSnapshotStore;

pub fn crate_name() -> &'static str {
    "index-vector"
}
