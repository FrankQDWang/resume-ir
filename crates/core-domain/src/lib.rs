//! Core domain types for the local-first resume search kernel.

use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[doc = concat!("Typed identifier for ", stringify!($name), ".")]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Deserialize, Serialize)]
        pub struct $name(String);

        impl $name {
            /// Generates a new local identifier with a type-specific prefix.
            #[must_use]
            pub fn new() -> Self {
                Self(format!("{}_{}", $prefix, Uuid::new_v4().simple()))
            }

            /// Returns the identifier as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

id_type!(DocumentId, "doc");
id_type!(ResumeVersionId, "ver");
id_type!(CandidateId, "cand");
id_type!(SectionId, "sec");
id_type!(EntityMentionId, "ment");
id_type!(VectorId, "vec");

/// Local document extension known by the ingestion path.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum DocumentExtension {
    /// Microsoft Word `.docx`.
    Docx,
    /// Portable Document Format.
    Pdf,
    /// Legacy Word `.doc`.
    Doc,
    /// Plain text.
    Txt,
    /// Image or scanned source that may require OCR.
    Image,
    /// Unsupported extension retained for diagnostics.
    Other(String),
}

/// A local source file discovered by the kernel.
#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct Document {
    /// Stable document identifier.
    pub doc_id: DocumentId,
    /// Local path or logical URI. Do not upload this value.
    pub source_uri: String,
    /// Normalized local path for dedupe and search.
    pub normalized_path: String,
    /// File name only.
    pub file_name: String,
    /// Document extension.
    pub extension: DocumentExtension,
    /// File size in bytes.
    pub byte_size: u64,
    /// Last modified timestamp as an implementation-neutral string.
    pub mtime: String,
    /// Optional original content hash.
    pub content_hash: Option<String>,
    /// Optional normalized text hash.
    pub text_hash: Option<String>,
    /// Whether the source is deleted or unreachable.
    pub is_deleted: bool,
    /// First discovery timestamp.
    pub created_at: String,
    /// Last update timestamp.
    pub updated_at: String,
}

impl fmt::Debug for Document {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Document")
            .field("doc_id", &self.doc_id)
            .field("source_uri", &"[redacted local path]")
            .field("normalized_path", &"[redacted local path]")
            .field("file_name", &"[redacted file name]")
            .field("extension", &self.extension)
            .field("byte_size", &self.byte_size)
            .field("mtime", &self.mtime)
            .field("content_hash_present", &self.content_hash.is_some())
            .field("text_hash_present", &self.text_hash.is_some())
            .field("is_deleted", &self.is_deleted)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Visibility state for parsed resume versions.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ResumeVisibility {
    /// Available for normal search.
    Searchable,
    /// Partially parsed and searchable with caveats.
    Partial,
    /// Hidden from search.
    Hidden,
}

/// Parsed version of a resume.
#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct ResumeVersion {
    /// Stable version identifier.
    pub version_id: ResumeVersionId,
    /// Source document identifier.
    pub doc_id: DocumentId,
    /// Optional candidate aggregation identifier.
    pub candidate_id: Option<CandidateId>,
    /// Parser version that produced this record.
    pub parse_version: String,
    /// Schema version for structured fields.
    pub schema_version: String,
    /// Detected language tags.
    pub language_set: Vec<String>,
    /// Optional page count.
    pub page_count: Option<u32>,
    /// Raw extracted text. Keep local.
    pub raw_text: Option<String>,
    /// Cleaned text. Keep local.
    pub clean_text: Option<String>,
    /// Optional parse quality score.
    pub quality_score: Option<f32>,
    /// Search visibility.
    pub visibility: ResumeVisibility,
}

impl fmt::Debug for ResumeVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResumeVersion")
            .field("version_id", &self.version_id)
            .field("doc_id", &self.doc_id)
            .field("candidate_id", &self.candidate_id)
            .field("parse_version", &self.parse_version)
            .field("schema_version", &self.schema_version)
            .field("language_set", &self.language_set)
            .field("page_count", &self.page_count)
            .field(
                "raw_text",
                &redacted_option(self.raw_text.as_ref(), "[redacted raw text]"),
            )
            .field(
                "clean_text",
                &redacted_option(self.clean_text.as_ref(), "[redacted clean text]"),
            )
            .field("quality_score", &self.quality_score)
            .field("visibility", &self.visibility)
            .finish()
    }
}

