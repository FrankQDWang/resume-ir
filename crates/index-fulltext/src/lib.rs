pub fn crate_name() -> &'static str {
    "index-fulltext"
}

use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use regex::Regex;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, Schema, TantivyDocument, Value, FAST, STORED, STRING, TEXT};
use tantivy::{Index, IndexReader, IndexWriter};

const WRITER_HEAP_BYTES: usize = 50_000_000;
const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 100;
const ACTIVE_SNAPSHOT_FILE: &str = "active-snapshot";
const SNAPSHOTS_DIR: &str = "snapshots";
const STAGING_DIR: &str = "staging";

#[derive(Clone, PartialEq, Eq)]
pub struct IndexDocument {
    pub doc_id: String,
    pub version_id: String,
    pub file_name: String,
    pub clean_text: String,
    pub sections: Vec<IndexSection>,
    pub is_deleted: bool,
}

impl fmt::Debug for IndexDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndexDocument")
            .field("doc_id", &self.doc_id)
            .field("version_id", &self.version_id)
            .field("file_name", &"<redacted>")
            .field("clean_text", &"<redacted>")
            .field("section_count", &self.sections.len())
            .field("is_deleted", &self.is_deleted)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IndexSection {
    pub section_type: String,
    pub text: String,
}

impl fmt::Debug for IndexSection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndexSection")
            .field("section_type", &self.section_type)
            .field("text", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SearchQuery {
    text: String,
    limit: usize,
}

impl fmt::Debug for SearchQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchQuery")
            .field("text", &"<redacted>")
            .field("limit", &self.limit)
            .finish()
    }
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            limit: DEFAULT_LIMIT,
        }
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.clamp(1, MAX_LIMIT);
        self
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
}

#[derive(Clone, PartialEq)]
pub struct SearchHit {
    pub rank: usize,
    pub score: f32,
    pub doc_id: String,
    pub version_id: String,
    pub file_name: String,
    pub snippet: String,
}

impl fmt::Debug for SearchHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchHit")
            .field("rank", &self.rank)
            .field("score", &self.score)
            .field("doc_id", &self.doc_id)
            .field("version_id", &self.version_id)
            .field("file_name", &"<redacted>")
            .field("snippet", &"<redacted>")
            .finish()
    }
}

pub struct FullTextIndex {
    index: Index,
    reader: IndexReader,
    writer: Option<Mutex<IndexWriter>>,
    fields: IndexFields,
}

impl FullTextIndex {
    pub fn open(index_dir: &Path) -> Result<Self> {
        let index = Index::open_in_dir(index_dir).map_err(FullTextError::tantivy)?;
        let schema = index.schema();
        let fields = IndexFields::from_schema(&schema)?;
        let reader = index.reader().map_err(FullTextError::tantivy)?;

        Ok(Self {
            index,
            reader,
            writer: None,
            fields,
        })
    }

    pub fn open_or_create(index_dir: &Path) -> Result<Self> {
        fs::create_dir_all(index_dir).map_err(FullTextError::io)?;
        let schema = build_schema();
        let index = if index_dir.join("meta.json").exists() {
            Index::open_in_dir(index_dir).map_err(FullTextError::tantivy)?
        } else {
            Index::create_in_dir(index_dir, schema).map_err(FullTextError::tantivy)?
        };

        let schema = index.schema();
        let fields = IndexFields::from_schema(&schema)?;
        let reader = index.reader().map_err(FullTextError::tantivy)?;
        let writer = index
            .writer(WRITER_HEAP_BYTES)
            .map_err(FullTextError::tantivy)?;

        Ok(Self {
            index,
            reader,
            writer: Some(Mutex::new(writer)),
            fields,
        })
    }

    pub fn open_active(index_root: &Path) -> Result<Option<Self>> {
        let Some(target_dir) = active_index_dir(index_root)? else {
            return Ok(None);
        };

        Self::open(&target_dir).map(Some)
    }

