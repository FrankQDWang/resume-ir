pub fn crate_name() -> &'static str {
    "index-fulltext"
}

use std::borrow::{Borrow, Cow};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use regex::Regex;
use tantivy::collector::TopDocs;
use tantivy::indexer::NoMergePolicy;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TantivyDocument, Value, STORED, STRING, TEXT,
};
use tantivy::{Index, IndexReader, IndexWriter, Term};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const DEFAULT_WRITER_HEAP_BYTES: usize = 50_000_000;
const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 100;
const ACTIVE_SNAPSHOT_FILE: &str = "active-snapshot";
const SNAPSHOTS_DIR: &str = "snapshots";
const STAGING_DIR: &str = "staging";
const ENCRYPTED_SNAPSHOT_FILE: &str = "fulltext.snapshot.enc";
const SNAPSHOT_MANIFEST_FILE: &str = "snapshot-manifest.json";
const SNAPSHOT_MANIFEST_SCHEMA_VERSION: &str = "fulltext.snapshot.v1";
const FULLTEXT_INDEX_SCHEMA_VERSION: &str = "tantivy.fulltext.v1";
const SNAPSHOT_KEY_FILE: &str = "fulltext.snapshot.key-v1";
const SNAPSHOT_HEADER_ENCRYPTED_V1: &str = "resume-ir-fulltext-snapshot-encrypted-v1";
const SNAPSHOT_ARCHIVE_HEADER_V1: &[u8] = b"resume-ir-fulltext-snapshot-archive-v1\n";
const SNAPSHOT_KEY_LEN: usize = 32;
const SNAPSHOT_NONCE_LEN: usize = 24;
const SNAPSHOT_PUBLISH_RETRY_ATTEMPTS: usize = 100;
const SNAPSHOT_PUBLISH_RETRY_DELAY: Duration = Duration::from_millis(50);
const INDEX_OPEN_RETRY_ATTEMPTS: usize = 20;
const INDEX_OPEN_RETRY_DELAY: Duration = Duration::from_millis(50);
const INDEX_MUTATION_RETRY_ATTEMPTS: usize = 20;
const INDEX_MUTATION_RETRY_DELAY: Duration = Duration::from_millis(50);
const SINGLE_WORKER_SNAPSHOT_DOCUMENT_LIMIT: usize = 10_000;
const SNAPSHOT_PUBLISH_CONTROL_DOCUMENT_INTERVAL: usize = 8;

#[cfg(test)]
static REDACTION_REGEX_PASSES: AtomicUsize = AtomicUsize::new(0);

#[cfg(test)]
fn record_redaction_regex_pass() {
    REDACTION_REGEX_PASSES.fetch_add(1, Ordering::Relaxed);
}

#[cfg(not(test))]
fn record_redaction_regex_pass() {}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotPublishPhase {
    Setup,
    DocumentIndexing,
    TantivyCommit,
    PlaintextValidation,
    EncryptedPublication,
    EncryptedValidation,
    ActiveSnapshotWrite,
}

impl SnapshotPublishPhase {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Setup => "index_publication_setup",
            Self::DocumentIndexing => "index_publication_documents",
            Self::TantivyCommit => "index_publication_commit",
            Self::PlaintextValidation => "index_publication_plaintext_validation",
            Self::EncryptedPublication => "index_publication_encrypted_publication",
            Self::EncryptedValidation => "index_publication_encrypted_validation",
            Self::ActiveSnapshotWrite => "index_publication_active_snapshot",
        }
    }
}

/// Optional cancellation and phase attribution control for long snapshot publication work.
#[derive(Clone, Copy)]
pub struct SnapshotPublishControl<'a> {
    cancel_check: Option<&'a dyn Fn() -> bool>,
    phase_observer: Option<&'a dyn Fn(SnapshotPublishPhase)>,
    phase_timing_observer: Option<&'a dyn Fn(SnapshotPublishPhase, Duration)>,
    document_interval: usize,
    writer_heap_bytes: usize,
}

impl<'a> SnapshotPublishControl<'a> {
    pub fn disabled() -> Self {
        Self {
            cancel_check: None,
            phase_observer: None,
            phase_timing_observer: None,
            document_interval: SNAPSHOT_PUBLISH_CONTROL_DOCUMENT_INTERVAL,
            writer_heap_bytes: DEFAULT_WRITER_HEAP_BYTES,
        }
    }

    pub fn from_cancel_check(cancel_check: &'a dyn Fn() -> bool) -> Self {
        Self {
            cancel_check: Some(cancel_check),
            phase_observer: None,
            phase_timing_observer: None,
            document_interval: SNAPSHOT_PUBLISH_CONTROL_DOCUMENT_INTERVAL,
            writer_heap_bytes: DEFAULT_WRITER_HEAP_BYTES,
        }
    }

    pub fn with_phase_observer(mut self, phase_observer: &'a dyn Fn(SnapshotPublishPhase)) -> Self {
        self.phase_observer = Some(phase_observer);
        self
    }

    pub fn with_phase_timing_observer(
        mut self,
        phase_timing_observer: &'a dyn Fn(SnapshotPublishPhase, Duration),
    ) -> Self {
        self.phase_timing_observer = Some(phase_timing_observer);
        self
    }

    pub fn with_writer_heap_bytes(mut self, writer_heap_bytes: usize) -> Self {
        self.writer_heap_bytes = writer_heap_bytes.max(1);
        self
    }

    fn writer_heap_bytes(self) -> usize {
        self.writer_heap_bytes
    }

    fn report_phase(self, phase: SnapshotPublishPhase) {
        if let Some(phase_observer) = self.phase_observer {
            phase_observer(phase);
        }
    }

    fn report_phase_timing(self, phase: SnapshotPublishPhase, elapsed: Duration) {
        if let Some(phase_timing_observer) = self.phase_timing_observer {
            phase_timing_observer(phase, elapsed);
        }
    }

    fn check(self) -> Result<()> {
        if self.cancel_check.is_some_and(|cancel_check| cancel_check()) {
            return Err(FullTextError::cancelled());
        }

        Ok(())
    }

    fn check_after_document(self, index: usize) -> Result<()> {
        if index.is_multiple_of(self.document_interval) {
            self.check()?;
        }

        Ok(())
    }
}

pub struct FullTextIndex {
    index: Index,
    reader: IndexReader,
    writer: Option<Mutex<IndexWriter>>,
    fields: IndexFields,
    _decrypted_snapshot_dir: Option<PrivateTempDir>,
}

impl FullTextIndex {
    pub fn open(index_dir: &Path) -> Result<Self> {
        retry_transient_index_open(
            || Self::open_once(index_dir),
            INDEX_OPEN_RETRY_ATTEMPTS,
            INDEX_OPEN_RETRY_DELAY,
        )
    }

    fn open_once(index_dir: &Path) -> Result<Self> {
        let index = Index::open_in_dir(index_dir).map_err(FullTextError::tantivy)?;
        let schema = index.schema();
        let fields = IndexFields::from_schema(&schema)?;
        let reader = index.reader().map_err(FullTextError::tantivy)?;

        Ok(Self {
            index,
            reader,
            writer: None,
            fields,
            _decrypted_snapshot_dir: None,
        })
    }

    pub fn open_or_create(index_dir: &Path) -> Result<Self> {
        Self::open_or_create_with_writer_mode(
            index_dir,
            WriterThreadMode::Auto,
            DEFAULT_WRITER_HEAP_BYTES,
        )
    }

    fn open_or_create_with_writer_mode(
        index_dir: &Path,
        writer_thread_mode: WriterThreadMode,
        writer_heap_bytes: usize,
    ) -> Result<Self> {
        Self::open_or_create_with_writer_config(
            index_dir,
            SnapshotWriterConfig {
                thread_mode: writer_thread_mode,
                merge_policy: WriterMergePolicy::Default,
            },
            writer_heap_bytes,
        )
    }

    fn open_or_create_with_writer_config(
        index_dir: &Path,
        writer_config: SnapshotWriterConfig,
        writer_heap_bytes: usize,
    ) -> Result<Self> {
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
        let writer = match writer_config.thread_mode {
            WriterThreadMode::Auto => index.writer(writer_heap_bytes),
            WriterThreadMode::SingleWorker => index.writer_with_num_threads(1, writer_heap_bytes),
        }
        .map_err(FullTextError::tantivy)?;
        if matches!(writer_config.merge_policy, WriterMergePolicy::NoMerge) {
            writer.set_merge_policy(Box::new(NoMergePolicy));
        }

        Ok(Self {
            index,
            reader,
            writer: Some(Mutex::new(writer)),
            fields,
            _decrypted_snapshot_dir: None,
        })
    }

    pub fn open_active(index_root: &Path) -> Result<Option<Self>> {
        match active_index_dir(index_root)? {
            Some(ActiveIndexDir::PublishedSnapshot(snapshot_dir)) => {
                open_published_snapshot(&snapshot_dir).map(Some)
            }
            Some(ActiveIndexDir::LegacyRoot(index_dir)) => Self::open(&index_dir).map(Some),
            None => Ok(None),
        }
    }

    pub fn replace_documents<I>(&self, documents: I) -> Result<()>
    where
        I: IntoIterator<Item = IndexDocument>,
    {
        self.replace_documents_with_control(documents, SnapshotPublishControl::disabled())
    }