/// Soft aggregation of one candidate's resume versions.
#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct Candidate {
    /// Candidate identifier.
    pub candidate_id: CandidateId,
    /// Optional display name.
    pub primary_name: Option<String>,
    /// Hash of phone value, never raw phone.
    pub phone_hash: Option<String>,
    /// Hash of email value, never raw email.
    pub email_hash: Option<String>,
    /// Soft dedupe key.
    pub dedupe_key: Option<String>,
    /// Confidence for this aggregation.
    pub merge_confidence: Option<f32>,
    /// Number of associated versions.
    pub version_count: u32,
}

impl fmt::Debug for Candidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Candidate")
            .field("candidate_id", &self.candidate_id)
            .field(
                "primary_name",
                &redacted_option(self.primary_name.as_ref(), "[redacted candidate name]"),
            )
            .field("phone_hash_present", &self.phone_hash.is_some())
            .field("email_hash_present", &self.email_hash.is_some())
            .field("dedupe_key_present", &self.dedupe_key.is_some())
            .field("merge_confidence", &self.merge_confidence)
            .field("version_count", &self.version_count)
            .finish()
    }
}

/// Resume section type.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum SectionType {
    /// Profile or summary.
    Profile,
    /// Contact details.
    Contact,
    /// Education history.
    Education,
    /// Work experience.
    Experience,
    /// Project experience.
    Project,
    /// Skills.
    Skill,
    /// Certificates.
    Certificate,
    /// Unclassified section.
    Other,
}

/// A text section within a parsed resume.
#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct Section {
    /// Section identifier.
    pub section_id: SectionId,
    /// Owning resume version.
    pub version_id: ResumeVersionId,
    /// Semantic section type.
    pub section_type: SectionType,
    /// Source order.
    pub order_no: u32,
    /// Optional page number.
    pub page_no: Option<u32>,
    /// Section text. Keep local.
    pub text: String,
    /// Optional start offset in clean text.
    pub char_start: Option<u32>,
    /// Optional end offset in clean text.
    pub char_end: Option<u32>,
    /// Section confidence.
    pub confidence: f32,
}

impl fmt::Debug for Section {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Section")
            .field("section_id", &self.section_id)
            .field("version_id", &self.version_id)
            .field("section_type", &self.section_type)
            .field("order_no", &self.order_no)
            .field("page_no", &self.page_no)
            .field("text", &"[redacted section text]")
            .field("char_start", &self.char_start)
            .field("char_end", &self.char_end)
            .field("confidence", &self.confidence)
            .finish()
    }
}

/// Extracted entity type.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum EntityType {
    /// Person name.
    Name,
    /// Email address.
    Email,
    /// Phone number.
    Phone,
    /// School.
    School,
    /// Company.
    Company,
    /// Job title.
    Title,
    /// Skill.
    Skill,
    /// Certificate.
    Certificate,
    /// Date or date range.
    Date,
    /// Location.
    Location,
    /// Other entity.
    Other(String),
}

/// Evidence-preserving extracted entity mention.
#[derive(Clone, PartialEq, Deserialize, Serialize)]
pub struct EntityMention {
    /// Mention identifier.
    pub mention_id: EntityMentionId,
    /// Owning resume version.
    pub version_id: ResumeVersionId,
    /// Optional section identifier.
    pub section_id: Option<SectionId>,
    /// Entity type.
    pub entity_type: EntityType,
    /// Raw extracted value. Keep local.
    pub raw_value: String,
    /// Optional normalized value.
    pub normalized_value: Option<String>,
    /// Optional start offset.
    pub span_start: Option<u32>,
    /// Optional end offset.
    pub span_end: Option<u32>,
    /// Extraction confidence.
    pub confidence: f32,
    /// Extractor name.
    pub extractor: String,
}

impl fmt::Debug for EntityMention {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntityMention")
            .field("mention_id", &self.mention_id)
            .field("version_id", &self.version_id)
            .field("section_id", &self.section_id)
            .field("entity_type", &self.entity_type)
            .field("raw_value", &"[redacted entity value]")
            .field(
                "normalized_value",
                &redacted_option(
                    self.normalized_value.as_ref(),
                    "[redacted normalized entity value]",
                ),
            )
            .field("span_start", &self.span_start)
            .field("span_end", &self.span_end)
            .field("confidence", &self.confidence)
            .field("extractor", &self.extractor)
            .finish()
    }
}

/// Scope of an embedding vector.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum VectorScope {
    /// Whole document vector.
    Document,
    /// Section vector.
    Section,
    /// Skill-focused vector.
    Skill,
    /// Experience-focused vector.
    Experience,
}

/// Vector quantization strategy.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum Quantization {
    /// 32-bit floating point.
    Fp32,
    /// 16-bit floating point.
    Fp16,
    /// 8-bit integer.
    Int8,
    /// Product quantization.
    Pq,
}