    pub fn replace_documents<I>(&self, documents: I) -> Result<()>
    where
        I: IntoIterator<Item = IndexDocument>,
    {
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| FullTextError::internal("index opened read-only"))?
            .lock()
            .map_err(|_| FullTextError::internal("index writer lock poisoned"))?;
        writer
            .delete_all_documents()
            .map_err(FullTextError::tantivy)?;

        for document in documents {
            if document.is_deleted {
                continue;
            }

            let file_name = redact_contact_values(&document.file_name);
            let clean_text = redact_contact_values(&document.clean_text);
            let sections = document
                .sections
                .iter()
                .map(|section| {
                    (
                        section.section_type.as_str(),
                        redact_contact_values(&section.text),
                    )
                })
                .collect::<Vec<_>>();
            let section_text = document
                .sections
                .iter()
                .zip(sections.iter())
                .map(|(_, (_, text))| text.as_str())
                .collect::<Vec<_>>()
                .join("\n");

            let mut tantivy_document = TantivyDocument::default();
            tantivy_document.add_text(self.fields.doc_id, &document.doc_id);
            tantivy_document.add_text(self.fields.version_id, &document.version_id);
            tantivy_document.add_text(self.fields.file_name, &file_name);
            tantivy_document.add_text(self.fields.clean_text, &clean_text);
            tantivy_document.add_text(self.fields.all_sections, &section_text);
            tantivy_document.add_bool(self.fields.is_deleted, false);
            for (section_type, text) in &sections {
                tantivy_document.add_text(self.fields.section_type, section_type);
                tantivy_document.add_text(self.fields.section_text, text);
            }
            writer
                .add_document(tantivy_document)
                .map_err(FullTextError::tantivy)?;
        }

        Ok(())
    }

    pub fn commit(&self) -> Result<()> {
        let mut writer = self
            .writer
            .as_ref()
            .ok_or_else(|| FullTextError::internal("index opened read-only"))?
            .lock()
            .map_err(|_| FullTextError::internal("index writer lock poisoned"))?;
        writer.commit().map_err(FullTextError::tantivy)?;
        Ok(())
    }

    pub fn reload(&self) -> Result<()> {
        self.reader.reload().map_err(FullTextError::tantivy)
    }

    pub fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        self.reload()?;
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.fields.file_name,
                self.fields.clean_text,
                self.fields.section_text,
                self.fields.all_sections,
            ],
        );
        if query.text().trim().is_empty() {
            return Ok(Vec::new());
        }

        let (parsed_query, _parse_errors) = query_parser.parse_query_lenient(query.text());
        let candidate_limit = query.limit();
        let top_docs = searcher
            .search(
                &parsed_query,
                &TopDocs::with_limit(candidate_limit).order_by_score(),
            )
            .map_err(FullTextError::tantivy)?;

        let mut hits = Vec::new();
        let mut seen_doc_ids = std::collections::BTreeSet::new();
        for (score, address) in top_docs {
            let stored = searcher
                .doc::<TantivyDocument>(address)
                .map_err(FullTextError::tantivy)?;
            if bool_value(&stored, self.fields.is_deleted).unwrap_or(false) {
                continue;
            }

            let Some(doc_id) = text_value(&stored, self.fields.doc_id) else {
                continue;
            };
            if !seen_doc_ids.insert(doc_id.clone()) {
                continue;
            }

            let clean_text = text_value(&stored, self.fields.clean_text).unwrap_or_default();
            hits.push(SearchHit {
                rank: hits.len() + 1,
                score,
                doc_id,
                version_id: text_value(&stored, self.fields.version_id).unwrap_or_default(),
                file_name: text_value(&stored, self.fields.file_name).unwrap_or_default(),
                snippet: build_snippet(&clean_text, query.text()),
            });

            if hits.len() == query.limit() {
                break;
            }
        }

        Ok(hits)
    }
}

