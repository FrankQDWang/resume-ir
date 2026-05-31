use std::{fmt, str::FromStr};

const FNV_OFFSET_A: u64 = 0xcbf29ce484222325;
const FNV_OFFSET_B: u64 = 0x6c62272e07bb0142;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const ID_DIGEST_HEX_LEN: usize = 32;
const CONTACT_HASH_HEX_LEN: usize = 64;

pub fn crate_name() -> &'static str {
    "core-domain"
}

macro_rules! stable_id_type {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Creates an opaque stable ID from non-secret components only.
            ///
            /// This is not a privacy hash and must not be used for phone,
            /// email, resume text, or any other sensitive value. Contact
            /// dedupe keys must be produced by an external keyed hash and
            /// hydrated with `ContactHash`.
            pub fn from_non_secret_parts(parts: &[&str]) -> Self {
                Self(format!(
                    "{}{}",
                    $prefix,
                    stable_id_digest(stringify!($name), parts)
                ))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                validate_stable_id($prefix, value)?;

                Ok(Self(value.to_string()))
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdParseError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                validate_stable_id($prefix, &value)?;

                Ok(Self(value))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

stable_id_type!(DocumentId, "doc_");
stable_id_type!(ResumeVersionId, "ver_");
stable_id_type!(CandidateId, "cand_");
stable_id_type!(SectionId, "sec_");
stable_id_type!(EntityMentionId, "ent_");
stable_id_type!(VectorRecordId, "vec_");
stable_id_type!(IngestJobId, "job_");
stable_id_type!(ImportTaskId, "imp_");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdParseError {
    InvalidPrefix { expected: &'static str },
    InvalidLength { expected: usize, actual: usize },
    InvalidHexDigest,
}

impl fmt::Display for IdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdParseError::InvalidPrefix { expected } => {
                write!(formatter, "ID must start with prefix `{expected}`")
            }
            IdParseError::InvalidLength { expected, actual } => {
                write!(formatter, "ID length must be {expected}, got {actual}")
            }
            IdParseError::InvalidHexDigest => formatter.write_str("ID digest must be hex"),
        }
    }
}

impl std::error::Error for IdParseError {}

fn validate_stable_id(expected_prefix: &'static str, value: &str) -> Result<(), IdParseError> {
    if !value.starts_with(expected_prefix) {
        return Err(IdParseError::InvalidPrefix {
            expected: expected_prefix,
        });
    }

    let expected_len = expected_prefix.len() + ID_DIGEST_HEX_LEN;
    if value.len() != expected_len {
        return Err(IdParseError::InvalidLength {
            expected: expected_len,
            actual: value.len(),
        });
    }

    let digest = &value[expected_prefix.len()..];
    if !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(IdParseError::InvalidHexDigest);
    }

    Ok(())
}

fn stable_id_digest(namespace: &str, parts: &[&str]) -> String {
    let hash_a = stable_hash(FNV_OFFSET_A, namespace, parts);
    let hash_b = stable_hash(FNV_OFFSET_B, namespace, parts);

    format!("{hash_a:016x}{hash_b:016x}")
}

fn stable_hash(seed: u64, namespace: &str, parts: &[&str]) -> u64 {
    let mut hash = seed;
    update_hash(&mut hash, namespace.as_bytes());
    update_hash(&mut hash, &(parts.len() as u64).to_le_bytes());

    for part in parts {
        update_hash(&mut hash, &(part.len() as u64).to_le_bytes());
        update_hash(&mut hash, part.as_bytes());
    }

    hash
}

fn update_hash(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(FNV_PRIME);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnixTimestamp {
    seconds: i64,
}

impl UnixTimestamp {
    pub fn from_unix_seconds(seconds: i64) -> Self {
        Self { seconds }
    }

    pub fn as_unix_seconds(self) -> i64 {
        self.seconds
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContactHash(String);

impl ContactHash {
    /// Hydrates a contact hash produced by an external keyed digest.
    ///
    /// S2 intentionally does not provide phone/email hashing. Callers must
    /// supply a keyed digest from the privacy boundary responsible for PII.
    pub fn from_keyed_digest(digest: impl Into<String>) -> Result<Self, ContactHashParseError> {
        let digest = digest.into();
        validate_contact_hash(&digest)?;

        Ok(Self(digest))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ContactHash {
    type Err = ContactHashParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::from_keyed_digest(value)
    }
}

impl TryFrom<String> for ContactHash {
    type Error = ContactHashParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_keyed_digest(value)
    }
}

impl fmt::Display for ContactHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

impl fmt::Debug for ContactHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ContactHash")
            .field(&"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContactHashParseError {
    InvalidLength { expected: usize, actual: usize },
    InvalidHexDigest,
}

impl fmt::Display for ContactHashParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContactHashParseError::InvalidLength { expected, actual } => write!(
                formatter,
                "contact hash length must be {expected}, got {actual}"
            ),
            ContactHashParseError::InvalidHexDigest => {
                formatter.write_str("contact hash must be hex")
            }
        }
    }
}