/// Metadata for a local embedding vector.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct VectorRecord {
    /// Vector identifier.
    pub vector_id: VectorId,
    /// Owning resume version.
    pub version_id: ResumeVersionId,
    /// Optional section identifier.
    pub section_id: Option<SectionId>,
    /// Vector scope.
    pub vector_scope: VectorScope,
    /// Embedding model identifier.
    pub model_id: String,
    /// Vector dimension.
    pub dim: u32,
    /// Quantization.
    pub quantization: Quantization,
    /// Creation timestamp.
    pub created_at: String,
}

/// Stable error categories used across crates.
#[derive(Clone, Debug, Eq, Error, PartialEq, Deserialize, Serialize)]
pub enum ErrorKind {
    /// Local file permission problem.
    #[error("permission denied")]
    PermissionDenied,
    /// Source document is encrypted.
    #[error("encrypted document")]
    EncryptedDocument,
    /// Source document is corrupted.
    #[error("corrupted document")]
    CorruptedDocument,
    /// Unsupported document type.
    #[error("unsupported document")]
    UnsupportedDocument,
    /// Local configuration is invalid.
    #[error("invalid configuration")]
    InvalidConfiguration,
    /// Local storage error.
    #[error("storage error")]
    Storage,
    /// Operation timed out.
    #[error("timeout")]
    Timeout,
    /// Internal invariant violation.
    #[error("internal error")]
    Internal,
}

/// Redaction level required before exposing an error outside diagnostics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum RedactionLevel {
    /// Message is safe for normal user display.
    Safe,
    /// Message may contain local paths or operational detail.
    LocalDiagnostic,
    /// Message may contain sensitive resume content or PII.
    Sensitive,
}

/// Component that produced an error.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum SourceComponent {
    /// Core domain crate.
    CoreDomain,
    /// Config crate.
    Config,
    /// Metadata store crate.
    MetaStore,
    /// Daemon crate.
    Daemon,
    /// CLI crate.
    Cli,
}

/// Unified error structure for user-facing and diagnostic reporting.
#[derive(Clone, Eq, Error, PartialEq, Deserialize, Serialize)]
#[error("{kind}: {user_message}")]
pub struct ResumeError {
    kind: ErrorKind,
    retryable: bool,
    user_message: String,
    diagnostic_message: String,
    redaction_level: RedactionLevel,
    source_component: SourceComponent,
}

impl fmt::Debug for ResumeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResumeError")
            .field("kind", &self.kind)
            .field("retryable", &self.retryable)
            .field("user_message", &self.user_message)
            .field("diagnostic_message", &self.redacted_diagnostic_message())
            .field("redaction_level", &self.redaction_level)
            .field("source_component", &self.source_component)
            .finish()
    }
}

impl ResumeError {
    /// Creates a new structured error.
    #[must_use]
    pub fn new(
        kind: ErrorKind,
        retryable: bool,
        user_message: impl Into<String>,
        diagnostic_message: impl Into<String>,
        redaction_level: RedactionLevel,
        source_component: SourceComponent,
    ) -> Self {
        Self {
            kind,
            retryable,
            user_message: user_message.into(),
            diagnostic_message: diagnostic_message.into(),
            redaction_level,
            source_component,
        }
    }

    /// Returns the stable error kind.
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind.clone()
    }

    /// Returns whether the operation may be retried.
    #[must_use]
    pub fn retryable(&self) -> bool {
        self.retryable
    }

    /// Returns the message safe for user display.
    #[must_use]
    pub fn user_message(&self) -> &str {
        &self.user_message
    }

    /// Returns a diagnostic message safe for non-local display.
    #[must_use]
    pub fn redacted_diagnostic_message(&self) -> &str {
        match self.redaction_level {
            RedactionLevel::Safe => &self.diagnostic_message,
            RedactionLevel::LocalDiagnostic => "[redacted local diagnostic]",
            RedactionLevel::Sensitive => "[redacted sensitive diagnostic]",
        }
    }

    /// Returns the raw local diagnostic message.
    ///
    /// This may contain local paths or sensitive resume text. Use only in local,
    /// explicitly protected diagnostics.
    #[must_use]
    pub fn local_diagnostic_message(&self) -> &str {
        &self.diagnostic_message
    }

    /// Returns the redaction level.
    #[must_use]
    pub fn redaction_level(&self) -> RedactionLevel {
        self.redaction_level
    }

    /// Returns the component that produced the error.
    #[must_use]
    pub fn source_component(&self) -> SourceComponent {
        self.source_component
    }
}

/// Domain result type.
pub type Result<T> = std::result::Result<T, ResumeError>;

fn redacted_option<T>(value: Option<&T>, redacted: &'static str) -> Option<&'static str> {
    value.map(|_| redacted)
}