    pub fn replace_document_refs<'a, I>(&self, documents: I) -> Result<()>
    where
        I: IntoIterator<Item = &'a IndexDocument>,
    {
        self.replace_documents_with_control(documents, SnapshotPublishControl::disabled())
    }

    fn replace_documents_with_control<I, D>(
        &self,
        documents: I,
        control: SnapshotPublishControl<'_>,
    ) -> Result<()>
    where
        I: IntoIterator<Item = D>,
        D: Borrow<IndexDocument>,
    {
        self.replace_documents_with_redaction(documents, control, IndexDocumentRedaction::Redact)
    }

    fn replace_documents_with_redaction<I, D>(
        &self,
        documents: I,
        control: SnapshotPublishControl<'_>,
        redaction: IndexDocumentRedaction,
    ) -> Result<()>
    where
        I: IntoIterator<Item = D>,
        D: Borrow<IndexDocument>,
    {
        control.report_phase(SnapshotPublishPhase::DocumentIndexing);
        control.check()?;
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| FullTextError::internal("index opened read-only"))?
            .lock()
            .map_err(|_| FullTextError::internal("index writer lock poisoned"))?;
        writer
            .delete_all_documents()
            .map_err(FullTextError::tantivy)?;

        for (index, document) in documents.into_iter().enumerate() {
            control.check_after_document(index)?;
            let document = document.borrow();
            if document.is_deleted {
                continue;
            }

            let (file_name, clean_text) = match redaction {
                IndexDocumentRedaction::Redact => (
                    redact_contact_values_cow(&document.file_name),
                    redact_contact_values_cow(&document.clean_text),
                ),
                IndexDocumentRedaction::TrustedRedacted => (
                    Cow::Borrowed(document.file_name.as_str()),
                    Cow::Borrowed(document.clean_text.as_str()),
                ),
            };
            let mut tantivy_document = TantivyDocument::default();
            tantivy_document.add_text(self.fields.doc_id, &document.doc_id);
            tantivy_document.add_text(self.fields.version_id, &document.version_id);
            tantivy_document.add_text(self.fields.file_name, file_name.as_ref());
            tantivy_document.add_text(self.fields.clean_text, clean_text.as_ref());
            tantivy_document.add_bool(self.fields.is_deleted, false);
            writer
                .add_document(tantivy_document)
                .map_err(FullTextError::tantivy)?;
        }
        control.check()?;