pub fn publish_snapshot<I>(index_root: &Path, snapshot_name: &str, documents: I) -> Result<()>
where
    I: IntoIterator<Item = IndexDocument>,
{
    validate_snapshot_name(snapshot_name)?;

    let staging_root = index_root.join(STAGING_DIR);
    let snapshots_root = index_root.join(SNAPSHOTS_DIR);
    fs::create_dir_all(&staging_root).map_err(FullTextError::io)?;
    fs::create_dir_all(&snapshots_root).map_err(FullTextError::io)?;

    let staging_dir = staging_root.join(format!("{snapshot_name}.tmp"));
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(FullTextError::io)?;
    }
    let published_dir = snapshots_root.join(snapshot_name);
    if published_dir.exists() {
        return Err(FullTextError::internal("full-text snapshot already exists"));
    }

    let index = FullTextIndex::open_or_create(&staging_dir)?;
    index.replace_documents(documents)?;
    index.commit()?;
    drop(index);

    let validation = FullTextIndex::open(&staging_dir)?;
    validation.search(SearchQuery::new("diagnostic").with_limit(1))?;
    drop(validation);

    fs::rename(&staging_dir, &published_dir).map_err(FullTextError::io)?;
    write_active_snapshot(index_root, snapshot_name)?;

    Ok(())
}

