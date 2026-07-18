use std::fmt;

use crate::{
    ActiveSearchProjection, ContentDigest, DocumentId, DocumentStatus, SearchProjectionDigest,
    UnixTimestamp,
};

pub const FULLTEXT_MANIFEST_SCHEMA_V2: &str = "fulltext.snapshot.v2";
pub const FULLTEXT_INDEX_SCHEMA_V2: &str = "tantivy.fulltext.v2";
pub const VECTOR_MANIFEST_SCHEMA_V3: &str = "vector.snapshot.v3";
pub const VECTOR_INDEX_SCHEMA_V3: &str = "hnsw-vector.v3";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPublicationState {
    Preparing,
    Validated,
    Ready,
    Abandoned,
}

impl SearchPublicationState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Validated => "validated",
            Self::Ready => "ready",
            Self::Abandoned => "abandoned",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPublicationOutcome {
    Applied,
    Superseded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchPublicationFailure {
    InvalidGeneration,
    InvalidClassifierEpoch,
    InvalidDescriptor,
    DescriptorMismatch,
    InvalidState,
    InvalidPersistedState,
    ProjectionMismatch,
    InvalidProjectionTransition,
    VectorCoverageMismatch,
    ExactClassificationMissing,
    InvalidDocumentState,
}

#[derive(Clone, PartialEq, Eq)]
pub struct FullTextSnapshotDescriptor {
    generation: String,
    document_count: u64,
    projection_digest: SearchProjectionDigest,
    logical_content_digest: ContentDigest,
}

impl FullTextSnapshotDescriptor {
    pub fn new(
        generation: String,
        document_count: u64,
        projection_digest: SearchProjectionDigest,
        logical_content_digest: ContentDigest,
    ) -> Self {
        Self {
            generation,
            document_count,
            projection_digest,
            logical_content_digest,
        }
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn manifest_schema(&self) -> &'static str {
        FULLTEXT_MANIFEST_SCHEMA_V2
    }

    pub fn index_schema(&self) -> &'static str {
        FULLTEXT_INDEX_SCHEMA_V2
    }

    pub fn document_count(&self) -> u64 {
        self.document_count
    }

    pub fn projection_digest(&self) -> &SearchProjectionDigest {
        &self.projection_digest
    }

    pub fn logical_content_digest(&self) -> &ContentDigest {
        &self.logical_content_digest
    }
}

impl fmt::Debug for FullTextSnapshotDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FullTextSnapshotDescriptor")
            .field("generation", &"<redacted>")
            .field("document_count", &self.document_count)
            .field("projection_digest", &self.projection_digest)
            .field("logical_content_digest", &self.logical_content_digest)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum VectorSnapshotMode {
    Disabled,
    Enabled { model_id: String, dimension: u32 },
}

impl fmt::Debug for VectorSnapshotMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("Disabled"),
            Self::Enabled { dimension, .. } => formatter
                .debug_struct("Enabled")
                .field("model_id", &"<redacted>")
                .field("dimension", dimension)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct VectorSnapshotDescriptor {
    generation: String,
    mode: VectorSnapshotMode,
    projection_count: u64,
    projection_digest: SearchProjectionDigest,
    coverage_digest: SearchProjectionDigest,
    vector_count: u64,
    document_count: u64,
    resume_version_count: u64,
    logical_content_digest: ContentDigest,
}

/// Named construction contract for an enabled immutable vector snapshot.
/// The publication validator checks the cross-field invariants before this
/// descriptor can become authoritative.
pub struct EnabledVectorSnapshotDescriptor {
    pub generation: String,
    pub model_id: String,
    pub dimension: u32,
    pub projection_count: u64,
    pub projection_digest: SearchProjectionDigest,
    pub coverage_digest: SearchProjectionDigest,
    pub vector_count: u64,
    pub document_count: u64,
    pub resume_version_count: u64,
    pub logical_content_digest: ContentDigest,
}

impl VectorSnapshotDescriptor {
    pub fn disabled(
        generation: String,
        projection_count: u64,
        projection_digest: SearchProjectionDigest,
        coverage_digest: SearchProjectionDigest,
        logical_content_digest: ContentDigest,
    ) -> Self {
        Self {
            generation,
            mode: VectorSnapshotMode::Disabled,
            projection_count,
            projection_digest,
            coverage_digest,
            vector_count: 0,
            document_count: 0,
            resume_version_count: 0,
            logical_content_digest,
        }
    }