        Ok(())
    }

    pub fn commit(&self) -> Result<()> {
        let mut writer = self
            .writer
            .as_ref()
            .ok_or_else(|| FullTextError::internal("index opened read-only"))?
            .lock()
            .map_err(|_| FullTextError::internal("index writer lock poisoned"))?;
        retry_transient_index_mutation(
            || writer.commit().map(|_| ()).map_err(FullTextError::tantivy),
            INDEX_MUTATION_RETRY_ATTEMPTS,
            INDEX_MUTATION_RETRY_DELAY,
        )
    }

    pub fn reload(&self) -> Result<()> {
        self.reader.reload().map_err(FullTextError::tantivy)
    }

    pub fn search(&self, query: SearchQuery) -> Result<Vec<SearchHit>> {
        self.search_internal(query, None)
    }

    pub fn search_allowed_doc_ids(
        &self,
        query: SearchQuery,
        allowed_doc_ids: &BTreeSet<String>,
    ) -> Result<Vec<SearchHit>> {
        self.search_internal(query, Some(allowed_doc_ids))
    }

    fn stored_documents_except(
        &self,
        excluded_doc_ids: &BTreeSet<String>,
    ) -> Result<Vec<IndexDocument>> {
        self.reload()?;
        let searcher = self.reader.searcher();
        let mut documents = Vec::new();
        for segment_reader in searcher.segment_readers() {
            let store_reader = segment_reader
                .get_store_reader(10)
                .map_err(FullTextError::io)?;
            for stored in store_reader.iter::<TantivyDocument>(segment_reader.alive_bitset()) {
                let stored = stored.map_err(FullTextError::tantivy)?;
                if bool_value(&stored, self.fields.is_deleted).unwrap_or(false) {
                    continue;
                }

                let Some(doc_id) = text_value(&stored, self.fields.doc_id) else {
                    continue;
                };
                if excluded_doc_ids.contains(&doc_id) {
                    continue;
                }

                let Some(version_id) = text_value(&stored, self.fields.version_id) else {
                    continue;
                };
                let Some(clean_text) = text_value(&stored, self.fields.clean_text) else {
                    continue;
                };

                documents.push(IndexDocument {
                    doc_id,
                    version_id,
                    file_name: text_value(&stored, self.fields.file_name).unwrap_or_default(),
                    clean_text,
                    sections: Vec::new(),
                    is_deleted: false,
                });
            }
        }

        Ok(documents)
    }

    fn search_internal(
        &self,
        query: SearchQuery,
        allowed_doc_ids: Option<&BTreeSet<String>>,
    ) -> Result<Vec<SearchHit>> {
        if allowed_doc_ids.is_some_and(BTreeSet::is_empty) {
            return Ok(Vec::new());
        }

        self.reload()?;
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.file_name, self.fields.clean_text],
        );
        if query.text().trim().is_empty() {
            return Ok(Vec::new());
        }

        let (parsed_query, _parse_errors) = query_parser.parse_query_lenient(query.text());
        let parsed_query = match allowed_doc_ids {
            Some(doc_ids) => with_doc_id_filter(parsed_query, self.fields.doc_id, doc_ids),
            None => parsed_query,
        };
        let candidate_limit = query.limit();
        let top_docs = searcher
            .search(
                parsed_query.as_ref(),
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

fn retry_transient_index_open<T>(
    mut open: impl FnMut() -> Result<T>,
    attempts: usize,
    retry_delay: Duration,
) -> Result<T> {
    let attempts = attempts.max(1);
    for attempt in 0..attempts {
        match open() {
            Ok(value) => return Ok(value),
            Err(error) if attempt + 1 < attempts && is_transient_index_open_error(&error) => {
                if !retry_delay.is_zero() {
                    thread::sleep(retry_delay);
                }
            }
            Err(error) => return Err(error),
        }
    }

    Err(FullTextError::internal(
        "full-text index open retry exhausted",
    ))
}

fn is_transient_index_open_error(error: &FullTextError) -> bool {
    is_transient_index_operation_error(error)
}

fn retry_transient_index_mutation<T>(
    mut mutate: impl FnMut() -> Result<T>,
    attempts: usize,
    retry_delay: Duration,
) -> Result<T> {
    let attempts = attempts.max(1);
    for attempt in 0..attempts {
        match mutate() {
            Ok(value) => return Ok(value),
            Err(error) if attempt + 1 < attempts && is_transient_index_operation_error(&error) => {
                if !retry_delay.is_zero() {
                    thread::sleep(retry_delay);
                }
            }
            Err(error) => return Err(error),
        }
    }

    Err(FullTextError::internal(
        "full-text index mutation retry exhausted",
    ))
}

fn is_transient_index_operation_error(error: &FullTextError) -> bool {
    match error {
        FullTextError::Io { diagnostic } | FullTextError::Tantivy { diagnostic } => {
            is_windows_file_lock_diagnostic(diagnostic)
        }
        FullTextError::Cancelled => false,
        FullTextError::Internal { .. } => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriterThreadMode {
    Auto,
    SingleWorker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriterMergePolicy {
    Default,
    NoMerge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SnapshotWriterConfig {
    thread_mode: WriterThreadMode,
    merge_policy: WriterMergePolicy,
}

fn writer_config_for_snapshot(document_count: Option<usize>) -> SnapshotWriterConfig {
    match document_count {
        Some(count) if count <= SINGLE_WORKER_SNAPSHOT_DOCUMENT_LIMIT => SnapshotWriterConfig {
            thread_mode: WriterThreadMode::SingleWorker,
            merge_policy: WriterMergePolicy::NoMerge,
        },
        _ => SnapshotWriterConfig {
            thread_mode: WriterThreadMode::Auto,
            merge_policy: WriterMergePolicy::Default,
        },
    }
}

fn exact_size_hint(size_hint: (usize, Option<usize>)) -> Option<usize> {
    match size_hint {
        (lower, Some(upper)) if lower == upper => Some(lower),
        _ => None,
    }
}

fn with_doc_id_filter(
    parsed_query: Box<dyn Query>,
    doc_id_field: Field,
    allowed_doc_ids: &BTreeSet<String>,
) -> Box<dyn Query> {
    let doc_filter_query = if allowed_doc_ids.len() == 1 {
        let doc_id = allowed_doc_ids.iter().next().expect("non-empty doc id set");
        Box::new(TermQuery::new(
            Term::from_field_text(doc_id_field, doc_id),
            IndexRecordOption::Basic,
        )) as Box<dyn Query>
    } else {
        Box::new(BooleanQuery::new(
            allowed_doc_ids
                .iter()
                .map(|doc_id| {
                    (
                        Occur::Should,
                        Box::new(TermQuery::new(
                            Term::from_field_text(doc_id_field, doc_id),
                            IndexRecordOption::Basic,
                        )) as Box<dyn Query>,
                    )
                })
                .collect(),
        )) as Box<dyn Query>
    };

    Box::new(BooleanQuery::new(vec![
        (Occur::Must, parsed_query),
        (Occur::Must, doc_filter_query),
    ]))
}

pub fn publish_snapshot<I>(index_root: &Path, snapshot_name: &str, documents: I) -> Result<()>
where
    I: IntoIterator<Item = IndexDocument>,
{
    publish_snapshot_with_control(
        index_root,
        snapshot_name,
        documents,
        SnapshotPublishControl::disabled(),
    )
}

pub fn publish_snapshot_with_control<I>(
    index_root: &Path,
    snapshot_name: &str,
    documents: I,
    control: SnapshotPublishControl<'_>,
) -> Result<()>
where
    I: IntoIterator<Item = IndexDocument>,
{
    publish_snapshot_documents_with_control(index_root, snapshot_name, documents, control)
}

pub fn publish_trusted_redacted_snapshot_with_control<I>(
    index_root: &Path,
    snapshot_name: &str,
    documents: I,
    control: SnapshotPublishControl<'_>,
) -> Result<()>
where
    I: IntoIterator<Item = IndexDocument>,
{
    publish_snapshot_documents_with_redaction(
        index_root,
        snapshot_name,
        documents,
        control,
        IndexDocumentRedaction::TrustedRedacted,
    )
}

pub fn publish_snapshot_refs<'a, I>(
    index_root: &Path,
    snapshot_name: &str,
    documents: I,
) -> Result<()>
where
    I: IntoIterator<Item = &'a IndexDocument>,
{
    publish_snapshot_refs_with_control(
        index_root,
        snapshot_name,
        documents,
        SnapshotPublishControl::disabled(),
    )
}

pub fn publish_snapshot_refs_with_control<'a, I>(
    index_root: &Path,
    snapshot_name: &str,
    documents: I,
    control: SnapshotPublishControl<'_>,
) -> Result<()>
where
    I: IntoIterator<Item = &'a IndexDocument>,
{
    publish_snapshot_documents_with_control(index_root, snapshot_name, documents, control)
}

fn publish_snapshot_documents_with_control<I, D>(
    index_root: &Path,
    snapshot_name: &str,
    documents: I,
    control: SnapshotPublishControl<'_>,
) -> Result<()>
where
    I: IntoIterator<Item = D>,
    D: Borrow<IndexDocument>,
{
    publish_snapshot_documents_with_redaction(
        index_root,
        snapshot_name,
        documents,
        control,
        IndexDocumentRedaction::Redact,
    )
}

#[derive(Clone, Copy)]
enum IndexDocumentRedaction {
    Redact,
    TrustedRedacted,
}

fn publish_snapshot_documents_with_redaction<I, D>(
    index_root: &Path,
    snapshot_name: &str,
    documents: I,
    control: SnapshotPublishControl<'_>,
    redaction: IndexDocumentRedaction,
) -> Result<()>
where
    I: IntoIterator<Item = D>,
    D: Borrow<IndexDocument>,
{
    let (documents, staging_dir, published_dir, index) =
        measure_snapshot_publish_phase(control, SnapshotPublishPhase::Setup, || {
            validate_snapshot_name(snapshot_name)?;
            control.check()?;
            let documents = documents.into_iter();
            let writer_config = writer_config_for_snapshot(exact_size_hint(documents.size_hint()));

            let staging_root = index_root.join(STAGING_DIR);
            let snapshots_root = index_root.join(SNAPSHOTS_DIR);
            fs::create_dir_all(&staging_root).map_err(FullTextError::io)?;
            fs::create_dir_all(&snapshots_root).map_err(FullTextError::io)?;

            let staging_dir = staging_root.join(format!("{snapshot_name}.tmp"));
            if staging_dir.exists() {
                remove_snapshot_dir_all(&staging_dir)?;
            }
            let published_dir = snapshots_root.join(snapshot_name);
            if published_dir.exists() {
                return Err(FullTextError::internal("full-text snapshot already exists"));
            }

            let index = FullTextIndex::open_or_create_with_writer_config(
                &staging_dir,
                writer_config,
                control.writer_heap_bytes(),
            )?;
            Ok((documents, staging_dir, published_dir, index))
        })?;

    measure_snapshot_publish_phase(control, SnapshotPublishPhase::DocumentIndexing, || {
        index.replace_documents_with_redaction(documents, control, redaction)
    })?;
    measure_snapshot_publish_phase(control, SnapshotPublishPhase::TantivyCommit, || {
        control.check()?;
        index.commit()
    })?;
    measure_snapshot_publish_phase(control, SnapshotPublishPhase::PlaintextValidation, || {
        control.check()?;
        drop(index);
        validate_plaintext_snapshot_contents(&staging_dir)
    })?;
    measure_snapshot_publish_phase(control, SnapshotPublishPhase::EncryptedPublication, || {
        control.check()?;
        publish_encrypted_staging_snapshot(index_root, &staging_dir, &published_dir)
    })?;
    measure_snapshot_publish_phase(control, SnapshotPublishPhase::EncryptedValidation, || {
        control.check()?;
        let validation = validate_snapshot_contents(&published_dir);
        if validation.is_err() {
            let _ = fs::remove_dir_all(&published_dir);
        }
        validation
    })?;
    measure_snapshot_publish_phase(control, SnapshotPublishPhase::ActiveSnapshotWrite, || {
        control.check()?;
        write_active_snapshot(index_root, snapshot_name)
    })?;

    Ok(())
}

fn measure_snapshot_publish_phase<T>(
    control: SnapshotPublishControl<'_>,
    phase: SnapshotPublishPhase,
    operation: impl FnOnce() -> Result<T>,
) -> Result<T> {
    control.report_phase(phase);
    let started = Instant::now();
    let result = operation();
    control.report_phase_timing(phase, started.elapsed());
    result
}

pub fn publish_incremental_snapshot<I>(
    index_root: &Path,
    snapshot_name: &str,
    replacement_documents: I,
    deleted_doc_ids: &BTreeSet<String>,
) -> Result<SnapshotPublishSummary>
where
    I: IntoIterator<Item = IndexDocument>,
{
    let documents =
        incremental_snapshot_documents(index_root, replacement_documents, deleted_doc_ids)?;
    let indexed_documents = documents.len();
    publish_snapshot(index_root, snapshot_name, documents)?;

    Ok(SnapshotPublishSummary { indexed_documents })
}

pub fn incremental_snapshot_documents<I>(
    index_root: &Path,
    replacement_documents: I,
    deleted_doc_ids: &BTreeSet<String>,
) -> Result<Vec<IndexDocument>>
where
    I: IntoIterator<Item = IndexDocument>,
{
    let replacement_documents = replacement_documents.into_iter().collect::<Vec<_>>();
    let mut excluded_doc_ids = deleted_doc_ids.clone();
    for document in &replacement_documents {
        excluded_doc_ids.insert(document.doc_id.clone());
    }

    let mut documents = active_index_documents_except(index_root, &excluded_doc_ids)?;
    documents.extend(
        replacement_documents
            .into_iter()
            .filter(|document| !document.is_deleted),
    );
    documents.sort_by(|left, right| {
        left.doc_id
            .cmp(&right.doc_id)
            .then_with(|| left.version_id.cmp(&right.version_id))
    });

    Ok(documents)
}

fn active_index_documents_except(
    index_root: &Path,
    excluded_doc_ids: &BTreeSet<String>,
) -> Result<Vec<IndexDocument>> {
    let Some(index) = FullTextIndex::open_active(index_root)? else {
        return Ok(Vec::new());
    };

    index.stored_documents_except(excluded_doc_ids)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotPublishSummary {
    indexed_documents: usize,
}

impl SnapshotPublishSummary {
    pub fn indexed_documents(self) -> usize {
        self.indexed_documents
    }
}

fn publish_encrypted_staging_snapshot(
    index_root: &Path,
    staging_dir: &Path,
    published_dir: &Path,
) -> Result<()> {
    let temp_published_dir = private_snapshot_dir_path(published_dir)?;
    if temp_published_dir.exists() {
        remove_snapshot_dir_all(&temp_published_dir)?;
    }
    fs::create_dir_all(&temp_published_dir).map_err(FullTextError::io)?;
    restrict_private_dir_permissions(&temp_published_dir)?;

    let archive = snapshot_archive_bytes(staging_dir)?;
    write_encrypted_snapshot(
        &temp_published_dir.join(ENCRYPTED_SNAPSHOT_FILE),
        &index_root.join(SNAPSHOT_KEY_FILE),
        &archive,
    )?;
    write_snapshot_manifest(&temp_published_dir)?;
    remove_snapshot_dir_all(staging_dir)?;

    let publish_result = publish_staging_snapshot_with(
        &temp_published_dir,
        published_dir,
        &FsSnapshotPublisher,
        SNAPSHOT_PUBLISH_RETRY_DELAY,
    );
    if publish_result.is_err() {
        let _ = remove_snapshot_dir_all(&temp_published_dir);
    }
    publish_result
}

fn remove_snapshot_dir_all(path: &Path) -> Result<()> {
    retry_transient_snapshot_fs_operation(SNAPSHOT_PUBLISH_RETRY_DELAY, || fs::remove_dir_all(path))
        .map_err(FullTextError::io)
}

trait SnapshotPublisher {
    fn publish(&self, staging_dir: &Path, published_dir: &Path) -> std::io::Result<()>;
}

struct FsSnapshotPublisher;

impl SnapshotPublisher for FsSnapshotPublisher {
    fn publish(&self, staging_dir: &Path, published_dir: &Path) -> std::io::Result<()> {
        fs::rename(staging_dir, published_dir)
    }
}

fn publish_staging_snapshot_with<P: SnapshotPublisher>(
    staging_dir: &Path,
    published_dir: &Path,
    publisher: &P,
    retry_delay: Duration,
) -> Result<()> {
    for attempt in 0..SNAPSHOT_PUBLISH_RETRY_ATTEMPTS {
        match publisher.publish(staging_dir, published_dir) {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt + 1 < SNAPSHOT_PUBLISH_RETRY_ATTEMPTS
                    && is_transient_snapshot_publish_error(&error) =>
            {
                if !retry_delay.is_zero() {
                    thread::sleep(retry_delay);
                }
            }
            Err(error) => return Err(FullTextError::io(error)),
        }
    }

    Err(FullTextError::internal(
        "full-text snapshot publish retry exhausted",
    ))
}

fn retry_transient_snapshot_fs_operation<T>(
    retry_delay: Duration,
    mut operation: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    for attempt in 0..SNAPSHOT_PUBLISH_RETRY_ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error)
                if attempt + 1 < SNAPSHOT_PUBLISH_RETRY_ATTEMPTS
                    && is_transient_snapshot_publish_error(&error) =>
            {
                if !retry_delay.is_zero() {
                    thread::sleep(retry_delay);
                }
            }
            Err(error) => return Err(error),
        }
    }

    Err(std::io::Error::other(
        "full-text snapshot filesystem retry exhausted",
    ))
}

fn read_snapshot_file(path: &Path) -> Result<Vec<u8>> {
    read_snapshot_file_with_retry(path, |path| fs::read(path))
}

fn read_snapshot_file_with_retry(
    path: &Path,
    mut read: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    retry_transient_snapshot_fs_operation(SNAPSHOT_PUBLISH_RETRY_DELAY, || read(path))
        .map_err(FullTextError::io)
}

fn is_transient_snapshot_publish_error(error: &std::io::Error) -> bool {
    if matches!(
        error.kind(),
        ErrorKind::DirectoryNotEmpty
            | ErrorKind::Interrupted
            | ErrorKind::PermissionDenied
            | ErrorKind::WouldBlock
    ) {
        return true;
    }

    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(32 | 33 | 145)) {
        return true;
    }

    let diagnostic = error.to_string();
    is_windows_file_lock_diagnostic(&diagnostic)
}