impl std::error::Error for ContactHashParseError {}

fn validate_contact_hash(digest: &str) -> Result<(), ContactHashParseError> {
    if digest.len() != CONTACT_HASH_HEX_LEN {
        return Err(ContactHashParseError::InvalidLength {
            expected: CONTACT_HASH_HEX_LEN,
            actual: digest.len(),
        });
    }

    if !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ContactHashParseError::InvalidHexDigest);
    }

    Ok(())
}

#[derive(Clone, PartialEq)]
pub struct Candidate {
    pub id: CandidateId,
    pub primary_name: Option<String>,
    pub phone_hash: Option<ContactHash>,
    pub email_hash: Option<ContactHash>,
    pub dedupe_key: Option<String>,
    pub merge_confidence: Option<f32>,
    pub version_count: u32,
}

impl fmt::Debug for Candidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Candidate")
            .field("id", &self.id)
            .field(
                "primary_name",
                &self.primary_name.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "phone_hash",
                &self.phone_hash.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "email_hash",
                &self.email_hash.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "dedupe_key",
                &self.dedupe_key.as_ref().map(|_| "<redacted>"),
            )
            .field("merge_confidence", &self.merge_confidence)
            .field("version_count", &self.version_count)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Document {
    pub id: DocumentId,
    pub source_uri: String,
    pub normalized_path: String,
    pub file_name: String,
    pub extension: FileExtension,
    pub byte_size: u64,
    pub mtime: UnixTimestamp,
    pub content_hash: Option<String>,
    pub text_hash: Option<String>,
    pub is_deleted: bool,
    pub created_at: UnixTimestamp,
    pub updated_at: UnixTimestamp,
    pub status: DocumentStatus,
}

