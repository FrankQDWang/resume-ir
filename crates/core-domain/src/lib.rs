use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id(prefix: &str) -> String {
    let value = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}{value:016x}")
}

macro_rules! id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        #[allow(clippy::new_without_default)]
        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(next_id($prefix))
            }

            #[must_use]
            pub fn from_raw(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

id_type!(DocumentId, "doc_");
id_type!(ResumeVersionId, "ver_");
id_type!(CandidateId, "cand_");
id_type!(SectionId, "sec_");
id_type!(EntityMentionId, "mention_");
id_type!(VectorRecordId, "vec_");

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentExtension {
    Docx,
    Pdf,
    Doc,
    Txt,
    Image,
    Other(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    pub doc_id: DocumentId,
    pub source_uri: String,
    pub normalized_path: String,
    pub file_name: String,
    pub extension: DocumentExtension,
    pub byte_size: u64,
    pub mtime: SystemTime,
    pub content_hash: Option<String>,
    pub text_hash: Option<String>,
    pub is_deleted: bool,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Visibility {
    Searchable,
    Partial,
    Hidden,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResumeVersion {
    pub version_id: ResumeVersionId,
    pub doc_id: DocumentId,
    pub candidate_id: Option<CandidateId>,
    pub parse_version: String,
    pub schema_version: String,
    pub language_set: Vec<String>,
    pub page_count: Option<u32>,
    pub raw_text: Option<String>,
    pub clean_text: Option<String>,
    pub quality_score: Option<f32>,
    pub visibility: Visibility,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub candidate_id: CandidateId,
    pub primary_name: Option<String>,
    pub phone_hash: Option<String>,
    pub email_hash: Option<String>,
    pub dedupe_key: Option<String>,
    pub merge_confidence: Option<f32>,
    pub version_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SectionType {
    Profile,
    Contact,
    Education,
    Experience,
    Project,
    Skill,
    Certificate,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub section_id: SectionId,
    pub version_id: ResumeVersionId,
    pub section_type: SectionType,
    pub order_no: u32,
    pub page_no: Option<u32>,
    pub text: String,
    pub char_start: Option<usize>,
    pub char_end: Option<usize>,
    pub confidence: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntityType {
    Name,
    Email,
    Phone,
    School,
    Company,
    Title,
    Skill,
    Certificate,
    Date,
    Location,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityMention {
    pub mention_id: EntityMentionId,
    pub version_id: ResumeVersionId,
    pub section_id: Option<SectionId>,
    pub entity_type: EntityType,
    pub raw_value: String,
    pub normalized_value: Option<String>,
    pub span_start: Option<usize>,
    pub span_end: Option<usize>,
    pub confidence: f32,
    pub extractor: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorScope {
    Document,
    Section,
    Skill,
    Experience,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Quantization {
    Fp32,
    Fp16,
    Int8,
    Pq,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorRecord {
    pub vector_id: VectorRecordId,
    pub version_id: ResumeVersionId,
    pub section_id: Option<SectionId>,
    pub vector_scope: VectorScope,
    pub model_id: String,
    pub dim: u32,
    pub quantization: Quantization,
    pub created_at: SystemTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    ConfigError,
    IoError,
    PermissionDenied,
    UnsupportedFormat,
    EncryptedDocument,
    CorruptedDocument,
    ParserTimeout,
    OcrTimeout,
    ModelError,
    IndexCorrupted,
    SchemaMismatch,
    ResourceExhausted,
    Cancelled,
    InternalBug,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RedactionLevel {
    Safe,
    Sensitive,
    Confidential,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceComponent {
    Config,
    FsCrawler,
    Parser,
    Ocr,
    Model,
    MetaStore,
    Index,
    Search,
    Daemon,
    Cli,
    Privacy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppError {
    kind: ErrorKind,
    retryable: bool,
    user_message: String,
    diagnostic_message: String,
    redaction_level: RedactionLevel,
    source_component: SourceComponent,
}

impl AppError {
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

    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    #[must_use]
    pub fn retryable(&self) -> bool {
        self.retryable
    }

    #[must_use]
    pub fn user_message(&self) -> &str {
        &self.user_message
    }

    #[must_use]
    pub fn diagnostic_message_for_logs(&self) -> &str {
        match self.redaction_level {
            RedactionLevel::Safe => &self.diagnostic_message,
            RedactionLevel::Sensitive | RedactionLevel::Confidential => "[redacted]",
        }
    }

    #[must_use]
    pub fn redaction_level(&self) -> RedactionLevel {
        self.redaction_level
    }

    #[must_use]
    pub fn source_component(&self) -> SourceComponent {
        self.source_component
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "core-domain"
}