fn is_windows_file_lock_diagnostic(diagnostic: &str) -> bool {
    let diagnostic = diagnostic.to_ascii_lowercase();
    diagnostic.contains("os error 5")
        || diagnostic.contains("os error 32")
        || diagnostic.contains("os error 33")
        || diagnostic.contains("os error 145")
        || diagnostic.contains("access is denied")
        || diagnostic.contains("directory is not empty")
        || diagnostic.contains("permission denied")
        || diagnostic.contains("being used by another process")
        || diagnostic.contains("locked a portion of the file")
}

fn validate_plaintext_snapshot_contents(snapshot_dir: &Path) -> Result<()> {
    let validation = FullTextIndex::open(snapshot_dir)?;
    validation
        .search(SearchQuery::new("diagnostic").with_limit(1))
        .map(|_| ())
}

fn validate_snapshot_contents(snapshot_dir: &Path) -> Result<()> {
    let validation = open_published_snapshot(snapshot_dir)?;
    validation
        .search(SearchQuery::new("diagnostic").with_limit(1))
        .map(|_| ())
}

fn open_published_snapshot(snapshot_dir: &Path) -> Result<FullTextIndex> {
    validate_snapshot_manifest(snapshot_dir)?;
    let encrypted_path = snapshot_dir.join(ENCRYPTED_SNAPSHOT_FILE);
    if !encrypted_path.exists() {
        return Err(FullTextError::internal(
            "full-text snapshot encrypted envelope missing",
        ));
    }

    let index_root = snapshot_dir
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| FullTextError::internal("full-text snapshot root missing"))?;
    let archive = read_encrypted_snapshot(&encrypted_path, &index_root.join(SNAPSHOT_KEY_FILE))?;
    let temp_dir = create_private_temp_dir("fulltext-snapshot")?;
    extract_snapshot_archive(&archive, temp_dir.path())?;
    let mut index = FullTextIndex::open(temp_dir.path())?;
    index._decrypted_snapshot_dir = Some(temp_dir);
    Ok(index)
}

fn write_snapshot_manifest(snapshot_dir: &Path) -> Result<()> {
    let manifest = format!(
        "{{\"schema_version\":\"{SNAPSHOT_MANIFEST_SCHEMA_VERSION}\",\"index_schema\":\"{FULLTEXT_INDEX_SCHEMA_VERSION}\",\"encrypted_snapshot\":\"{SNAPSHOT_HEADER_ENCRYPTED_V1}\"}}\n"
    );
    write_private_file(
        &snapshot_dir.join(SNAPSHOT_MANIFEST_FILE),
        manifest.as_bytes(),
    )
}

fn validate_snapshot_manifest(snapshot_dir: &Path) -> Result<()> {
    let manifest_path = snapshot_dir.join(SNAPSHOT_MANIFEST_FILE);
    let manifest = String::from_utf8(read_snapshot_file(&manifest_path)?)
        .map_err(|_| FullTextError::internal("full-text snapshot manifest corrupt"))?;

    if !manifest.contains(&format!(
        "\"schema_version\":\"{SNAPSHOT_MANIFEST_SCHEMA_VERSION}\""
    )) || !manifest.contains(&format!(
        "\"index_schema\":\"{FULLTEXT_INDEX_SCHEMA_VERSION}\""
    )) || !manifest.contains(&format!(
        "\"encrypted_snapshot\":\"{SNAPSHOT_HEADER_ENCRYPTED_V1}\""
    )) {
        return Err(FullTextError::internal(
            "full-text snapshot schema mismatch",
        ));
    }

    Ok(())
}

fn write_encrypted_snapshot(path: &Path, key_path: &Path, plaintext: &[u8]) -> Result<()> {
    let key = load_or_create_snapshot_key(key_path)?;
    let nonce = random_nonce()?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: SNAPSHOT_HEADER_ENCRYPTED_V1.as_bytes(),
            },
        )
        .map_err(|_| FullTextError::internal("full-text snapshot encryption failed"))?;

    let mut file = create_private_file(path)?;
    writeln!(file, "{SNAPSHOT_HEADER_ENCRYPTED_V1}").map_err(FullTextError::io)?;
    writeln!(file, "{}", encode_hex(&nonce)).map_err(FullTextError::io)?;
    file.write_all(&ciphertext).map_err(FullTextError::io)?;
    file.sync_all().map_err(FullTextError::io)?;
    Ok(())
}

fn read_encrypted_snapshot(path: &Path, key_path: &Path) -> Result<Vec<u8>> {
    let envelope = read_snapshot_file(path)?;
    let first_newline = envelope
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| FullTextError::internal("full-text snapshot envelope corrupt"))?;
    let second_newline = envelope[first_newline + 1..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| first_newline + 1 + offset)
        .ok_or_else(|| FullTextError::internal("full-text snapshot envelope corrupt"))?;
    let header = std::str::from_utf8(&envelope[..first_newline])
        .map_err(|_| FullTextError::internal("full-text snapshot envelope corrupt"))?;
    if header != SNAPSHOT_HEADER_ENCRYPTED_V1 {
        return Err(FullTextError::internal(
            "full-text snapshot encrypted header invalid",
        ));
    }
    let nonce_hex = std::str::from_utf8(&envelope[first_newline + 1..second_newline])
        .map_err(|_| FullTextError::internal("full-text snapshot envelope corrupt"))?;
    let nonce = decode_fixed_hex::<SNAPSHOT_NONCE_LEN>(nonce_hex)?;
    let ciphertext = &envelope[second_newline + 1..];
    let key = read_snapshot_key(key_path)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: ciphertext,
                aad: SNAPSHOT_HEADER_ENCRYPTED_V1.as_bytes(),
            },
        )
        .map_err(|_| FullTextError::internal("full-text snapshot decryption failed"))
}