pub fn inspect_snapshot_root(index_root: &Path) -> Result<SnapshotRootInspection> {
    let staging_orphans = staging_orphan_count(index_root)?;
    match read_active_snapshot_pointer(index_root)? {
        ActiveSnapshotPointer::Valid(snapshot_name) => {
            let snapshot_dir = index_root.join(SNAPSHOTS_DIR).join(&snapshot_name);
            if snapshot_dir.join("meta.json").exists() && snapshot_is_usable(&snapshot_dir) {
                return Ok(SnapshotRootInspection {
                    state: SnapshotRootState::Ready,
                    read_target: Some(SnapshotReadTarget::PublishedSnapshot),
                    active_snapshot: Some(snapshot_name),
                    fallback_snapshot: None,
                    staging_orphans,
                });
            }

            if let Some(fallback_snapshot) = last_good_snapshot(index_root, Some(&snapshot_name))? {
                return Ok(SnapshotRootInspection {
                    state: SnapshotRootState::Recovered,
                    read_target: Some(SnapshotReadTarget::PublishedSnapshot),
                    active_snapshot: Some(snapshot_name),
                    fallback_snapshot: Some(fallback_snapshot),
                    staging_orphans,
                });
            }

            return Ok(SnapshotRootInspection {
                state: if snapshot_dir.join("meta.json").exists() {
                    SnapshotRootState::Corrupt
                } else {
                    SnapshotRootState::ActiveMissing
                },
                read_target: None,
                active_snapshot: Some(snapshot_name),
                fallback_snapshot: None,
                staging_orphans,
            });
        }
        ActiveSnapshotPointer::Invalid => {
            if let Some(fallback_snapshot) = last_good_snapshot(index_root, None)? {
                return Ok(SnapshotRootInspection {
                    state: SnapshotRootState::Recovered,
                    read_target: Some(SnapshotReadTarget::PublishedSnapshot),
                    active_snapshot: None,
                    fallback_snapshot: Some(fallback_snapshot),
                    staging_orphans,
                });
            }

            return Ok(SnapshotRootInspection {
                state: SnapshotRootState::Corrupt,
                read_target: None,
                active_snapshot: None,
                fallback_snapshot: None,
                staging_orphans,
            });
        }
        ActiveSnapshotPointer::Missing => {
            if let Some(fallback_snapshot) = last_good_snapshot(index_root, None)? {
                return Ok(SnapshotRootInspection {
                    state: SnapshotRootState::Recovered,
                    read_target: Some(SnapshotReadTarget::PublishedSnapshot),
                    active_snapshot: None,
                    fallback_snapshot: Some(fallback_snapshot),
                    staging_orphans,
                });
            }
        }
    }

    if index_root.join("meta.json").exists() {
        let state = if FullTextIndex::open(index_root).is_ok() {
            SnapshotRootState::Ready
        } else {
            SnapshotRootState::Corrupt
        };
        return Ok(SnapshotRootInspection {
            state,
            read_target: Some(SnapshotReadTarget::LegacyRoot),
            active_snapshot: None,
            fallback_snapshot: None,
            staging_orphans,
        });
    }

    Ok(SnapshotRootInspection {
        state: SnapshotRootState::Missing,
        read_target: None,
        active_snapshot: None,
        fallback_snapshot: None,
        staging_orphans,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRootInspection {
    state: SnapshotRootState,
    read_target: Option<SnapshotReadTarget>,
    active_snapshot: Option<String>,
    fallback_snapshot: Option<String>,
    staging_orphans: usize,
}

impl SnapshotRootInspection {
    pub fn state(&self) -> SnapshotRootState {
        self.state
    }

    pub fn read_target(&self) -> Option<SnapshotReadTarget> {
        self.read_target
    }

    pub fn active_snapshot(&self) -> Option<&str> {
        self.active_snapshot.as_deref()
    }

    pub fn fallback_snapshot(&self) -> Option<&str> {
        self.fallback_snapshot.as_deref()
    }

    pub fn staging_orphans(&self) -> usize {
        self.staging_orphans
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotRootState {
    Missing,
    Ready,
    Recovered,
    Corrupt,
    ActiveMissing,
}

impl SnapshotRootState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Ready => "ready",
            Self::Recovered => "recovered",
            Self::Corrupt => "corrupt",
            Self::ActiveMissing => "active_missing",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotReadTarget {
    PublishedSnapshot,
    LegacyRoot,
}

impl SnapshotReadTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::PublishedSnapshot => "published_snapshot",
            Self::LegacyRoot => "legacy_root",
        }
    }
}

fn active_index_dir(index_root: &Path) -> Result<Option<PathBuf>> {
    let inspection = inspect_snapshot_root(index_root)?;
    match inspection.state() {
        SnapshotRootState::Ready | SnapshotRootState::Recovered => match inspection.read_target() {
            Some(SnapshotReadTarget::PublishedSnapshot) => {
                let snapshot_name = inspection
                    .fallback_snapshot()
                    .or_else(|| inspection.active_snapshot())
                    .ok_or_else(|| FullTextError::internal("full-text snapshot pointer missing"))?;
                Ok(Some(index_root.join(SNAPSHOTS_DIR).join(snapshot_name)))
            }
            Some(SnapshotReadTarget::LegacyRoot) => Ok(Some(index_root.to_path_buf())),
            None => Err(FullTextError::internal("full-text snapshot target missing")),
        },
        SnapshotRootState::Missing => Ok(None),
        SnapshotRootState::Corrupt | SnapshotRootState::ActiveMissing => {
            Err(FullTextError::internal("full-text snapshot is unavailable"))
        }
    }
}

enum ActiveSnapshotPointer {
    Missing,
    Valid(String),
    Invalid,
}

fn read_active_snapshot_pointer(index_root: &Path) -> Result<ActiveSnapshotPointer> {
    let path = index_root.join(ACTIVE_SNAPSHOT_FILE);
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(ActiveSnapshotPointer::Missing);
        }
        Err(error) => return Err(FullTextError::io(error)),
    };
    let snapshot_name = content.trim();
    if validate_snapshot_name(snapshot_name).is_err() {
        return Ok(ActiveSnapshotPointer::Invalid);
    }
    Ok(ActiveSnapshotPointer::Valid(snapshot_name.to_string()))
}