    pub fn enabled(descriptor: EnabledVectorSnapshotDescriptor) -> Self {
        Self {
            generation: descriptor.generation,
            mode: VectorSnapshotMode::Enabled {
                model_id: descriptor.model_id,
                dimension: descriptor.dimension,
            },
            projection_count: descriptor.projection_count,
            projection_digest: descriptor.projection_digest,
            coverage_digest: descriptor.coverage_digest,
            vector_count: descriptor.vector_count,
            document_count: descriptor.document_count,
            resume_version_count: descriptor.resume_version_count,
            logical_content_digest: descriptor.logical_content_digest,
        }
    }

    pub fn generation(&self) -> &str {
        &self.generation
    }

    pub fn manifest_schema(&self) -> &'static str {
        VECTOR_MANIFEST_SCHEMA_V3
    }

    pub fn index_schema(&self) -> &'static str {
        VECTOR_INDEX_SCHEMA_V3
    }

    pub fn mode(&self) -> &VectorSnapshotMode {
        &self.mode
    }

    pub fn projection_count(&self) -> u64 {
        self.projection_count
    }

    pub fn projection_digest(&self) -> &SearchProjectionDigest {
        &self.projection_digest
    }

    pub fn coverage_digest(&self) -> &SearchProjectionDigest {
        &self.coverage_digest
    }

    pub fn vector_count(&self) -> u64 {
        self.vector_count
    }

    pub fn document_count(&self) -> u64 {
        self.document_count
    }

    pub fn resume_version_count(&self) -> u64 {
        self.resume_version_count
    }

    pub fn logical_content_digest(&self) -> &ContentDigest {
        &self.logical_content_digest
    }
}

impl fmt::Debug for VectorSnapshotDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorSnapshotDescriptor")
            .field("generation", &"<redacted>")
            .field("mode", &self.mode)
            .field("projection_count", &self.projection_count)
            .field("projection_digest", &self.projection_digest)
            .field("coverage_digest", &self.coverage_digest)
            .field("vector_count", &self.vector_count)
            .field("document_count", &self.document_count)
            .field("resume_version_count", &self.resume_version_count)
            .field("logical_content_digest", &self.logical_content_digest)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SearchPublicationDraft {
    pub generation: String,
    pub base_generation: Option<String>,
    pub expected_visible_epoch: u64,
    pub classifier_epoch: String,
    pub projection_digest: SearchProjectionDigest,
    pub now: UnixTimestamp,
}

impl fmt::Debug for SearchPublicationDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchPublicationDraft")
            .field("generation", &"<redacted>")
            .field(
                "base_generation",
                &self.base_generation.as_ref().map(|_| "<redacted>"),
            )
            .field("expected_visible_epoch", &self.expected_visible_epoch)
            .field("classifier_epoch", &"<redacted>")
            .field("projection_digest", &self.projection_digest)
            .field("now", &self.now)
            .finish()
    }
}

pub struct SearchPublicationValidation<'a> {
    pub generation: &'a str,
    pub fulltext: &'a FullTextSnapshotDescriptor,
    pub vector: &'a VectorSnapshotDescriptor,
    pub now: UnixTimestamp,
}

pub struct SearchPublicationCommit<'a> {
    pub generation: &'a str,
    pub terminal_documents: &'a [TerminalDocumentUpdate],
    pub projections: &'a [ActiveSearchProjection],
    pub vector_coverage: &'a [ActiveSearchProjection],
    pub now: UnixTimestamp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalDocumentUpdate {
    pub document_id: DocumentId,
    pub expected_status: DocumentStatus,
    pub expected_is_deleted: bool,
    pub expected_content_hash: ContentDigest,
    pub terminal_status: DocumentStatus,
    pub terminal_is_deleted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchPublicationPrunePolicy {
    pub retain_ready: usize,
    pub abandoned_updated_before: UnixTimestamp,
    pub max_delete: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SearchPublicationRecord {
    pub generation: String,
    pub base_generation: Option<String>,
    pub expected_visible_epoch: u64,
    pub classifier_epoch: String,
    pub projection_digest: SearchProjectionDigest,
    pub publication_fingerprint: Option<ContentDigest>,
    pub state: SearchPublicationState,
    pub fulltext: Option<FullTextSnapshotDescriptor>,
    pub vector: Option<VectorSnapshotDescriptor>,
    pub created_at: UnixTimestamp,
    pub updated_at: UnixTimestamp,
}

impl fmt::Debug for SearchPublicationRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchPublicationRecord")
            .field("generation", &"<redacted>")
            .field(
                "base_generation",
                &self.base_generation.as_ref().map(|_| "<redacted>"),
            )
            .field("expected_visible_epoch", &self.expected_visible_epoch)
            .field("classifier_epoch", &"<redacted>")
            .field("projection_digest", &self.projection_digest)
            .field("publication_fingerprint", &self.publication_fingerprint)
            .field("state", &self.state)
            .field("fulltext", &self.fulltext)
            .field("vector", &self.vector)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}