fn snapshot_archive_bytes(root: &Path) -> Result<Vec<u8>> {
    let mut entries = Vec::new();
    collect_snapshot_archive_entries(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut output = Vec::new();
    output.extend_from_slice(SNAPSHOT_ARCHIVE_HEADER_V1);
    output.extend_from_slice(
        &u32::try_from(entries.len())
            .map_err(|_| FullTextError::internal("full-text snapshot archive too large"))?
            .to_be_bytes(),
    );
    for (relative_path, bytes) in entries {
        let path_bytes = relative_path.as_bytes();
        output.extend_from_slice(
            &u32::try_from(path_bytes.len())
                .map_err(|_| FullTextError::internal("full-text snapshot path too large"))?
                .to_be_bytes(),
        );
        output.extend_from_slice(path_bytes);
        output.extend_from_slice(
            &u64::try_from(bytes.len())
                .map_err(|_| FullTextError::internal("full-text snapshot file too large"))?
                .to_be_bytes(),
        );
        output.extend_from_slice(&bytes);
    }
    Ok(output)
}

fn collect_snapshot_archive_entries(
    root: &Path,
    current: &Path,
    entries: &mut Vec<(String, Vec<u8>)>,
) -> Result<()> {
    for entry in fs::read_dir(current).map_err(FullTextError::io)? {
        let entry = entry.map_err(FullTextError::io)?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(FullTextError::io)?;
        if file_type.is_dir() {
            collect_snapshot_archive_entries(root, &path, entries)?;
        } else if file_type.is_file() {
            let relative_path = archive_relative_path(root, &path)?;
            let bytes = read_snapshot_file(&path)?;
            entries.push((relative_path, bytes));
        }
    }
    Ok(())
}

fn extract_snapshot_archive(archive: &[u8], destination: &Path) -> Result<()> {
    let mut cursor = Cursor::new(archive);
    cursor.expect_prefix(SNAPSHOT_ARCHIVE_HEADER_V1)?;
    let entry_count = cursor.read_u32()?;
    for _ in 0..entry_count {
        let path_len = cursor.read_u32()? as usize;
        let path_bytes = cursor.read_bytes(path_len)?;
        let relative_path = std::str::from_utf8(path_bytes)
            .map_err(|_| FullTextError::internal("full-text snapshot archive path corrupt"))?;
        let output_path = archive_destination_path(destination, relative_path)?;
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(FullTextError::io)?;
            restrict_private_dir_permissions(parent)?;
        }
        let file_len = cursor.read_u64()?;
        let file_len = usize::try_from(file_len)
            .map_err(|_| FullTextError::internal("full-text snapshot archive file too large"))?;
        let file_bytes = cursor.read_bytes(file_len)?;
        write_private_file(&output_path, file_bytes)?;
    }
    if !cursor.is_finished() {
        return Err(FullTextError::internal(
            "full-text snapshot archive trailing bytes",
        ));
    }
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn expect_prefix(&mut self, prefix: &[u8]) -> Result<()> {
        if self.bytes.get(self.position..self.position + prefix.len()) != Some(prefix) {
            return Err(FullTextError::internal(
                "full-text snapshot archive header corrupt",
            ));
        }
        self.position += prefix.len();
        Ok(())
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_be_bytes(bytes.try_into().map_err(|_| {
            FullTextError::internal("full-text snapshot archive corrupt")
        })?))
    }

    fn read_u64(&mut self) -> Result<u64> {
        let bytes = self.read_bytes(8)?;
        Ok(u64::from_be_bytes(bytes.try_into().map_err(|_| {
            FullTextError::internal("full-text snapshot archive corrupt")
        })?))
    }

    fn read_bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .ok_or_else(|| FullTextError::internal("full-text snapshot archive corrupt"))?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or_else(|| FullTextError::internal("full-text snapshot archive truncated"))?;
        self.position = end;
        Ok(bytes)
    }

    fn is_finished(&self) -> bool {
        self.position == self.bytes.len()
    }
}

fn archive_relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| FullTextError::internal("full-text snapshot archive path invalid"))?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    FullTextError::internal("full-text snapshot archive path invalid")
                })?;
                if value.is_empty() || value.contains('/') || value.contains('\\') {
                    return Err(FullTextError::internal(
                        "full-text snapshot archive path invalid",
                    ));
                }
                parts.push(value.to_string());
            }
            _ => {
                return Err(FullTextError::internal(
                    "full-text snapshot archive path invalid",
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err(FullTextError::internal(
            "full-text snapshot archive path invalid",
        ));
    }
    Ok(parts.join("/"))
}

fn archive_destination_path(root: &Path, relative_path: &str) -> Result<PathBuf> {
    if relative_path.is_empty()
        || relative_path.starts_with('/')
        || relative_path.starts_with('\\')
        || relative_path.contains("..")
        || relative_path.contains('\\')
    {
        return Err(FullTextError::internal(
            "full-text snapshot archive path invalid",
        ));
    }
    let mut output = root.to_path_buf();
    for part in relative_path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(FullTextError::internal(
                "full-text snapshot archive path invalid",
            ));
        }
        output.push(part);
    }
    Ok(output)
}

fn load_or_create_snapshot_key(key_path: &Path) -> Result<[u8; SNAPSHOT_KEY_LEN]> {
    match read_snapshot_key(key_path) {
        Ok(key) => Ok(key),
        Err(FullTextError::Io { .. }) if !key_path.exists() => {
            let key = random_key()?;
            write_private_file(key_path, encode_hex(&key).as_bytes())?;
            Ok(key)
        }
        Err(error) => Err(error),
    }
}

fn read_snapshot_key(key_path: &Path) -> Result<[u8; SNAPSHOT_KEY_LEN]> {
    let value = fs::read_to_string(key_path).map_err(FullTextError::io)?;
    decode_fixed_hex::<SNAPSHOT_KEY_LEN>(value.trim())
}

fn random_key() -> Result<[u8; SNAPSHOT_KEY_LEN]> {
    let mut key = [0_u8; SNAPSHOT_KEY_LEN];
    getrandom::getrandom(&mut key)
        .map_err(|_| FullTextError::internal("full-text snapshot key random failed"))?;
    Ok(key)
}

fn random_nonce() -> Result<[u8; SNAPSHOT_NONCE_LEN]> {
    let mut nonce = [0_u8; SNAPSHOT_NONCE_LEN];
    getrandom::getrandom(&mut nonce)
        .map_err(|_| FullTextError::internal("full-text snapshot nonce random failed"))?;
    Ok(nonce)
}

fn private_snapshot_dir_path(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| FullTextError::internal("full-text snapshot parent missing"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| FullTextError::internal("full-text snapshot path invalid"))?;
    let mut suffix = [0_u8; 8];
    getrandom::getrandom(&mut suffix)
        .map_err(|_| FullTextError::internal("full-text snapshot random failed"))?;
    Ok(parent.join(format!(".{file_name}.tmp-{}", encode_hex(&suffix))))
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(FullTextError::io)?;
        restrict_private_dir_permissions(parent)?;
    }
    let mut file = create_private_file(path)?;
    file.write_all(bytes).map_err(FullTextError::io)?;
    file.sync_all().map_err(FullTextError::io)?;
    restrict_private_file_permissions(path)?;
    Ok(())
}

fn create_private_file(path: &Path) -> Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(FullTextError::io)?;
        restrict_private_dir_permissions(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let file = options.open(path).map_err(FullTextError::io)?;
    restrict_private_file_permissions(path)?;
    Ok(file)
}

#[cfg(unix)]
fn restrict_private_file_permissions(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path).map_err(FullTextError::io)?.permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions).map_err(FullTextError::io)
}

#[cfg(not(unix))]
fn restrict_private_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_private_dir_permissions(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path).map_err(FullTextError::io)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).map_err(FullTextError::io)
}

#[cfg(not(unix))]
fn restrict_private_dir_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

struct PrivateTempDir {
    path: PathBuf,
}