fn write_active_snapshot(index_root: &Path, snapshot_name: &str) -> Result<()> {
    validate_snapshot_name(snapshot_name)?;
    let active_path = index_root.join(ACTIVE_SNAPSHOT_FILE);
    let temp_path = index_root.join(format!(".{ACTIVE_SNAPSHOT_FILE}.tmp"));
    fs::write(&temp_path, format!("{snapshot_name}\n")).map_err(FullTextError::io)?;
    match fs::rename(&temp_path, &active_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            fs::remove_file(&active_path).map_err(FullTextError::io)?;
            fs::rename(&temp_path, &active_path).map_err(FullTextError::io)
        }
        Err(error) => Err(FullTextError::io(error)),
    }
}

fn staging_orphan_count(index_root: &Path) -> Result<usize> {
    let staging_root = index_root.join(STAGING_DIR);
    let entries = match fs::read_dir(staging_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(FullTextError::io(error)),
    };

    let mut count = 0_usize;
    for entry in entries {
        let entry = entry.map_err(FullTextError::io)?;
        if entry.file_type().map_err(FullTextError::io)?.is_dir() {
            count += 1;
        }
    }
    Ok(count)
}

fn last_good_snapshot(
    index_root: &Path,
    excluded_snapshot: Option<&str>,
) -> Result<Option<String>> {
    let snapshots_root = index_root.join(SNAPSHOTS_DIR);
    let entries = match fs::read_dir(snapshots_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(FullTextError::io(error)),
    };

    let mut snapshots = Vec::new();
    for entry in entries {
        let entry = entry.map_err(FullTextError::io)?;
        if !entry.file_type().map_err(FullTextError::io)?.is_dir() {
            continue;
        }
        let Ok(snapshot_name) = entry.file_name().into_string() else {
            continue;
        };
        if excluded_snapshot == Some(snapshot_name.as_str())
            || validate_snapshot_name(&snapshot_name).is_err()
        {
            continue;
        }
        snapshots.push(snapshot_name);
    }
    snapshots.sort_by(|left, right| right.cmp(left));

    for snapshot_name in snapshots {
        let snapshot_dir = index_root.join(SNAPSHOTS_DIR).join(&snapshot_name);
        if snapshot_is_usable(&snapshot_dir) {
            return Ok(Some(snapshot_name));
        }
    }

    Ok(None)
}

fn snapshot_is_usable(snapshot_dir: &Path) -> bool {
    if !snapshot_metadata_looks_valid(snapshot_dir) {
        return false;
    }

    FullTextIndex::open(snapshot_dir)
        .and_then(|index| {
            index
                .search(SearchQuery::new("diagnostic").with_limit(1))
                .map(|_| ())
        })
        .is_ok()
}

fn snapshot_metadata_looks_valid(snapshot_dir: &Path) -> bool {
    let Ok(meta_json) = fs::read_to_string(snapshot_dir.join("meta.json")) else {
        return false;
    };
    meta_json.trim_start().starts_with('{')
}

