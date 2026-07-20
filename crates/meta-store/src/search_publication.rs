mod authority;
mod model;
mod persistence;
mod retention;
mod retirement;
mod retirement_settlement;
mod store;
mod validation;

pub use model::{
    EnabledVectorSnapshotDescriptor, FullTextSnapshotDescriptor, ProjectedDocumentSnapshot,
    SearchPublicationCommit, SearchPublicationDraft, SearchPublicationFailure,
    SearchPublicationOutcome, SearchPublicationPrunePolicy, SearchPublicationRecord,
    SearchPublicationRetirementFailureOutcome, SearchPublicationState, SearchPublicationValidation,
    TerminalDocumentUpdate, VectorSnapshotDescriptor, VectorSnapshotMode, FULLTEXT_INDEX_SCHEMA_V3,
    FULLTEXT_MANIFEST_SCHEMA_V3, VECTOR_INDEX_SCHEMA_V4, VECTOR_MANIFEST_SCHEMA_V4,
};
pub use retirement::{
    SearchArtifactExpectation, SearchPublicationRetirement, SearchPublicationRetirementArtifact,
    SearchPublicationRetirementPhase, SearchPublicationRetirementPlan,
    SEARCH_PUBLICATION_RETIREMENT_REPLAY_LIMIT,
};
pub use validation::search_publication_fingerprint;

pub(crate) use authority::validate_persisted_authority;
pub(super) use persistence::search_publication_in_connection;
pub(crate) use retirement::ensure_no_pending_retirement;