impl PrivateTempDir {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PrivateTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn create_private_temp_dir(label: &str) -> Result<PrivateTempDir> {
    for _ in 0..32 {
        let mut suffix = [0_u8; 8];
        getrandom::getrandom(&mut suffix)
            .map_err(|_| FullTextError::internal("full-text temp random failed"))?;
        let path = std::env::temp_dir().join(format!(
            "resume-ir-{label}-{}-{}",
            std::process::id(),
            encode_hex(&suffix)
        ));
        match fs::create_dir(&path) {
            Ok(()) => {
                restrict_private_dir_permissions(&path)?;
                return Ok(PrivateTempDir { path });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(FullTextError::io(error)),
        }
    }

    Err(FullTextError::internal(
        "full-text private temp directory allocation failed",
    ))
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

fn decode_fixed_hex<const N: usize>(value: &str) -> Result<[u8; N]> {
    let bytes = decode_hex(value)?;
    bytes
        .try_into()
        .map_err(|_| FullTextError::internal("full-text snapshot hex length invalid"))
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(FullTextError::internal(
            "full-text snapshot hex length invalid",
        ));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut index = 0;
    while index < value.len() {
        let byte = u8::from_str_radix(&value[index..index + 2], 16)
            .map_err(|_| FullTextError::internal("full-text snapshot hex invalid"))?;
        bytes.push(byte);
        index += 2;
    }
    Ok(bytes)
}

pub fn inspect_snapshot_root(index_root: &Path) -> Result<SnapshotRootInspection> {
    let staging_orphans = staging_orphan_count(index_root)?;
    match read_active_snapshot_pointer(index_root)? {
        ActiveSnapshotPointer::Valid(snapshot_name) => {
            let snapshot_dir = index_root.join(SNAPSHOTS_DIR).join(&snapshot_name);
            if snapshot_exists(&snapshot_dir) && snapshot_is_usable(&snapshot_dir) {
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
                state: if snapshot_exists(&snapshot_dir) {
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

pub fn purge_obsolete_snapshots(index_root: &Path) -> Result<SnapshotPurgeSummary> {
    let active_snapshot = match read_active_snapshot_pointer(index_root)? {
        ActiveSnapshotPointer::Valid(snapshot_name) => Some(snapshot_name),
        ActiveSnapshotPointer::Missing | ActiveSnapshotPointer::Invalid => None,
    };
    let snapshots_root = index_root.join(SNAPSHOTS_DIR);
    let mut removed_snapshots = 0_usize;
    match fs::read_dir(&snapshots_root) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(FullTextError::io)?;
                if !entry.file_type().map_err(FullTextError::io)?.is_dir() {
                    continue;
                }
                let snapshot_name = entry.file_name();
                let snapshot_name = snapshot_name.to_string_lossy();
                if active_snapshot.as_deref() == Some(snapshot_name.as_ref()) {
                    continue;
                }
                fs::remove_dir_all(entry.path()).map_err(FullTextError::io)?;
                removed_snapshots += 1;
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(FullTextError::io(error)),
    }

    let staging_root = index_root.join(STAGING_DIR);
    let mut removed_staging = 0_usize;
    match fs::read_dir(&staging_root) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(FullTextError::io)?;
                if !entry.file_type().map_err(FullTextError::io)?.is_dir() {
                    continue;
                }
                fs::remove_dir_all(entry.path()).map_err(FullTextError::io)?;
                removed_staging += 1;
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(FullTextError::io(error)),
    }

    Ok(SnapshotPurgeSummary {
        removed_snapshots,
        removed_staging,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotPurgeSummary {
    removed_snapshots: usize,
    removed_staging: usize,
}

impl SnapshotPurgeSummary {
    pub fn removed_snapshots(self) -> usize {
        self.removed_snapshots
    }

    pub fn removed_staging(self) -> usize {
        self.removed_staging
    }
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

enum ActiveIndexDir {
    PublishedSnapshot(PathBuf),
    LegacyRoot(PathBuf),
}

fn active_index_dir(index_root: &Path) -> Result<Option<ActiveIndexDir>> {
    let inspection = inspect_snapshot_root(index_root)?;
    match inspection.state() {
        SnapshotRootState::Ready | SnapshotRootState::Recovered => match inspection.read_target() {
            Some(SnapshotReadTarget::PublishedSnapshot) => {
                let snapshot_name = inspection
                    .fallback_snapshot()
                    .or_else(|| inspection.active_snapshot())
                    .ok_or_else(|| FullTextError::internal("full-text snapshot pointer missing"))?;
                Ok(Some(ActiveIndexDir::PublishedSnapshot(
                    index_root.join(SNAPSHOTS_DIR).join(snapshot_name),
                )))
            }
            Some(SnapshotReadTarget::LegacyRoot) => {
                Ok(Some(ActiveIndexDir::LegacyRoot(index_root.to_path_buf())))
            }
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
    match retry_transient_snapshot_fs_operation(SNAPSHOT_PUBLISH_RETRY_DELAY, || {
        fs::rename(&temp_path, &active_path)
    }) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            retry_transient_snapshot_fs_operation(SNAPSHOT_PUBLISH_RETRY_DELAY, || {
                fs::remove_file(&active_path)
            })
            .map_err(FullTextError::io)?;
            retry_transient_snapshot_fs_operation(SNAPSHOT_PUBLISH_RETRY_DELAY, || {
                fs::rename(&temp_path, &active_path)
            })
            .map_err(FullTextError::io)
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

    validate_snapshot_contents(snapshot_dir).is_ok()
}

fn snapshot_exists(snapshot_dir: &Path) -> bool {
    snapshot_dir.join(ENCRYPTED_SNAPSHOT_FILE).exists() || snapshot_dir.join("meta.json").exists()
}

fn snapshot_metadata_looks_valid(snapshot_dir: &Path) -> bool {
    validate_snapshot_manifest(snapshot_dir).is_ok()
        && encrypted_snapshot_header_looks_valid(snapshot_dir)
}

fn encrypted_snapshot_header_looks_valid(snapshot_dir: &Path) -> bool {
    let Ok(envelope) = read_snapshot_file(&snapshot_dir.join(ENCRYPTED_SNAPSHOT_FILE)) else {
        return false;
    };
    envelope
        .split(|byte| *byte == b'\n')
        .next()
        .is_some_and(|line| line == SNAPSHOT_HEADER_ENCRYPTED_V1.as_bytes())
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
            is_deleted: schema
                .get_field("is_deleted")
                .map_err(FullTextError::tantivy)?,
        })
    }
}

fn build_schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_text_field("doc_id", STRING | STORED);
    builder.add_text_field("version_id", STORED);
    builder.add_text_field("file_name", TEXT | STORED);
    builder.add_text_field("clean_text", TEXT | STORED);
    builder.add_bool_field("is_deleted", STORED);
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
    redact_contact_values_cow(text).into_owned()
}

fn redact_contact_values_cow(text: &str) -> Cow<'_, str> {
    static EMAIL_REGEX: OnceLock<Regex> = OnceLock::new();
    static PHONE_REGEX: OnceLock<Regex> = OnceLock::new();
    static COMPACT_PHONE_REGEX: OnceLock<Regex> = OnceLock::new();
    static WECHAT_REGEX: OnceLock<Regex> = OnceLock::new();
    static LOCAL_PATH_REGEX: OnceLock<Regex> = OnceLock::new();

    let mut redacted = None;
    if text.contains('@') {
        record_redaction_regex_pass();
        replace_redaction(
            &mut redacted,
            text,
            EMAIL_REGEX.get_or_init(|| {
                Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").unwrap()
            }),
            "<redacted-email>",
        );
    }
    if contains_separated_phone_signal(redacted_text(text, &redacted)) {
        record_redaction_regex_pass();
        replace_redaction(
            &mut redacted,
            text,
            PHONE_REGEX.get_or_init(|| {
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
            }),
            "<redacted-phone>",
        );
    }
    if contains_compact_phone_signal(redacted_text(text, &redacted)) {
        record_redaction_regex_pass();
        replace_redaction(
            &mut redacted,
            text,
            COMPACT_PHONE_REGEX.get_or_init(|| Regex::new(r"\+?(?:1)?\d{10}\b").unwrap()),
            "<redacted-phone>",
        );
    }
    if contains_wechat_signal(redacted_text(text, &redacted)) {
        record_redaction_regex_pass();
        replace_redaction(
            &mut redacted,
            text,
            WECHAT_REGEX.get_or_init(|| {
                Regex::new(
                    r"(?ix)\b(?:wechat|weixin|wx|微信|微信号)\s*[:：]\s*[A-Za-z][A-Za-z0-9_.-]{5,31}\b",
                )
                .unwrap()
            }),
            "<redacted-wechat>",
        );
    }
    if contains_local_path_signal(redacted_text(text, &redacted)) {
        record_redaction_regex_pass();
        replace_redaction(
            &mut redacted,
            text,
            LOCAL_PATH_REGEX.get_or_init(|| {
                Regex::new(
                    r"(?ix)
                    (?:
                        file://\S+
                        |
                        (?:~|/Users|/home|/private|/var|/tmp|[A-Z]:[\\/])\S*
                        |
                        \b[A-Z]:\\\S+
                        |
                        \S*(?:/Users/|/home/|/private|\\Users\\)\S*
                    )
                    ",
                )
                .unwrap()
            }),
            "<redacted-path>",
        );
    }
    match redacted {
        Some(value) => Cow::Owned(value),
        None => Cow::Borrowed(text),
    }
}

fn replace_redaction(
    current: &mut Option<String>,
    original: &str,
    regex: &Regex,
    replacement: &str,
) {
    if let Cow::Owned(value) = regex.replace_all(redacted_text(original, current), replacement) {
        *current = Some(value);
    }
}

fn redacted_text<'a>(original: &'a str, redacted: &'a Option<String>) -> &'a str {
    redacted.as_deref().unwrap_or(original)
}

fn contains_separated_phone_signal(text: &str) -> bool {
    let mut digits = 0_usize;
    let mut candidate_len = 0_usize;
    let mut separator_seen = false;

    for byte in text.bytes() {
        match byte {
            b'0'..=b'9' => {
                digits += 1;
                candidate_len += 1;
                if digits >= 10 && separator_seen {
                    return true;
                }
            }
            b'+' | b'(' | b')' => {
                candidate_len += 1;
                separator_seen = true;
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'.' | b'-' if candidate_len > 0 => {
                candidate_len += 1;
                separator_seen = true;
            }
            _ => {
                digits = 0;
                candidate_len = 0;
                separator_seen = false;
            }
        }

        if candidate_len > 32 {
            digits = 0;
            candidate_len = 0;
            separator_seen = false;
        }
    }

    false
}

fn contains_compact_phone_signal(text: &str) -> bool {
    let mut consecutive_digits = 0_usize;
    for byte in text.bytes() {
        if byte.is_ascii_digit() {
            consecutive_digits += 1;
            if consecutive_digits >= 10 {
                return true;
            }
        } else {
            consecutive_digits = 0;
        }
    }

    false
}

fn contains_wechat_signal(text: &str) -> bool {
    text.contains("微信")
        || contains_ascii_case_insensitive(text, b"wechat")
        || contains_ascii_case_insensitive(text, b"weixin")
        || contains_ascii_case_insensitive(text, b"wx")
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &[u8]) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn contains_local_path_signal(text: &str) -> bool {
    text.as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'/' | b'\\' | b'~'))
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
    Cancelled,
    Io { diagnostic: String },
    Tantivy { diagnostic: String },
    Internal { diagnostic: String },
}

impl FullTextError {
    fn cancelled() -> Self {
        Self::Cancelled
    }

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
            FullTextError::Cancelled => formatter.write_str("full-text index operation cancelled"),
            FullTextError::Io { .. } => formatter.write_str("full-text index IO error"),
            FullTextError::Tantivy { .. } => {
                formatter.write_str("full-text index operation failed")
            }
            FullTextError::Internal { .. } => formatter.write_str("full-text index internal error"),
        }
    }
}

impl std::error::Error for FullTextError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn contact_redaction_skips_regex_passes_when_text_has_no_match_signals() {
        REDACTION_REGEX_PASSES.store(0, Ordering::Relaxed);

        let redacted = redact_contact_values("Synthetic candidate summary without contact markers");

        assert_eq!(
            redacted,
            "Synthetic candidate summary without contact markers"
        );
        assert_eq!(
            REDACTION_REGEX_PASSES.load(Ordering::Relaxed),
            0,
            "no-match text should not run full regex replacement passes"
        );
    }

    #[test]
    fn contact_redaction_borrows_text_when_no_redaction_is_needed() {
        let text = "Synthetic candidate summary without contact markers";

        let redacted = redact_contact_values_cow(text);

        assert!(matches!(redacted, Cow::Borrowed(value) if value == text));
    }

    #[test]
    fn contact_redaction_preserves_all_supported_redaction_outputs() {
        let redacted = redact_contact_values(
            "Email: person@example.test Phone: +1 650-555-1234 Compact: 16505551234 \
             WeChat: wx_candidate01 File: file://redacted-source/resume.pdf",
        );

        assert!(redacted.contains("<redacted-email>"));
        assert_eq!(redacted.matches("<redacted-phone>").count(), 2);
        assert!(redacted.contains("<redacted-wechat>"));
        assert!(redacted.contains("<redacted-path>"));
        assert!(!redacted.contains("person@example.test"));
        assert!(!redacted.contains("650-555-1234"));
        assert!(!redacted.contains("16505551234"));
        assert!(!redacted.contains("wx_candidate01"));
        assert!(!redacted.contains("file://redacted-source/resume.pdf"));
    }

    #[test]
    fn contact_redaction_skips_phone_regexes_for_date_only_numbers() {
        REDACTION_REGEX_PASSES.store(0, Ordering::Relaxed);

        let redacted =
            redact_contact_values("Experience 2020-01 to 2024-12; led 3 projects and 2 teams");

        assert_eq!(
            redacted,
            "Experience 2020-01 to 2024-12; led 3 projects and 2 teams"
        );
        assert_eq!(
            REDACTION_REGEX_PASSES.load(Ordering::Relaxed),
            0,
            "date-like numeric text should not run phone redaction regexes"
        );
    }

    #[test]
    fn borrowed_snapshot_publish_indexes_documents_without_taking_ownership() {
        let index_root = temp_dir("borrowed-snapshot-publish");
        let documents = [IndexDocument {
            doc_id: "doc_borrowed".to_string(),
            version_id: "ver_borrowed".to_string(),
            file_name: "borrowed.pdf".to_string(),
            clean_text: "Borrowed snapshot Rust search".to_string(),
            sections: vec![IndexSection {
                section_type: "skills".to_string(),
                text: "Rust search".to_string(),
            }],
            is_deleted: false,
        }];

        publish_snapshot_refs(&index_root, "fulltext-borrowed-1-0-0", documents.iter()).unwrap();

        let index = FullTextIndex::open_active(&index_root).unwrap().unwrap();
        let hits = index.search(SearchQuery::new("Borrowed Rust")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc_id, "doc_borrowed");
        assert_eq!(documents[0].doc_id, "doc_borrowed");

        remove_dir(&index_root);
    }

    #[test]
    fn snapshot_publish_control_cancels_between_documents() {
        let index_root = temp_dir("snapshot-publish-control-cancel");
        let documents = (0..32)
            .map(|index| IndexDocument {
                doc_id: format!("doc_{index:03}"),
                version_id: format!("ver_{index:03}"),
                file_name: format!("candidate-{index:03}.pdf"),
                clean_text: format!("Candidate {index:03} Rust search"),
                sections: vec![IndexSection {
                    section_type: "skills".to_string(),
                    text: "Rust search".to_string(),
                }],
                is_deleted: false,
            })
            .collect::<Vec<_>>();
        let checks = AtomicUsize::new(0);
        let cancel_check = || checks.fetch_add(1, Ordering::SeqCst) >= 2;
        let control = SnapshotPublishControl::from_cancel_check(&cancel_check);

        let error = publish_snapshot_refs_with_control(
            &index_root,
            "fulltext-cancelled-1-0-0",
            documents.iter(),
            control,
        )
        .unwrap_err();

        assert!(matches!(error, FullTextError::Cancelled));
        assert!(checks.load(Ordering::SeqCst) >= 3);
        assert!(FullTextIndex::open_active(&index_root).unwrap().is_none());

        remove_dir(&index_root);
    }

    #[test]
    fn snapshot_publish_control_reports_publication_subphases() {
        let index_root = temp_dir("snapshot-publish-control-phases");
        let documents = [IndexDocument {
            doc_id: "doc_phases".to_string(),
            version_id: "ver_phases".to_string(),
            file_name: "phases.pdf".to_string(),
            clean_text: "Snapshot phase attribution Rust search".to_string(),
            sections: vec![IndexSection {
                section_type: "skills".to_string(),
                text: "Rust search".to_string(),
            }],
            is_deleted: false,
        }];
        let phases = Mutex::new(Vec::new());
        let cancel_check = || false;
        let phase_observer = |phase: SnapshotPublishPhase| {
            phases.lock().unwrap().push(phase.as_label().to_string());
        };
        let control = SnapshotPublishControl::from_cancel_check(&cancel_check)
            .with_phase_observer(&phase_observer);

        publish_snapshot_refs_with_control(
            &index_root,
            "fulltext-phases-1-0-0",
            documents.iter(),
            control,
        )
        .unwrap();

        let phases = phases.into_inner().unwrap();
        for expected_phase in [
            "index_publication_setup",
            "index_publication_documents",
            "index_publication_commit",
            "index_publication_plaintext_validation",
            "index_publication_encrypted_publication",
            "index_publication_encrypted_validation",
            "index_publication_active_snapshot",
        ] {
            assert!(
                phases.iter().any(|phase| phase == expected_phase),
                "missing {expected_phase} in {phases:?}"
            );
        }

        remove_dir(&index_root);
    }

    #[test]
    fn snapshot_publish_control_reports_publication_phase_timings() {
        let index_root = temp_dir("snapshot-publish-control-phase-timings");
        let documents = [IndexDocument {
            doc_id: "doc_phase_timings".to_string(),
            version_id: "ver_phase_timings".to_string(),
            file_name: "phase-timings.pdf".to_string(),
            clean_text: "Snapshot phase timing Rust search".to_string(),
            sections: vec![IndexSection {
                section_type: "skills".to_string(),
                text: "Rust search".to_string(),
            }],
            is_deleted: false,
        }];
        let timings = Mutex::new(Vec::new());
        let phase_timing_observer = |phase: SnapshotPublishPhase, elapsed: Duration| {
            timings
                .lock()
                .unwrap()
                .push((phase.as_label().to_string(), elapsed));
        };
        let control =
            SnapshotPublishControl::disabled().with_phase_timing_observer(&phase_timing_observer);

        publish_snapshot_refs_with_control(
            &index_root,
            "fulltext-phase-timings-1-0-0",
            documents.iter(),
            control,
        )
        .unwrap();

        let timings = timings.into_inner().unwrap();
        for expected_phase in [
            "index_publication_setup",
            "index_publication_documents",
            "index_publication_commit",
            "index_publication_plaintext_validation",
            "index_publication_encrypted_publication",
            "index_publication_encrypted_validation",
            "index_publication_active_snapshot",
        ] {
            assert!(
                timings
                    .iter()
                    .any(|(phase, elapsed)| phase == expected_phase && *elapsed >= Duration::ZERO),
                "missing timing for {expected_phase} in {timings:?}"
            );
        }

        remove_dir(&index_root);
    }

    #[test]
    fn trusted_redacted_snapshot_publish_skips_redundant_redaction_passes() {
        let index_root = temp_dir("trusted-redacted-snapshot-publish");
        let documents = vec![IndexDocument {
            doc_id: "doc_trusted_redacted".to_string(),
            version_id: "ver_trusted_redacted".to_string(),
            file_name: "<redacted-email> resume.pdf".to_string(),
            clean_text: "Email <redacted-email> Phone <redacted-phone> File <redacted-path> Rust"
                .to_string(),
            sections: Vec::new(),
            is_deleted: false,
        }];
        REDACTION_REGEX_PASSES.store(0, Ordering::Relaxed);

        publish_trusted_redacted_snapshot_with_control(
            &index_root,
            "trusted-redacted-1-0-0",
            documents,
            SnapshotPublishControl::disabled(),
        )
        .unwrap();

        assert_eq!(
            REDACTION_REGEX_PASSES.load(Ordering::Relaxed),
            0,
            "trusted-redacted snapshot publish should not rerun contact redaction regexes"
        );

        remove_dir(&index_root);
    }

    #[test]
    fn staged_import_snapshot_writer_mode_uses_single_worker_for_milestones() {
        assert_eq!(
            writer_config_for_snapshot(Some(1)).thread_mode,
            WriterThreadMode::SingleWorker
        );
        assert_eq!(
            writer_config_for_snapshot(Some(100)).thread_mode,
            WriterThreadMode::SingleWorker
        );
        assert_eq!(
            writer_config_for_snapshot(Some(1_000)).thread_mode,
            WriterThreadMode::SingleWorker
        );
        assert_eq!(
            writer_config_for_snapshot(Some(1_200)).thread_mode,
            WriterThreadMode::SingleWorker
        );
        assert_eq!(
            writer_config_for_snapshot(Some(8_248)).thread_mode,
            WriterThreadMode::SingleWorker
        );
        assert_eq!(
            writer_config_for_snapshot(Some(10_000)).thread_mode,
            WriterThreadMode::SingleWorker
        );
        assert_eq!(
            writer_config_for_snapshot(Some(10_001)).thread_mode,
            WriterThreadMode::Auto
        );
        assert_eq!(
            writer_config_for_snapshot(None).thread_mode,
            WriterThreadMode::Auto
        );
    }

    #[test]
    fn staged_import_snapshot_writer_config_uses_single_worker_without_commit_merges() {
        let config = writer_config_for_snapshot(Some(8_248));

        assert_eq!(config.thread_mode, WriterThreadMode::SingleWorker);
        assert_eq!(config.merge_policy, WriterMergePolicy::NoMerge);
    }

    #[test]
    fn fulltext_schema_keeps_metadata_out_of_unused_columnar_indexes() {
        use tantivy::schema::FieldType;

        let schema = build_schema();
        let doc_id = schema.get_field("doc_id").unwrap();
        let version_id = schema.get_field("version_id").unwrap();
        let is_deleted = schema.get_field("is_deleted").unwrap();

        match schema.get_field_entry(doc_id).field_type() {
            FieldType::Str(options) => {
                assert!(options.is_stored());
                assert!(options.get_indexing_options().is_some());
                assert!(!options.is_fast());
            }
            other => panic!("doc_id should be a string field, got {other:?}"),
        }
        match schema.get_field_entry(version_id).field_type() {
            FieldType::Str(options) => {
                assert!(options.is_stored());
                assert!(options.get_indexing_options().is_none());
                assert!(!options.is_fast());
            }
            other => panic!("version_id should be a string field, got {other:?}"),
        }
        match schema.get_field_entry(is_deleted).field_type() {
            FieldType::Bool(options) => {
                assert!(options.is_stored());
                assert!(!options.is_indexed());
                assert!(!options.is_fast());
            }
            other => panic!("is_deleted should be a bool field, got {other:?}"),
        }
    }

    #[test]
    fn snapshot_publish_retries_transient_windows_rename_lock() {
        let index_root = temp_dir("retry-publish");
        let staging_dir = index_root.join("staging").join("fulltext-retry.tmp");
        let published_dir = index_root.join("snapshots").join("fulltext-retry");
        fs::create_dir_all(&staging_dir).unwrap();
        fs::create_dir_all(published_dir.parent().unwrap()).unwrap();
        fs::write(staging_dir.join("meta.json"), b"{}").unwrap();

        let publisher = TransientLockPublisher::new(2);
        publish_staging_snapshot_with(
            &staging_dir,
            &published_dir,
            &publisher,
            std::time::Duration::ZERO,
        )
        .unwrap();

        assert_eq!(publisher.attempts(), 3);
        assert!(published_dir.join("meta.json").exists());
        assert!(!staging_dir.exists());

        remove_dir(&index_root);
    }

    #[test]
    fn snapshot_publish_does_not_retry_existing_destination() {
        let index_root = temp_dir("already-exists-publish");
        let staging_dir = index_root.join("staging").join("fulltext-exists.tmp");
        let published_dir = index_root.join("snapshots").join("fulltext-exists");
        fs::create_dir_all(&staging_dir).unwrap();
        fs::create_dir_all(&published_dir).unwrap();

        let publisher = ExistingDestinationPublisher::default();
        let error = publish_staging_snapshot_with(
            &staging_dir,
            &published_dir,
            &publisher,
            std::time::Duration::ZERO,
        )
        .unwrap_err();

        assert_eq!(publisher.attempts(), 1);
        assert!(matches!(error, FullTextError::Io { .. }));

        remove_dir(&index_root);
    }

    #[test]
    fn snapshot_file_read_retries_transient_windows_lock_violation() {
        let index_root = temp_dir("retry-snapshot-file-read");
        let payload_path = index_root.join("payload.bin");
        fs::write(&payload_path, b"snapshot payload").unwrap();

        let mut attempts = 0_usize;
        let bytes = read_snapshot_file_with_retry(&payload_path, |path| {
            attempts += 1;
            if attempts < 3 {
                return Err(std::io::Error::other(
                    "The process cannot access the file because another process has locked a portion of the file. (os error 33)",
                ));
            }
            fs::read(path)
        })
        .unwrap();

        assert_eq!(bytes, b"snapshot payload");
        assert_eq!(attempts, 3);

        remove_dir(&index_root);
    }

    #[test]
    fn index_open_retries_transient_windows_access_denied() {
        let mut attempts = 0_usize;

        let opened = retry_transient_index_open(
            || {
                attempts += 1;
                if attempts < 3 {
                    return Err(FullTextError::Tantivy {
                        diagnostic: "An IO error occurred: 'Access is denied. (os error 5)'"
                            .to_string(),
                    });
                }
                Ok("opened")
            },
            4,
            std::time::Duration::ZERO,
        )
        .unwrap();

        assert_eq!(opened, "opened");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn index_open_retries_transient_windows_share_violation() {
        let mut attempts = 0_usize;

        let opened = retry_transient_index_open(
            || {
                attempts += 1;
                if attempts < 3 {
                    let diagnostic = concat!(
                        "An IO error occurred: 'The process cannot access the file because it ",
                        "is being used by another process. (os error 32)'"
                    );
                    return Err(FullTextError::Tantivy {
                        diagnostic: diagnostic.to_string(),
                    });
                }
                Ok("opened")
            },
            4,
            std::time::Duration::ZERO,
        )
        .unwrap();

        assert_eq!(opened, "opened");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn index_mutation_retries_transient_windows_access_denied() {
        let mut attempts = 0_usize;

        let committed = retry_transient_index_mutation(
            || {
                attempts += 1;
                if attempts < 3 {
                    return Err(FullTextError::Tantivy {
                        diagnostic: "An IO error occurred: 'Access is denied. (os error 5)'"
                            .to_string(),
                    });
                }
                Ok("committed")
            },
            4,
            std::time::Duration::ZERO,
        )
        .unwrap();

        assert_eq!(committed, "committed");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn transient_snapshot_fs_operation_retries_permission_denied() {
        let mut attempts = 0_usize;

        let result = retry_transient_snapshot_fs_operation(std::time::Duration::ZERO, || {
            attempts += 1;
            if attempts < 3 {
                return Err(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "fixture transient Windows file lock",
                ));
            }
            Ok("removed")
        })
        .unwrap();

        assert_eq!(result, "removed");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn transient_snapshot_fs_operation_retries_windows_lock_violation() {
        let mut attempts = 0_usize;

        let result = retry_transient_snapshot_fs_operation(std::time::Duration::ZERO, || {
            attempts += 1;
            if attempts < 3 {
                return Err(std::io::Error::other(
                    "The process cannot access the file because another process has locked a portion of the file. (os error 33)",
                ));
            }
            Ok("published")
        })
        .unwrap();

        assert_eq!(result, "published");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn transient_snapshot_fs_operation_retries_windows_directory_not_empty() {
        let mut attempts = 0_usize;

        let result = retry_transient_snapshot_fs_operation(std::time::Duration::ZERO, || {
            attempts += 1;
            if attempts < 3 {
                return Err(std::io::Error::new(
                    ErrorKind::DirectoryNotEmpty,
                    "The directory is not empty. (os error 145)",
                ));
            }
            Ok("removed")
        })
        .unwrap();

        assert_eq!(result, "removed");
        assert_eq!(attempts, 3);
    }

    #[test]
    fn transient_snapshot_fs_operation_retries_extended_windows_lock_release() {
        let mut attempts = 0_usize;

        let result = retry_transient_snapshot_fs_operation(std::time::Duration::ZERO, || {
            attempts += 1;
            if attempts <= 8 {
                return Err(std::io::Error::other(
                    "The process cannot access the file because another process has locked a portion of the file. (os error 33)",
                ));
            }
            Ok("removed")
        })
        .unwrap();

        assert_eq!(result, "removed");
        assert_eq!(attempts, 9);
    }

    struct TransientLockPublisher {
        remaining_failures: Mutex<usize>,
        attempts: Mutex<usize>,
    }

    impl TransientLockPublisher {
        fn new(failures: usize) -> Self {
            Self {
                remaining_failures: Mutex::new(failures),
                attempts: Mutex::new(0),
            }
        }

        fn attempts(&self) -> usize {
            *self.attempts.lock().unwrap()
        }
    }

    impl SnapshotPublisher for TransientLockPublisher {
        fn publish(&self, staging_dir: &Path, published_dir: &Path) -> std::io::Result<()> {
            *self.attempts.lock().unwrap() += 1;
            let mut remaining_failures = self.remaining_failures.lock().unwrap();
            if *remaining_failures > 0 {
                *remaining_failures -= 1;
                return Err(std::io::Error::new(
                    ErrorKind::PermissionDenied,
                    "fixture transient lock",
                ));
            }
            fs::rename(staging_dir, published_dir)
        }
    }

    #[derive(Default)]
    struct ExistingDestinationPublisher {
        attempts: Mutex<usize>,
    }

    impl ExistingDestinationPublisher {
        fn attempts(&self) -> usize {
            *self.attempts.lock().unwrap()
        }
    }

    impl SnapshotPublisher for ExistingDestinationPublisher {
        fn publish(&self, _staging_dir: &Path, _published_dir: &Path) -> std::io::Result<()> {
            *self.attempts.lock().unwrap() += 1;
            Err(std::io::Error::new(ErrorKind::AlreadyExists, "exists"))
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("resume-ir-index-unit-{label}-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn remove_dir(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }
}