impl fmt::Debug for Document {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Document")
            .field("id", &self.id)
            .field("source_uri", &"<redacted>")
            .field("normalized_path", &"<redacted>")
            .field("file_name", &"<redacted>")
            .field("extension", &self.extension)
            .field("byte_size", &self.byte_size)
            .field("mtime", &self.mtime)
            .field(
                "content_hash",
                &self.content_hash.as_ref().map(|_| "<redacted>"),
            )
            .field("text_hash", &self.text_hash.as_ref().map(|_| "<redacted>"))
            .field("is_deleted", &self.is_deleted)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("status", &self.status)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileExtension {
    Docx,
    Pdf,
    Doc,
    Txt,
    Image,
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentStatus {
    Discovered,
    Fingerprinted,
    ParseQueued,
    ParseRunning,
    TextExtracted,
    OcrRequired,
    OcrRunning,
    OcrDone,
    TextCleaned,
    FieldsExtracted,
    EmbeddingDone,
    IndexedPartial,
    Searchable,
    FailedRetryable,
    FailedPermanent,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngestJobKind {
    DiscoverDocument,
    FingerprintDocument,
    ParseDocument,
    CleanText,
    ExtractFields,
    UpdateIndex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IngestJobStatus {
    Queued,
    Running,
    Interrupted,
    Completed,
    FailedRetryable,
    FailedPermanent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexStateStatus {
    Empty,
    Building,
    Ready,
    Stale,
}

#[derive(Clone, PartialEq)]
pub struct ResumeVersion {
    pub id: ResumeVersionId,
    pub document_id: DocumentId,
    pub candidate_id: Option<CandidateId>,
    pub parse_version: String,
    pub schema_version: String,
    pub language_set: Vec<String>,
    pub page_count: Option<u32>,
    pub raw_text: Option<String>,
    pub clean_text: Option<String>,
    pub quality_score: Option<f32>,
    pub visibility: ResumeVisibility,
}

impl fmt::Debug for ResumeVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResumeVersion")
            .field("id", &self.id)
            .field("document_id", &self.document_id)
            .field("candidate_id", &self.candidate_id)
            .field("parse_version", &self.parse_version)
            .field("schema_version", &self.schema_version)
            .field("language_set", &self.language_set)
            .field("page_count", &self.page_count)
            .field("raw_text", &self.raw_text.as_ref().map(|_| "<redacted>"))
            .field(
                "clean_text",
                &self.clean_text.as_ref().map(|_| "<redacted>"),
            )
            .field("quality_score", &self.quality_score)
            .field("visibility", &self.visibility)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResumeVisibility {
    Searchable,
    Partial,
    Hidden,
}

#[derive(Clone, PartialEq)]
pub struct Section {
    pub id: SectionId,
    pub resume_version_id: ResumeVersionId,
    pub section_type: SectionType,
    pub order_no: u32,
    pub page_no: Option<u32>,
    pub text: String,
    pub char_start: Option<usize>,
    pub char_end: Option<usize>,
    pub confidence: f32,
}

impl fmt::Debug for Section {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Section")
            .field("id", &self.id)
            .field("resume_version_id", &self.resume_version_id)
            .field("section_type", &self.section_type)
            .field("order_no", &self.order_no)
            .field("page_no", &self.page_no)
            .field("text", &"<redacted>")
            .field("char_start", &self.char_start)
            .field("char_end", &self.char_end)
            .field("confidence", &self.confidence)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionType {
    Profile,
    Contact,
    Education,
    Experience,
    Project,
    Skill,
    Certificate,
    Other(String),
}

#[derive(Clone, PartialEq)]
pub struct EntityMention {
    pub id: EntityMentionId,
    pub resume_version_id: ResumeVersionId,
    pub section_id: Option<SectionId>,
    pub entity_type: EntityType,
    pub raw_value: String,
    pub normalized_value: Option<String>,
    pub span_start: Option<usize>,
    pub span_end: Option<usize>,
    pub confidence: f32,
    pub extractor: String,
}

impl fmt::Debug for EntityMention {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntityMention")
            .field("id", &self.id)
            .field("resume_version_id", &self.resume_version_id)
            .field("section_id", &self.section_id)
            .field("entity_type", &self.entity_type)
            .field("raw_value", &"<redacted>")
            .field(
                "normalized_value",
                &self.normalized_value.as_ref().map(|_| "<redacted>"),
            )
            .field("span_start", &self.span_start)
            .field("span_end", &self.span_end)
            .field("confidence", &self.confidence)
            .field("extractor", &self.extractor)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityType {
    Name,
    Email,
    Phone,
    School,
    Company,
    Title,
    Education,
    Skills,
    Skill,
    Certificate,
    Date,
    Location,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VectorRecord {
    pub id: VectorRecordId,
    pub resume_version_id: ResumeVersionId,
    pub section_id: Option<SectionId>,
    pub vector_scope: VectorScope,
    pub model_id: String,
    pub dim: usize,
    pub quantization: VectorQuantization,
    pub created_at: UnixTimestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorScope {
    Document,
    Section,
    Skill,
    Experience,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorQuantization {
    Fp32,
    Fp16,
    Int8,
    Pq,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedactionLevel {
    None,
    Sensitive,
    Secret,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceComponent {
    CoreDomain,
    Config,
    MetaStore,
    Daemon,
    Cli,
    Parser,
    Index,
    Search,
    Unknown,
}

impl fmt::Display for SourceComponent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let component = match self {
            SourceComponent::CoreDomain => "core-domain",
            SourceComponent::Config => "config",
            SourceComponent::MetaStore => "meta-store",
            SourceComponent::Daemon => "daemon",
            SourceComponent::Cli => "cli",
            SourceComponent::Parser => "parser",
            SourceComponent::Index => "index",
            SourceComponent::Search => "search",
            SourceComponent::Unknown => "unknown",
        };

        formatter.write_str(component)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ResumeIrError {
    pub kind: ErrorKind,
    pub retryable: bool,
    pub user_message: String,
    diagnostic_message: String,
    pub redaction_level: RedactionLevel,
    pub source_component: SourceComponent,
}

impl ResumeIrError {
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

    pub fn diagnostic_message(&self) -> &str {
        &self.diagnostic_message
    }

    fn diagnostic_debug_value(&self) -> &'static str {
        match self.redaction_level {
            RedactionLevel::None => "<available>",
            RedactionLevel::Sensitive | RedactionLevel::Secret => "<redacted>",
        }
    }
}

impl fmt::Debug for ResumeIrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResumeIrError")
            .field("kind", &self.kind)
            .field("retryable", &self.retryable)
            .field("user_message", &self.user_message)
            .field("diagnostic_message", &self.diagnostic_debug_value())
            .field("redaction_level", &self.redaction_level)
            .field("source_component", &self.source_component)
            .finish()
    }
}

impl fmt::Display for ResumeIrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} [kind={:?}, source_component={}, retryable={}]",
            self.user_message, self.kind, self.source_component, self.retryable
        )
    }
}

impl std::error::Error for ResumeIrError {}