fn validate_snapshot_name(snapshot_name: &str) -> Result<()> {
    if snapshot_name.is_empty()
        || !snapshot_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(FullTextError::internal(
            "full-text snapshot name is invalid",
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct IndexFields {
    doc_id: Field,
    version_id: Field,
    file_name: Field,
    clean_text: Field,
    section_type: Field,
    section_text: Field,
    all_sections: Field,
    is_deleted: Field,
}

impl IndexFields {
    fn from_schema(schema: &Schema) -> Result<Self> {
        Ok(Self {
            doc_id: schema.get_field("doc_id").map_err(FullTextError::tantivy)?,
            version_id: schema
                .get_field("version_id")
                .map_err(FullTextError::tantivy)?,
            file_name: schema
                .get_field("file_name")
                .map_err(FullTextError::tantivy)?,
            clean_text: schema
                .get_field("clean_text")
                .map_err(FullTextError::tantivy)?,
            section_type: schema
                .get_field("section_type")
                .map_err(FullTextError::tantivy)?,
            section_text: schema
                .get_field("section_text")
                .map_err(FullTextError::tantivy)?,
            all_sections: schema
                .get_field("all_sections")
                .map_err(FullTextError::tantivy)?,
            is_deleted: schema
                .get_field("is_deleted")
                .map_err(FullTextError::tantivy)?,
        })
    }
}

fn build_schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_text_field("doc_id", STRING | STORED | FAST);
    builder.add_text_field("version_id", STRING | STORED | FAST);
    builder.add_text_field("file_name", TEXT | STORED);
    builder.add_text_field("clean_text", TEXT | STORED);
    builder.add_text_field("section_type", STRING | STORED | FAST);
    builder.add_text_field("section_text", TEXT | STORED);
    builder.add_text_field("all_sections", TEXT | STORED);
    builder.add_bool_field("is_deleted", STORED | FAST);
    builder.build()
}

fn text_value(document: &TantivyDocument, field: Field) -> Option<String> {
    document
        .get_first(field)
        .and_then(|value| value.as_value().as_str())
        .map(str::to_string)
}

fn bool_value(document: &TantivyDocument, field: Field) -> Option<bool> {
    document
        .get_first(field)
        .and_then(|value| value.as_value().as_bool())
}

fn build_snippet(text: &str, query: &str) -> String {
    let terms = query.split_whitespace().collect::<Vec<_>>();
    let lower_text = text.to_ascii_lowercase();
    let first_match = terms
        .iter()
        .filter(|term| !term.is_empty())
        .find_map(|term| lower_text.find(&term.to_ascii_lowercase()))
        .unwrap_or(0);

    let start = nearest_char_boundary_before(text, first_match.saturating_sub(40));
    let end = nearest_char_boundary_after(text, (first_match + 80).min(text.len()));
    redact_contact_values(text[start..end].trim())
}

pub fn redact_contact_values(text: &str) -> String {
    static EMAIL_REGEX: OnceLock<Regex> = OnceLock::new();
    static PHONE_REGEX: OnceLock<Regex> = OnceLock::new();
    static COMPACT_PHONE_REGEX: OnceLock<Regex> = OnceLock::new();

    let email_redacted = EMAIL_REGEX
        .get_or_init(|| Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").unwrap())
        .replace_all(text, "<redacted-email>");
    let phone_redacted = PHONE_REGEX
        .get_or_init(|| {
            Regex::new(
                r"(?x)
                (?:\+\d{1,3}[\s.-]*)?
                (?:
                    \(\d{3}\)[\s.-]*
                    |
                    \d{3,4}[\s.-]+
                )
                \d{3,4}[\s.-]*\d{4}
                ",
            )
            .unwrap()
        })
        .replace_all(&email_redacted, "<redacted-phone>");
    COMPACT_PHONE_REGEX
        .get_or_init(|| Regex::new(r"\+?(?:1)?\d{10}\b").unwrap())
        .replace_all(&phone_redacted, "<redacted-phone>")
        .into_owned()
}

fn nearest_char_boundary_before(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn nearest_char_boundary_after(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

pub type Result<T> = std::result::Result<T, FullTextError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FullTextError {
    Io { diagnostic: String },
    Tantivy { diagnostic: String },
    Internal { diagnostic: String },
}

impl FullTextError {
    fn io(error: std::io::Error) -> Self {
        Self::Io {
            diagnostic: error.to_string(),
        }
    }

    fn tantivy(error: tantivy::TantivyError) -> Self {
        Self::Tantivy {
            diagnostic: error.to_string(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            diagnostic: message.into(),
        }
    }
}

impl fmt::Display for FullTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FullTextError::Io { .. } => formatter.write_str("full-text index IO error"),
            FullTextError::Tantivy { .. } => {
                formatter.write_str("full-text index operation failed")
            }
            FullTextError::Internal { .. } => formatter.write_str("full-text index internal error"),
        }
    }
}

impl std::error::Error for FullTextError {}
