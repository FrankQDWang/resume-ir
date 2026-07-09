use std::{collections::BTreeSet, fmt, str::FromStr};

use unicode_normalization::UnicodeNormalization;

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

pub const QUERY_SET_BUCKETS: [&str; 7] = [
    "single_term",
    "and_2",
    "and_3_5",
    "and_6_16",
    "field_filter",
    "hybrid",
    "semantic",
];
pub const QUERY_SET_MAX_QUERY_BYTES: usize = 4096;
pub const QUERY_SET_MAX_TERMS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuerySetSourceKind {
    TraceSourceSearchV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuerySetSourceKindParseError {
    Invalid,
}

impl QuerySetSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TraceSourceSearchV1 => "trace_source_search_v1",
        }
    }

    pub fn is_agent_query_replay(self) -> bool {
        matches!(self, Self::TraceSourceSearchV1)
    }
}

impl FromStr for QuerySetSourceKind {
    type Err = QuerySetSourceKindParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "trace_source_search_v1" => Ok(Self::TraceSourceSearchV1),
            _ => Err(QuerySetSourceKindParseError::Invalid),
        }
    }
}

impl fmt::Display for QuerySetSourceKindParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => formatter.write_str("invalid query set source kind"),
        }
    }
}

impl std::error::Error for QuerySetSourceKindParseError {}

pub fn query_set_query_in_semantic_bounds(query: &str) -> bool {
    let term_count = QuerySetSampleShape::from_query(query).term_count();
    query.len() <= QUERY_SET_MAX_QUERY_BYTES && term_count > 0 && term_count <= QUERY_SET_MAX_TERMS
}

pub fn normalize_query_set_query(query: &str) -> Option<String> {
    let normalized = query.nfkc().collect::<String>();
    let mut seen = BTreeSet::new();
    let mut terms = Vec::new();
    for term in query_set_logical_terms(&normalized) {
        let value = term.value.split_whitespace().collect::<Vec<_>>().join(" ");
        if value.is_empty() {
            continue;
        }
        let key = (term.quoted, value.clone());
        if !seen.insert(key) {
            continue;
        }
        if term.quoted {
            terms.push(format!("\"{value}\""));
        } else {
            terms.push(value);
        }
    }
    let normalized = terms.join(" ");
    query_set_query_in_semantic_bounds(&normalized).then_some(normalized)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuerySetSampleShape {
    term_count: usize,
    has_boolean: bool,
    has_location: bool,
    has_years: bool,
    has_degree: bool,
    has_skill: bool,
    has_phrase: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuerySetSampleShapeMetadata {
    pub term_count: usize,
    pub has_boolean: bool,
    pub has_location: bool,
    pub has_years: bool,
    pub has_degree: bool,
    pub has_skill: bool,
    pub has_phrase: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuerySetLogicalTerm<'a> {
    value: &'a str,
    quoted: bool,
}

impl QuerySetSampleShape {
    pub fn from_query(query: &str) -> Self {
        let lower = query.to_lowercase();
        let terms = query_set_logical_terms(query);
        Self {
            term_count: terms.len(),
            has_boolean: terms
                .iter()
                .any(|term| !term.quoted && is_query_set_explicit_boolean_operator(term.value)),
            has_location: query_set_contains_any_token(
                &lower,
                &[
                    "beijing",
                    "shanghai",
                    "shenzhen",
                    "guangzhou",
                    "hangzhou",
                    "chengdu",
                    "wuhan",
                    "北京",
                    "上海",
                    "深圳",
                    "广州",
                    "杭州",
                    "成都",
                    "武汉",
                ],
            ),
            has_years: query.chars().any(|ch| ch.is_ascii_digit()),
            has_degree: query_set_contains_any_token(
                &lower,
                &[
                    "bachelor", "master", "phd", "doctor", "degree", "本科", "硕士", "博士", "学历",
                ],
            ),
            has_skill: terms
                .iter()
                .any(|term| term.value.chars().any(char::is_alphabetic)),
            has_phrase: terms.iter().any(|term| term.quoted),
        }
    }

    pub fn from_metadata(metadata: QuerySetSampleShapeMetadata) -> Self {
        Self {
            term_count: metadata.term_count,
            has_boolean: metadata.has_boolean,
            has_location: metadata.has_location,
            has_years: metadata.has_years,
            has_degree: metadata.has_degree,
            has_skill: metadata.has_skill,
            has_phrase: metadata.has_phrase,
        }
    }

    pub fn term_count(self) -> usize {
        self.term_count
    }

    pub fn has_boolean(self) -> bool {
        self.has_boolean
    }

    pub fn has_location(self) -> bool {
        self.has_location
    }

    pub fn has_years(self) -> bool {
        self.has_years
    }

    pub fn has_degree(self) -> bool {
        self.has_degree
    }

    pub fn has_skill(self) -> bool {
        self.has_skill
    }

    pub fn has_phrase(self) -> bool {
        self.has_phrase
    }

    pub fn bucket(self) -> &'static str {
        if self.has_boolean {
            "hybrid"
        } else if self.has_phrase {
            "semantic"
        } else if self.term_count >= 2 && (self.has_location || self.has_years || self.has_degree) {
            "field_filter"
        } else {
            match self.term_count {
                0 | 1 => "single_term",
                2 => "and_2",
                3..=5 => "and_3_5",
                _ => "and_6_16",
            }
        }
    }
}

fn query_set_logical_terms(query: &str) -> Vec<QuerySetLogicalTerm<'_>> {
    let mut terms = Vec::new();
    let mut chars = query.char_indices().peekable();
    while let Some((start, character)) = chars.next() {
        if character.is_whitespace() {
            continue;
        }
        if is_query_set_phrase_quote(character) {
            let content_start = start + character.len_utf8();
            let mut content_end = query.len();
            for (index, next) in chars.by_ref() {
                if is_query_set_phrase_quote(next) {
                    content_end = index;
                    break;
                }
            }
            let value = query[content_start..content_end].trim();
            if !value.is_empty() {
                terms.push(QuerySetLogicalTerm {
                    value,
                    quoted: true,
                });
            }
            continue;
        }
        let mut end = query.len();
        while let Some((index, next)) = chars.peek().copied() {
            if next.is_whitespace() {
                end = index;
                break;
            }
            chars.next();
        }
        terms.push(QuerySetLogicalTerm {
            value: &query[start..end],
            quoted: false,
        });
    }
    terms
}

fn is_query_set_phrase_quote(character: char) -> bool {
    matches!(character, '"' | '“' | '”')
}

fn is_query_set_explicit_boolean_operator(value: &str) -> bool {
    matches!(
        value.trim_matches(|ch: char| !ch.is_alphanumeric()),
        "AND" | "OR" | "NOT"
    )
}

fn query_set_contains_any_token(value: &str, tokens: &[&str]) -> bool {
    tokens.iter().any(|token| value.contains(token))
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContactHash(String);

impl ContactHash {
    /// Hydrates a contact hash produced by an external keyed digest.
    ///
    /// S2 intentionally does not provide phone/email hashing. Callers must
    /// supply a keyed digest from the privacy boundary responsible for PII.
    pub fn from_keyed_digest(digest: impl Into<String>) -> Result<Self, ContactHashParseError> {
        let digest = digest.into().to_ascii_lowercase();
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
    OcrDocument,
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
    WeChat,
    School,
    SchoolTier,
    Degree,
    Major,
    Company,
    Title,
    Education,
    Skills,
    Skill,
    Certificate,
    Date,
    DateRange,
    YearsExperience,
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
