mod model;
mod persistence;
mod retention;
mod store;
mod validation;

pub use model::{
    EnabledVectorSnapshotDescriptor, FullTextSnapshotDescriptor, ProjectedDocumentSnapshot,
    SearchPublicationCommit, SearchPublicationDraft, SearchPublicationFailure,
    SearchPublicationOutcome, SearchPublicationPrunePolicy, SearchPublicationRecord,
    SearchPublicationState, SearchPublicationValidation, TerminalDocumentUpdate,
    VectorSnapshotDescriptor, VectorSnapshotMode, FULLTEXT_INDEX_SCHEMA_V2,
    FULLTEXT_MANIFEST_SCHEMA_V2, VECTOR_INDEX_SCHEMA_V3, VECTOR_MANIFEST_SCHEMA_V3,
};
pub use validation::search_publication_fingerprint;

pub(super) use persistence::search_publication_in_connection;
