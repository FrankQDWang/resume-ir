pub fn crate_name() -> &'static str {
    "index-fulltext"
}

use std::borrow::{Borrow, Cow};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use core_domain::{ContentDigest, SearchProjectionDigest};
use privacy::redact_contact_values;
use tantivy::collector::TopDocs;
use tantivy::indexer::NoMergePolicy;
use tantivy::query::{BooleanQuery, Occur, Query, QueryParser, TermQuery};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TantivyDocument, Value, STORED, STRING, TEXT,
};
use tantivy::{Index, IndexReader, IndexWriter, Term};

mod manifest;
mod purge_artifact;
mod snapshot_gc;
mod snapshot_generation;
mod snapshot_identity;
mod snapshot_path_identity;

use manifest::{
    decode_manifest, encode_manifest, ManifestError, FULLTEXT_INDEX_SCHEMA_VERSION,
    MAX_MANIFEST_BYTES, SNAPSHOT_HEADER_ENCRYPTED_V2,
};
pub use manifest::{
    FullTextSnapshotSchema, PublishedSnapshotMetadata, FULLTEXT_SNAPSHOT_SCHEMA_V2,
};
pub use purge_artifact::{classify_purge_artifact, FullTextPurgeArtifactClass};
pub use snapshot_gc::{
    commit_snapshot_gc, prepare_snapshot_gc, try_acquire_snapshot_gc,
    FullTextSnapshotGcAcquisition, FullTextSnapshotGcCommitReport, FullTextSnapshotGcFailureClass,
    FullTextSnapshotGcFailurePhase, FullTextSnapshotGcPreparation, PreparedFullTextSnapshotGc,
    SnapshotPurgePartialFailure, SnapshotPurgeSummary,
};
use snapshot_generation::{
    create_generation_pin, remove_generation_pin, SnapshotGenerationReadLease, GENERATION_PINS_DIR,
};
use snapshot_path_identity::PinnedSnapshotDirectory;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;

const DEFAULT_WRITER_HEAP_BYTES: usize = 50_000_000;
const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 100;
const SNAPSHOTS_DIR: &str = "snapshots";
const STAGING_DIR: &str = "staging";
const SNAPSHOT_READER_LOCK_FILE: &str = "snapshot-readers.lock";
const SNAPSHOT_PUBLICATION_LOCK_FILE: &str = "snapshot-publication.lock";
const ENCRYPTED_SNAPSHOT_FILE: &str = "fulltext.snapshot.enc";
const SNAPSHOT_MANIFEST_FILE: &str = "snapshot-manifest.json";
const SNAPSHOT_KEY_FILE: &str = "fulltext.snapshot.key-v2";
const SNAPSHOT_ARCHIVE_HEADER_V2: &[u8] = b"resume-ir-fulltext-snapshot-archive-v2\n";
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
#[cfg(windows)]
const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x0000_0007;
const SNAPSHOT_KEY_LEN: usize = 32;
const ENCODED_SNAPSHOT_KEY_LEN: usize = SNAPSHOT_KEY_LEN * 2;
const MAX_SNAPSHOT_KEY_FILE_BYTES: usize = ENCODED_SNAPSHOT_KEY_LEN + 1;
const SNAPSHOT_NONCE_LEN: usize = 24;
const STABLE_ID_DIGEST_LEN: usize = 32;
const MAX_SNAPSHOT_NAME_BYTES: usize = 160;
const SNAPSHOT_PUBLISH_RETRY_ATTEMPTS: usize = 100;
const SNAPSHOT_PUBLISH_RETRY_DELAY: Duration = Duration::from_millis(50);
const INDEX_OPEN_RETRY_ATTEMPTS: usize = 20;
const INDEX_OPEN_RETRY_DELAY: Duration = Duration::from_millis(50);
const INDEX_MUTATION_RETRY_ATTEMPTS: usize = 20;
const INDEX_MUTATION_RETRY_DELAY: Duration = Duration::from_millis(50);
const SINGLE_WORKER_SNAPSHOT_DOCUMENT_LIMIT: usize = 10_000;
const SNAPSHOT_PUBLISH_CONTROL_DOCUMENT_INTERVAL: usize = 8;

#[derive(Clone, PartialEq, Eq)]
pub struct IndexDocument {
    pub doc_id: String,
    pub resume_version_id: String,
    pub file_name: String,
    pub clean_text: String,
    pub sections: Vec<IndexSection>,
}

impl fmt::Debug for IndexDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndexDocument")
            .field("doc_id", &self.doc_id)
            .field("resume_version_id", &self.resume_version_id)
            .field("file_name", &"<redacted>")
            .field("clean_text", &"<redacted>")
            .field("section_count", &self.sections.len())
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
    pub resume_version_id: String,
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
            .field("resume_version_id", &self.resume_version_id)
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
    AtomicPublication,
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
            Self::AtomicPublication => "index_publication_atomic_commit",
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
    snapshot_metadata: Option<PublishedSnapshotMetadata>,
    exact_identity_pairs: Option<Vec<(String, String)>>,
    _snapshot_generation_lease: Option<SnapshotGenerationReadLease>,
    _decrypted_snapshot_dir: Option<PrivateTempDir>,
}

/// Cross-process lease fencing metadata selection and immutable snapshot reads
/// from physical generation reclamation.
///
/// Acquire this lease before reading publication metadata, then transfer it to
/// [`FullTextIndex::open_snapshot_with_lease`]. Exact open swaps this root-wide
/// acquisition fence for a generation-scoped read pin. A missing lease file
/// means no fenced snapshot has been published yet; acquisition returns `None`
/// without creating query-path runtime state.
pub struct SnapshotReadLease {
    file: File,
    index_root: PathBuf,
    root_identity: PinnedSnapshotDirectory,
    snapshots_identity: PinnedSnapshotDirectory,
    pins_identity: PinnedSnapshotDirectory,
}

impl SnapshotReadLease {
    pub fn acquire(index_root: &Path) -> Result<Option<Self>> {
        match fs::symlink_metadata(index_root) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(FullTextError::io(error)),
        }
        let index_root = fs::canonicalize(index_root).map_err(FullTextError::io)?;
        let root_identity = PinnedSnapshotDirectory::acquire(&index_root)?;
        let Some(file) = open_existing_snapshot_reader_lock(&index_root)? else {
            return Ok(None);
        };
        let snapshots_identity = PinnedSnapshotDirectory::acquire(&index_root.join(SNAPSHOTS_DIR))?;
        let pins_identity =
            PinnedSnapshotDirectory::acquire(&index_root.join(GENERATION_PINS_DIR))?;
        file.lock_shared().map_err(FullTextError::io)?;
        let lease = Self {
            file,
            index_root,
            root_identity,
            snapshots_identity,
            pins_identity,
        };
        lease.validate_layout()?;
        Ok(Some(lease))
    }

    fn protects(&self, index_root: &Path) -> Result<bool> {
        if self.index_root != fs::canonicalize(index_root).map_err(FullTextError::io)? {
            return Ok(false);
        }
        self.validate_layout()?;
        Ok(true)
    }

    fn validate_layout(&self) -> Result<()> {
        self.root_identity.validate_current()?;
        self.snapshots_identity.validate_current()?;
        self.pins_identity.validate_current()
    }
}

impl Drop for SnapshotReadLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

struct SnapshotGcLease {
    file: File,
    root_identity: PinnedSnapshotDirectory,
    snapshots_identity: PinnedSnapshotDirectory,
    staging_identity: PinnedSnapshotDirectory,
    pins_identity: PinnedSnapshotDirectory,
}

struct SnapshotPublicationLease {
    file: File,
    index_root: PathBuf,
    root_identity: PinnedSnapshotDirectory,
    snapshots_identity: Option<PinnedSnapshotDirectory>,
    staging_identity: Option<PinnedSnapshotDirectory>,
    pins_identity: Option<PinnedSnapshotDirectory>,
}

impl SnapshotPublicationLease {
    fn acquire(index_root: &Path) -> Result<Self> {
        let index_root = ensure_canonical_index_root(index_root)?;
        let root_identity = PinnedSnapshotDirectory::acquire(&index_root)?;
        let file = open_or_create_private_lock(&index_root.join(SNAPSHOT_PUBLICATION_LOCK_FILE))?;
        file.lock().map_err(FullTextError::io)?;
        let lease = Self {
            file,
            index_root,
            root_identity,
            snapshots_identity: None,
            staging_identity: None,
            pins_identity: None,
        };
        lease.root_identity.validate_current()?;
        Ok(lease)
    }

    fn index_root(&self) -> &Path {
        &self.index_root
    }

    fn try_acquire_existing(index_root: &Path) -> Result<Option<Self>> {
        let index_root = fs::canonicalize(index_root).map_err(FullTextError::io)?;
        let root_identity = PinnedSnapshotDirectory::acquire(&index_root)?;
        let snapshots_identity = PinnedSnapshotDirectory::acquire(&index_root.join(SNAPSHOTS_DIR))?;
        let staging_identity = PinnedSnapshotDirectory::acquire(&index_root.join(STAGING_DIR))?;
        let pins_identity =
            PinnedSnapshotDirectory::acquire(&index_root.join(GENERATION_PINS_DIR))?;
        let file = open_existing_private_lock(&index_root.join(SNAPSHOT_PUBLICATION_LOCK_FILE))?
            .ok_or_else(|| {
                FullTextError::internal("full-text snapshot publication lock missing")
            })?;
        if !try_exclusive_file_lock(&file)? {
            return Ok(None);
        }
        let lease = Self {
            file,
            index_root,
            root_identity,
            snapshots_identity: Some(snapshots_identity),
            staging_identity: Some(staging_identity),
            pins_identity: Some(pins_identity),
        };
        lease.validate_layout()?;
        Ok(Some(lease))
    }

    fn pin_layout(&mut self) -> Result<()> {
        self.root_identity.validate_current()?;
        self.snapshots_identity = Some(PinnedSnapshotDirectory::acquire(
            &self.index_root.join(SNAPSHOTS_DIR),
        )?);
        self.staging_identity = Some(PinnedSnapshotDirectory::acquire(
            &self.index_root.join(STAGING_DIR),
        )?);
        self.pins_identity = Some(PinnedSnapshotDirectory::acquire(
            &self.index_root.join(GENERATION_PINS_DIR),
        )?);
        self.validate_layout()
    }

    fn validate_layout(&self) -> Result<()> {
        self.root_identity.validate_current()?;
        self.snapshots_identity
            .as_ref()
            .ok_or_else(|| FullTextError::internal("full-text snapshot layout is not pinned"))?
            .validate_current()?;
        self.staging_identity
            .as_ref()
            .ok_or_else(|| FullTextError::internal("full-text staging layout is not pinned"))?
            .validate_current()?;
        self.pins_identity
            .as_ref()
            .ok_or_else(|| FullTextError::internal("full-text pin layout is not pinned"))?
            .validate_current()
    }
}

impl Drop for SnapshotPublicationLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl SnapshotGcLease {
    fn try_acquire(index_root: &Path) -> Result<Option<Self>> {
        let root_identity = PinnedSnapshotDirectory::acquire(index_root)?;
        let snapshots_identity = PinnedSnapshotDirectory::acquire(&index_root.join(SNAPSHOTS_DIR))?;
        let staging_identity = PinnedSnapshotDirectory::acquire(&index_root.join(STAGING_DIR))?;
        let pins_identity =
            PinnedSnapshotDirectory::acquire(&index_root.join(GENERATION_PINS_DIR))?;
        let file = open_existing_snapshot_reader_lock(index_root)?.ok_or_else(|| {
            FullTextError::internal("full-text snapshot root-acquisition lock missing")
        })?;
        if !try_exclusive_file_lock(&file)? {
            return Ok(None);
        }
        let lease = Self {
            file,
            root_identity,
            snapshots_identity,
            staging_identity,
            pins_identity,
        };
        lease.validate_layout()?;
        Ok(Some(lease))
    }

    fn validate_layout(&self) -> Result<()> {
        self.root_identity.validate_current()?;
        self.snapshots_identity.validate_current()?;
        self.staging_identity.validate_current()?;
        self.pins_identity.validate_current()
    }

    fn same_layout_as(&self, publication: &SnapshotPublicationLease) -> bool {
        let Some(publication_snapshots) = publication.snapshots_identity.as_ref() else {
            return false;
        };
        let Some(publication_staging) = publication.staging_identity.as_ref() else {
            return false;
        };
        let Some(publication_pins) = publication.pins_identity.as_ref() else {
            return false;
        };
        self.root_identity.same_identity(&publication.root_identity)
            && self.snapshots_identity.same_identity(publication_snapshots)
            && self.staging_identity.same_identity(publication_staging)
            && self.pins_identity.same_identity(publication_pins)
    }
}

impl Drop for SnapshotGcLease {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn try_exclusive_file_lock(file: &File) -> Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(error)) => Err(FullTextError::io(error)),
    }
}

fn create_snapshot_reader_lock(index_root: &Path) -> Result<File> {
    fs::create_dir_all(index_root).map_err(FullTextError::io)?;
    open_or_create_private_lock(&index_root.join(SNAPSHOT_READER_LOCK_FILE))
}

fn open_existing_snapshot_reader_lock(index_root: &Path) -> Result<Option<File>> {
    open_existing_private_lock(&index_root.join(SNAPSHOT_READER_LOCK_FILE))
}

fn open_or_create_private_lock(path: &Path) -> Result<File> {
    let existed = validate_existing_private_lock(path)?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_WRITE_THROUGH);
    let file = options.open(path).map_err(FullTextError::io)?;
    validate_open_private_lock(path, &file)?;
    if !existed {
        file.sync_all().map_err(FullTextError::io)?;
        let parent = path
            .parent()
            .ok_or_else(|| FullTextError::internal("full-text lock parent missing"))?;
        sync_directory(parent)?;
    }
    Ok(file)
}

fn open_existing_private_lock(path: &Path) -> Result<Option<File>> {
    if !validate_existing_private_lock(path)? {
        return Ok(None);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(FullTextError::io)?;
    validate_open_private_lock(path, &file)?;
    Ok(Some(file))
}

fn validate_existing_private_lock(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_private_lock_metadata(&metadata)?;
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(FullTextError::io(error)),
    }
}

fn validate_open_private_lock(path: &Path, file: &File) -> Result<()> {
    let opened = file.metadata().map_err(FullTextError::io)?;
    validate_private_lock_metadata(&opened)?;
    let current = fs::symlink_metadata(path).map_err(FullTextError::io)?;
    validate_private_lock_metadata(&current)?;
    if !same_open_file_identity(file, path, &opened, &current).map_err(FullTextError::io)? {
        return Err(FullTextError::internal(
            "full-text lock file identity changed during open",
        ));
    }
    Ok(())
}

fn validate_private_lock_metadata(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(FullTextError::internal(
            "full-text lock path must be a regular non-symlink file",
        ));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(FullTextError::internal(
            "full-text lock file permissions must be owner-only read-write",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn same_open_file_identity(
    _file: &File,
    _path: &Path,
    opened: &fs::Metadata,
    current: &fs::Metadata,
) -> std::io::Result<bool> {
    Ok(opened.dev() == current.dev() && opened.ino() == current.ino())
}

#[cfg(windows)]
fn same_open_file_identity(
    file: &File,
    path: &Path,
    _opened: &fs::Metadata,
    _current: &fs::Metadata,
) -> std::io::Result<bool> {
    let opened = same_file::Handle::from_file(file.try_clone()?)?;
    let current = same_file::Handle::from_path(path)?;
    let final_metadata = fs::symlink_metadata(path)?;
    if !final_metadata.file_type().is_file() || final_metadata.file_type().is_symlink() {
        return Ok(false);
    }
    Ok(opened == current)
}

#[cfg(not(any(unix, windows)))]
fn same_open_file_identity(
    _file: &File,
    _path: &Path,
    _opened: &fs::Metadata,
    _current: &fs::Metadata,
) -> std::io::Result<bool> {
    Ok(false)
}

impl FullTextIndex {
    fn open(index_dir: &Path) -> Result<Self> {
        retry_transient_index_open(
            || Self::open_once(index_dir),
            INDEX_OPEN_RETRY_ATTEMPTS,
            INDEX_OPEN_RETRY_DELAY,
        )
    }

    fn open_once(index_dir: &Path) -> Result<Self> {
        let index = Index::open_in_dir(index_dir).map_err(FullTextError::tantivy)?;
        let schema = index.schema();
        validate_index_schema(&schema)?;
        let fields = IndexFields::from_schema(&schema)?;
        let reader = index.reader().map_err(FullTextError::tantivy)?;

        Ok(Self {
            index,
            reader,
            writer: None,
            fields,
            snapshot_metadata: None,
            exact_identity_pairs: None,
            _snapshot_generation_lease: None,
            _decrypted_snapshot_dir: None,
        })
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
        validate_index_schema(&schema)?;
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
            snapshot_metadata: None,
            exact_identity_pairs: None,
            _snapshot_generation_lease: None,
            _decrypted_snapshot_dir: None,
        })
    }

    /// Opens one immutable published generation by its database-owned token.
    ///
    /// Callers must supply the database-owned Ready generation; the filesystem
    /// never selects a generation on their behalf.
    fn open_snapshot(index_root: &Path, snapshot_name: &str) -> Result<Option<Self>> {
        validate_snapshot_name(snapshot_name)?;
        let Some(lease) = SnapshotReadLease::acquire(index_root)? else {
            return Ok(None);
        };
        Self::open_snapshot_with_lease(index_root, snapshot_name, lease)
    }

    /// Opens an exact generation while adopting a lease acquired before the
    /// caller's publication-metadata transaction.
    pub fn open_snapshot_with_lease(
        index_root: &Path,
        snapshot_name: &str,
        lease: SnapshotReadLease,
    ) -> Result<Option<Self>> {
        validate_snapshot_name(snapshot_name)?;
        if !lease.protects(index_root)? {
            return Err(FullTextError::internal(
                "full-text snapshot lease belongs to another index root",
            ));
        }
        let pinned_root = lease.index_root.clone();
        lease.validate_layout()?;
        let snapshot_dir = pinned_root.join(SNAPSHOTS_DIR).join(snapshot_name);
        if !snapshot_directory_exists(&snapshot_dir)? {
            return Ok(None);
        }
        let generation_identity = PinnedSnapshotDirectory::acquire(&snapshot_dir)?;
        let generation_lease = SnapshotGenerationReadLease::acquire(&pinned_root, snapshot_name)?;
        lease.validate_layout()?;
        generation_identity.validate_current()?;
        let mut index = open_published_snapshot(&pinned_root, &snapshot_dir, snapshot_name)?;
        lease.validate_layout()?;
        generation_identity.validate_current()?;
        drop(lease);
        index._snapshot_generation_lease = Some(generation_lease);
        Ok(Some(index))
    }

    pub fn snapshot_metadata(&self) -> Option<&PublishedSnapshotMetadata> {
        self.snapshot_metadata.as_ref()
    }

    fn replace_documents_with_redaction<I, D>(
        &self,
        documents: I,
        control: SnapshotPublishControl<'_>,
        redaction: IndexDocumentRedaction,
    ) -> Result<usize>
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

        let mut document_ids = BTreeSet::new();
        let mut resume_version_ids = BTreeSet::new();
        let mut indexed_documents = 0_usize;
        for (index, document) in documents.into_iter().enumerate() {
            control.check_after_document(index)?;
            let document = document.borrow();
            validate_stable_id(&document.doc_id, "doc_", "document")?;
            validate_stable_id(&document.resume_version_id, "ver_", "resume version")?;
            if !document_ids.insert(document.doc_id.clone()) {
                return Err(FullTextError::internal(
                    "full-text snapshot contains duplicate document identity",
                ));
            }
            if !resume_version_ids.insert(document.resume_version_id.clone()) {
                return Err(FullTextError::internal(
                    "full-text snapshot contains duplicate resume version identity",
                ));
            }
            let (file_name, clean_text) = match redaction {
                IndexDocumentRedaction::Redact => (
                    redact_contact_values(&document.file_name),
                    redact_contact_values(&document.clean_text),
                ),
                IndexDocumentRedaction::TrustedRedacted => (
                    Cow::Borrowed(document.file_name.as_str()),
                    Cow::Borrowed(document.clean_text.as_str()),
                ),
            };
            let mut tantivy_document = TantivyDocument::default();
            tantivy_document.add_text(self.fields.doc_id, &document.doc_id);
            tantivy_document.add_text(self.fields.resume_version_id, &document.resume_version_id);
            tantivy_document.add_text(self.fields.file_name, file_name.as_ref());
            tantivy_document.add_text(self.fields.clean_text, clean_text.as_ref());
            writer
                .add_document(tantivy_document)
                .map_err(FullTextError::tantivy)?;
            indexed_documents += 1;
        }
        control.check()?;

        Ok(indexed_documents)
    }

    fn commit(&self) -> Result<()> {
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

    fn reload(&self) -> Result<()> {
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
                let doc_id = required_text_value(&stored, self.fields.doc_id, "document id")?;
                if excluded_doc_ids.contains(&doc_id) {
                    continue;
                }

                let resume_version_id = required_text_value(
                    &stored,
                    self.fields.resume_version_id,
                    "resume version id",
                )?;
                let clean_text =
                    required_text_value(&stored, self.fields.clean_text, "clean text")?;

                documents.push(IndexDocument {
                    doc_id,
                    resume_version_id,
                    file_name: text_value(&stored, self.fields.file_name).unwrap_or_default(),
                    clean_text,
                    sections: Vec::new(),
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
        let mut query_parser = QueryParser::for_index(
            &self.index,
            vec![self.fields.file_name, self.fields.clean_text],
        );
        query_parser.set_conjunction_by_default();
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
            let doc_id = required_text_value(&stored, self.fields.doc_id, "document id")?;
            if !seen_doc_ids.insert(doc_id.clone()) {
                return Err(FullTextError::internal(
                    "full-text snapshot returned duplicate document identity",
                ));
            }

            let resume_version_id =
                required_text_value(&stored, self.fields.resume_version_id, "resume version id")?;
            let clean_text = required_text_value(&stored, self.fields.clean_text, "clean text")?;
            hits.push(SearchHit {
                rank: hits.len() + 1,
                score,
                doc_id,
                resume_version_id,
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

pub fn publish_snapshot<I>(
    index_root: &Path,
    snapshot_name: &str,
    documents: I,
) -> Result<PublishedSnapshotMetadata>
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
) -> Result<PublishedSnapshotMetadata>
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
) -> Result<PublishedSnapshotMetadata>
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
) -> Result<PublishedSnapshotMetadata>
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
) -> Result<PublishedSnapshotMetadata>
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
) -> Result<PublishedSnapshotMetadata>
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
) -> Result<PublishedSnapshotMetadata>
where
    I: IntoIterator<Item = D>,
    D: Borrow<IndexDocument>,
{
    let (documents, index_root, staging_dir, published_dir, index, publication_lease) =
        measure_snapshot_publish_phase(control, SnapshotPublishPhase::Setup, || {
            validate_snapshot_name(snapshot_name)?;
            control.check()?;
            let mut publication_lease = SnapshotPublicationLease::acquire(index_root)?;
            let index_root = publication_lease.index_root().to_path_buf();
            let documents = documents.into_iter();
            let writer_config = writer_config_for_snapshot(exact_size_hint(documents.size_hint()));

            let staging_root = index_root.join(STAGING_DIR);
            let snapshots_root = index_root.join(SNAPSHOTS_DIR);
            let generation_pins_root = index_root.join(GENERATION_PINS_DIR);
            let layout_created = ensure_private_snapshot_directory(&staging_root)?
                | ensure_private_snapshot_directory(&snapshots_root)?
                | ensure_private_snapshot_directory(&generation_pins_root)?;
            drop(create_snapshot_reader_lock(&index_root)?);
            if layout_created {
                sync_directory(&index_root)?;
            }
            publication_lease.pin_layout()?;

            let staging_path = private_staging_dir_path(&staging_root, snapshot_name)?;
            let published_dir = snapshots_root.join(snapshot_name);
            if path_entry_exists(&published_dir)? {
                return Err(FullTextError::internal("full-text snapshot already exists"));
            }

            if !ensure_private_snapshot_directory(&staging_path)? {
                return Err(FullTextError::internal("full-text staging path collision"));
            }
            let staging_dir = PinnedSnapshotDirectory::acquire(&staging_path)?;
            let index = FullTextIndex::open_or_create_with_writer_config(
                staging_dir.path(),
                writer_config,
                control.writer_heap_bytes(),
            )?;
            Ok((
                documents,
                index_root,
                staging_dir,
                published_dir,
                index,
                publication_lease,
            ))
        })?;

    let indexed_documents =
        measure_snapshot_publish_phase(control, SnapshotPublishPhase::DocumentIndexing, || {
            publication_lease.validate_layout()?;
            staging_dir.validate_current()?;
            let indexed = index.replace_documents_with_redaction(documents, control, redaction)?;
            staging_dir.validate_current()?;
            Ok(indexed)
        })?;
    measure_snapshot_publish_phase(control, SnapshotPublishPhase::TantivyCommit, || {
        publication_lease.validate_layout()?;
        staging_dir.validate_current()?;
        control.check()?;
        index.commit()?;
        staging_dir.validate_current()
    })?;
    let (projection_digest, logical_content_digest) =
        measure_snapshot_publish_phase(control, SnapshotPublishPhase::PlaintextValidation, || {
            publication_lease.validate_layout()?;
            staging_dir.validate_current()?;
            control.check()?;
            drop(index);
            let digests =
                validate_plaintext_snapshot_contents(staging_dir.path(), indexed_documents)?;
            staging_dir.validate_current()?;
            Ok(digests)
        })?;
    let encrypted_staging = measure_snapshot_publish_phase(
        control,
        SnapshotPublishPhase::EncryptedPublication,
        || {
            publication_lease.validate_layout()?;
            control.check()?;
            prepare_encrypted_staging_snapshot(
                &index_root,
                snapshot_name,
                indexed_documents,
                &projection_digest,
                &logical_content_digest,
                staging_dir,
                &published_dir,
            )
        },
    )?;
    let validation =
        measure_snapshot_publish_phase(control, SnapshotPublishPhase::EncryptedValidation, || {
            publication_lease.validate_layout()?;
            encrypted_staging.validate_current()?;
            control.check()?;
            let metadata = validate_snapshot_contents(
                &index_root,
                encrypted_staging.path(),
                snapshot_name,
                indexed_documents,
            )?;
            encrypted_staging.validate_current()?;
            Ok(metadata)
        });
    let metadata = match validation {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = remove_pinned_snapshot_dir_all(&encrypted_staging);
            return Err(error);
        }
    };
    let publication =
        measure_snapshot_publish_phase(control, SnapshotPublishPhase::AtomicPublication, || {
            publication_lease.validate_layout()?;
            encrypted_staging.validate_current()?;
            control.check()?;
            create_generation_pin(&index_root, snapshot_name)?;
            publish_pinned_staging_snapshot_with_pin_cleanup(
                &encrypted_staging,
                &published_dir,
                &FsSnapshotPublisher,
                SNAPSHOT_PUBLISH_RETRY_DELAY,
                || remove_generation_pin(&index_root, snapshot_name),
                |_| {},
                |_| {},
            )?;
            sync_directory(&published_dir)?;
            publication_lease.validate_layout()?;
            encrypted_staging.validate_identity_at(&published_dir)?;
            Ok(())
        });
    if publication.is_err() {
        let _ = remove_pinned_snapshot_dir_all(&encrypted_staging);
    }
    publication?;
    Ok(metadata)
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
    base_snapshot: Option<&str>,
    snapshot_name: &str,
    replacement_documents: I,
    deleted_doc_ids: &BTreeSet<String>,
) -> Result<SnapshotPublishSummary>
where
    I: IntoIterator<Item = IndexDocument>,
{
    let documents = incremental_snapshot_documents(
        index_root,
        base_snapshot,
        replacement_documents,
        deleted_doc_ids,
    )?;
    let metadata = publish_snapshot(index_root, snapshot_name, documents)?;
    let indexed_documents = metadata.document_count();

    Ok(SnapshotPublishSummary {
        indexed_documents,
        metadata,
    })
}

pub fn incremental_snapshot_documents<I>(
    index_root: &Path,
    base_snapshot: Option<&str>,
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

    let mut documents = snapshot_documents_except(index_root, base_snapshot, &excluded_doc_ids)?;
    documents.extend(replacement_documents);
    documents.sort_by(|left, right| {
        left.doc_id
            .cmp(&right.doc_id)
            .then_with(|| left.resume_version_id.cmp(&right.resume_version_id))
    });

    Ok(documents)
}

fn snapshot_documents_except(
    index_root: &Path,
    base_snapshot: Option<&str>,
    excluded_doc_ids: &BTreeSet<String>,
) -> Result<Vec<IndexDocument>> {
    let Some(snapshot_name) = base_snapshot else {
        return Ok(Vec::new());
    };
    let Some(index) = FullTextIndex::open_snapshot(index_root, snapshot_name)? else {
        return Err(FullTextError::internal(
            "full-text base snapshot is unavailable",
        ));
    };

    index.stored_documents_except(excluded_doc_ids)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotPublishSummary {
    indexed_documents: usize,
    metadata: PublishedSnapshotMetadata,
}

impl SnapshotPublishSummary {
    pub fn indexed_documents(&self) -> usize {
        self.indexed_documents
    }

    pub fn metadata(&self) -> &PublishedSnapshotMetadata {
        &self.metadata
    }
}

fn prepare_encrypted_staging_snapshot(
    index_root: &Path,
    snapshot_name: &str,
    document_count: usize,
    projection_digest: &SearchProjectionDigest,
    logical_content_digest: &ContentDigest,
    staging_dir: PinnedSnapshotDirectory,
    published_dir: &Path,
) -> Result<PinnedSnapshotDirectory> {
    staging_dir.validate_current()?;
    let temp_published_dir = private_snapshot_dir_path(published_dir)?;
    if path_entry_exists(&temp_published_dir)? {
        return Err(FullTextError::internal(
            "full-text encrypted staging path collision",
        ));
    }
    if !ensure_private_snapshot_directory(&temp_published_dir)? {
        return Err(FullTextError::internal(
            "full-text encrypted staging path collision",
        ));
    }
    let encrypted_staging = PinnedSnapshotDirectory::acquire(&temp_published_dir)?;

    let archive = snapshot_archive_bytes(staging_dir.path())?;
    staging_dir.validate_current()?;
    let artifact_digest = write_encrypted_snapshot(
        &encrypted_staging.path().join(ENCRYPTED_SNAPSHOT_FILE),
        &index_root.join(SNAPSHOT_KEY_FILE),
        snapshot_name,
        &archive,
    )?;
    write_snapshot_manifest(
        encrypted_staging.path(),
        snapshot_name,
        document_count,
        projection_digest,
        logical_content_digest,
        &artifact_digest,
    )?;
    encrypted_staging.validate_current()?;
    staging_dir.validate_current()?;
    remove_pinned_snapshot_dir_all(&staging_dir)?;
    encrypted_staging.validate_current()?;
    Ok(encrypted_staging)
}

fn remove_snapshot_dir_all(path: &Path) -> Result<()> {
    retry_transient_snapshot_fs_operation(SNAPSHOT_PUBLISH_RETRY_DELAY, || fs::remove_dir_all(path))
        .map_err(FullTextError::io)
}

fn remove_pinned_snapshot_dir_all(directory: &PinnedSnapshotDirectory) -> Result<()> {
    directory.validate_current()?;
    remove_snapshot_dir_all(directory.path())
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailedSnapshotCleanupClass {
    Io,
    Contract,
    Cancelled,
    Index,
}

impl FailedSnapshotCleanupClass {
    fn from_error(error: &FullTextError) -> Self {
        match error {
            FullTextError::Io { .. } => Self::Io,
            FullTextError::Internal { .. } => Self::Contract,
            FullTextError::Cancelled => Self::Cancelled,
            FullTextError::Tantivy { .. } => Self::Index,
        }
    }
}

fn publish_pinned_staging_snapshot_with_pin_cleanup<P: SnapshotPublisher>(
    staging_dir: &PinnedSnapshotDirectory,
    published_dir: &Path,
    publisher: &P,
    retry_delay: Duration,
    cleanup_pin: impl FnOnce() -> Result<()>,
    observe_cleanup_failure: impl FnOnce(FailedSnapshotCleanupClass),
    after_publish: impl FnOnce(&Path),
) -> Result<()> {
    staging_dir.validate_current()?;
    match publish_staging_snapshot_with(staging_dir.path(), published_dir, publisher, retry_delay) {
        Ok(()) => {
            after_publish(published_dir);
            staging_dir.validate_identity_at(published_dir)
        }
        Err(primary) => {
            // The unpublished generation pin is a bounded, controlled orphan
            // reclaimed by GC. Its cleanup failure must not replace the rename
            // failure that explains why publication did not happen.
            if let Err(cleanup_error) = cleanup_pin() {
                observe_cleanup_failure(FailedSnapshotCleanupClass::from_error(&cleanup_error));
            }
            Err(primary)
        }
    }
}

#[cfg(unix)]
fn sync_directory(published_dir: &Path) -> Result<()> {
    File::open(published_dir)
        .and_then(|directory| directory.sync_all())
        .map_err(FullTextError::io)?;
    let parent = published_dir
        .parent()
        .ok_or_else(|| FullTextError::internal("full-text snapshot parent missing"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(FullTextError::io)
}

#[cfg(windows)]
fn sync_directory(published_dir: &Path) -> Result<()> {
    sync_windows_directory(published_dir)?;
    let parent = published_dir
        .parent()
        .ok_or_else(|| FullTextError::internal("full-text snapshot parent missing"))?;
    sync_windows_directory(parent)
}

#[cfg(windows)]
fn sync_windows_directory(path: &Path) -> Result<()> {
    let directory = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ_WRITE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_WRITE_THROUGH)
        .open(path)
        .map_err(FullTextError::io)?;
    directory.sync_all().map_err(FullTextError::io)
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_published_dir: &Path) -> Result<()> {
    Err(FullTextError::internal(
        "full-text directory durability is unsupported on this platform",
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
    read_snapshot_file_with_retry(path, |path| read_regular_file(path, FilePrivacy::Regular))
}

fn read_private_snapshot_file(path: &Path) -> Result<Vec<u8>> {
    read_snapshot_file_with_retry(path, |path| read_regular_file(path, FilePrivacy::OwnerOnly))
}

fn read_private_snapshot_file_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    read_snapshot_file_with_retry(path, |path| {
        read_regular_file_bounded(path, FilePrivacy::OwnerOnly, max_bytes)
    })
}

fn read_snapshot_file_with_retry(
    path: &Path,
    mut read: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    retry_transient_snapshot_fs_operation(SNAPSHOT_PUBLISH_RETRY_DELAY, || read(path))
        .map_err(FullTextError::io)
}

#[derive(Clone, Copy)]
enum FilePrivacy {
    Regular,
    OwnerOnly,
}

fn read_regular_file(path: &Path, privacy: FilePrivacy) -> std::io::Result<Vec<u8>> {
    read_regular_file_with_optional_limit(path, privacy, None)
}

fn read_regular_file_bounded(
    path: &Path,
    privacy: FilePrivacy,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    read_regular_file_with_optional_limit(path, privacy, Some(max_bytes))
}

fn read_regular_file_with_optional_limit(
    path: &Path,
    privacy: FilePrivacy,
    max_bytes: Option<usize>,
) -> std::io::Result<Vec<u8>> {
    let before = fs::symlink_metadata(path)?;
    validate_regular_file_metadata(&before, privacy)?;
    validate_regular_file_size(&before, max_bytes)?;
    let mut file = File::open(path)?;
    let opened = file.metadata()?;
    validate_regular_file_metadata(&opened, privacy)?;
    validate_regular_file_size(&opened, max_bytes)?;
    let current = fs::symlink_metadata(path)?;
    validate_regular_file_metadata(&current, privacy)?;
    validate_regular_file_size(&current, max_bytes)?;
    if !same_open_file_identity(&file, path, &opened, &current)? {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "full-text file identity changed during open",
        ));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(opened.len())
            .unwrap_or(usize::MAX)
            .min(max_bytes.unwrap_or(8 * 1024)),
    );
    if let Some(max_bytes) = max_bytes {
        file.take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > max_bytes {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "full-text artifact exceeds size limit",
            ));
        }
    } else {
        file.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}

fn validate_regular_file_size(
    metadata: &fs::Metadata,
    max_bytes: Option<usize>,
) -> std::io::Result<()> {
    if max_bytes.is_some_and(|max_bytes| metadata.len() > max_bytes as u64) {
        Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "full-text artifact exceeds size limit",
        ))
    } else {
        Ok(())
    }
}

fn validate_regular_file_metadata(
    metadata: &fs::Metadata,
    privacy: FilePrivacy,
) -> std::io::Result<()> {
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "full-text artifact must be a regular non-symlink file",
        ));
    }
    #[cfg(unix)]
    if matches!(privacy, FilePrivacy::OwnerOnly) && metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(std::io::Error::new(
            ErrorKind::InvalidData,
            "full-text private artifact permissions invalid",
        ));
    }
    #[cfg(not(unix))]
    let _ = privacy;
    Ok(())
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

fn validate_plaintext_snapshot_contents(
    snapshot_dir: &Path,
    expected_document_count: usize,
) -> Result<(SearchProjectionDigest, ContentDigest)> {
    let validation = FullTextIndex::open(snapshot_dir)?;
    validation
        .validate_exact_contents(expected_document_count)
        .map(|validation| {
            (
                validation.projection_digest,
                validation.logical_content_digest,
            )
        })
}

fn validate_snapshot_contents(
    index_root: &Path,
    snapshot_dir: &Path,
    expected_generation: &str,
    expected_document_count: usize,
) -> Result<PublishedSnapshotMetadata> {
    let validation = open_published_snapshot(index_root, snapshot_dir, expected_generation)?;
    let metadata = validation
        .snapshot_metadata()
        .ok_or_else(|| FullTextError::internal("full-text snapshot metadata missing"))?;
    if metadata.document_count() != expected_document_count {
        return Err(FullTextError::internal(
            "full-text snapshot manifest count mismatch",
        ));
    }
    Ok(metadata.clone())
}

fn open_published_snapshot(
    index_root: &Path,
    snapshot_dir: &Path,
    expected_generation: &str,
) -> Result<FullTextIndex> {
    validate_snapshot_directory(snapshot_dir)?;
    let metadata = validate_snapshot_manifest(snapshot_dir, expected_generation)?;
    let encrypted_path = snapshot_dir.join(ENCRYPTED_SNAPSHOT_FILE);
    let archive = read_encrypted_snapshot(
        &encrypted_path,
        &index_root.join(SNAPSHOT_KEY_FILE),
        expected_generation,
        metadata.artifact_digest(),
    )?;
    let temp_dir = create_private_temp_dir("fulltext-snapshot")?;
    extract_snapshot_archive(&archive, temp_dir.path())?;
    let mut index = FullTextIndex::open(temp_dir.path())?;
    let validation = index.validate_exact_contents(metadata.document_count())?;
    let actual_projection_digest = validation.projection_digest;
    let actual_logical_content_digest = validation.logical_content_digest;
    if &actual_projection_digest != metadata.projection_digest() {
        return Err(FullTextError::internal(
            "full-text snapshot projection digest mismatch",
        ));
    }
    if &actual_logical_content_digest != metadata.logical_content_digest() {
        return Err(FullTextError::internal(
            "full-text snapshot logical content digest mismatch",
        ));
    }
    index.exact_identity_pairs = Some(validation.identity_pairs);
    index.snapshot_metadata = Some(metadata);
    index._decrypted_snapshot_dir = Some(temp_dir);
    Ok(index)
}

fn write_snapshot_manifest(
    snapshot_dir: &Path,
    snapshot_name: &str,
    document_count: usize,
    projection_digest: &SearchProjectionDigest,
    logical_content_digest: &ContentDigest,
    artifact_digest: &ContentDigest,
) -> Result<()> {
    let manifest = encode_manifest(
        snapshot_name,
        document_count,
        projection_digest,
        logical_content_digest,
        artifact_digest,
    )
    .map_err(map_manifest_error)?;
    write_private_file(&snapshot_dir.join(SNAPSHOT_MANIFEST_FILE), &manifest)
}

fn validate_snapshot_manifest(
    snapshot_dir: &Path,
    expected_generation: &str,
) -> Result<PublishedSnapshotMetadata> {
    let manifest_path = snapshot_dir.join(SNAPSHOT_MANIFEST_FILE);
    decode_manifest(
        &read_private_snapshot_file_bounded(&manifest_path, MAX_MANIFEST_BYTES)?,
        expected_generation,
    )
    .map_err(map_manifest_error)
}

fn map_manifest_error(error: ManifestError) -> FullTextError {
    match error {
        ManifestError::Corrupt => FullTextError::internal("full-text snapshot manifest corrupt"),
        ManifestError::SchemaMismatch => {
            FullTextError::internal("full-text snapshot schema mismatch")
        }
    }
}

fn write_encrypted_snapshot(
    path: &Path,
    key_path: &Path,
    generation: &str,
    plaintext: &[u8],
) -> Result<ContentDigest> {
    let key = load_or_create_snapshot_key(key_path)?;
    let nonce = random_nonce()?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &snapshot_encryption_aad(generation),
            },
        )
        .map_err(|_| FullTextError::internal("full-text snapshot encryption failed"))?;

    let mut envelope =
        format!("{SNAPSHOT_HEADER_ENCRYPTED_V2}\n{}\n", encode_hex(&nonce)).into_bytes();
    envelope.extend_from_slice(&ciphertext);
    let content_digest = ContentDigest::from_bytes(&envelope);

    let mut file = create_private_file(path)?;
    file.write_all(&envelope).map_err(FullTextError::io)?;
    file.sync_all().map_err(FullTextError::io)?;
    Ok(content_digest)
}

fn read_encrypted_snapshot(
    path: &Path,
    key_path: &Path,
    generation: &str,
    expected_artifact_digest: &ContentDigest,
) -> Result<Vec<u8>> {
    let envelope = read_private_snapshot_file(path)?;
    if &ContentDigest::from_bytes(&envelope) != expected_artifact_digest {
        return Err(FullTextError::internal(
            "full-text snapshot artifact digest mismatch",
        ));
    }
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
    if header != SNAPSHOT_HEADER_ENCRYPTED_V2 {
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
                aad: &snapshot_encryption_aad(generation),
            },
        )
        .map_err(|_| FullTextError::internal("full-text snapshot decryption failed"))
}

fn snapshot_encryption_aad(generation: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(SNAPSHOT_HEADER_ENCRYPTED_V2.len() + generation.len() + 8);
    aad.extend_from_slice(SNAPSHOT_HEADER_ENCRYPTED_V2.as_bytes());
    aad.extend_from_slice(&(generation.len() as u64).to_le_bytes());
    aad.extend_from_slice(generation.as_bytes());
    aad
}

fn snapshot_archive_bytes(root: &Path) -> Result<Vec<u8>> {
    let mut entries = Vec::new();
    collect_snapshot_archive_entries(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut output = Vec::new();
    output.extend_from_slice(SNAPSHOT_ARCHIVE_HEADER_V2);
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
    cursor.expect_prefix(SNAPSHOT_ARCHIVE_HEADER_V2)?;
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
    match fs::symlink_metadata(key_path) {
        Ok(_) => read_snapshot_key(key_path),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let key = random_key()?;
            write_new_private_file(key_path, encode_hex(&key).as_bytes())?;
            let parent = key_path
                .parent()
                .ok_or_else(|| FullTextError::internal("full-text snapshot key parent missing"))?;
            sync_directory(parent)?;
            Ok(key)
        }
        Err(error) => Err(FullTextError::io(error)),
    }
}

fn read_snapshot_key(key_path: &Path) -> Result<[u8; SNAPSHOT_KEY_LEN]> {
    let bytes = read_private_snapshot_file_bounded(key_path, MAX_SNAPSHOT_KEY_FILE_BYTES)?;
    let encoded = match bytes.as_slice() {
        value if value.len() == ENCODED_SNAPSHOT_KEY_LEN => value,
        value if value.len() == MAX_SNAPSHOT_KEY_FILE_BYTES && value.last() == Some(&b'\n') => {
            &value[..ENCODED_SNAPSHOT_KEY_LEN]
        }
        _ => return Err(FullTextError::internal("full-text snapshot key corrupt")),
    };
    let value = std::str::from_utf8(encoded)
        .map_err(|_| FullTextError::internal("full-text snapshot key corrupt"))?;
    decode_fixed_hex::<SNAPSHOT_KEY_LEN>(value)
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

fn private_staging_dir_path(staging_root: &Path, snapshot_name: &str) -> Result<PathBuf> {
    let mut suffix = [0_u8; 8];
    getrandom::getrandom(&mut suffix)
        .map_err(|_| FullTextError::internal("full-text staging random failed"))?;
    Ok(staging_root.join(format!(".{snapshot_name}.staging-{}", encode_hex(&suffix))))
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

fn write_new_private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(FullTextError::io)?;
        restrict_private_dir_permissions(parent)?;
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_WRITE_THROUGH);
    let mut file = options.open(path).map_err(FullTextError::io)?;
    let opened = file.metadata().map_err(FullTextError::io)?;
    validate_regular_file_metadata(&opened, FilePrivacy::OwnerOnly).map_err(FullTextError::io)?;
    let current = fs::symlink_metadata(path).map_err(FullTextError::io)?;
    validate_regular_file_metadata(&current, FilePrivacy::OwnerOnly).map_err(FullTextError::io)?;
    if !same_open_file_identity(&file, path, &opened, &current).map_err(FullTextError::io)? {
        return Err(FullTextError::internal(
            "full-text private file identity changed during create",
        ));
    }
    file.write_all(bytes).map_err(FullTextError::io)?;
    file.sync_all().map_err(FullTextError::io)
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
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_WRITE_THROUGH);
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

pub fn staging_orphan_count(index_root: &Path) -> Result<usize> {
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

fn snapshot_directory_exists(snapshot_dir: &Path) -> Result<bool> {
    match fs::symlink_metadata(snapshot_dir) {
        Ok(metadata) => {
            validate_snapshot_directory_metadata(&metadata)?;
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(FullTextError::io(error)),
    }
}

fn ensure_canonical_index_root(index_root: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(index_root) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(index_root).map_err(FullTextError::io)?;
            restrict_private_dir_permissions(index_root)?;
            let parent = index_root
                .parent()
                .ok_or_else(|| FullTextError::internal("full-text index parent missing"))?;
            sync_directory(parent)?;
        }
        Err(error) => return Err(FullTextError::io(error)),
    }
    let index_root = fs::canonicalize(index_root).map_err(FullTextError::io)?;
    validate_snapshot_directory(&index_root)?;
    Ok(index_root)
}

fn ensure_private_snapshot_directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_snapshot_directory_metadata(&metadata)?;
            Ok(false)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(path).map_err(FullTextError::io)?;
            restrict_private_dir_permissions(path)?;
            validate_snapshot_directory(path)?;
            Ok(true)
        }
        Err(error) => Err(FullTextError::io(error)),
    }
}

fn validate_snapshot_directory(snapshot_dir: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(snapshot_dir).map_err(FullTextError::io)?;
    validate_snapshot_directory_metadata(&metadata)
}

fn validate_snapshot_directory_metadata(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(FullTextError::internal(
            "full-text generation must be a regular non-symlink directory",
        ));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(FullTextError::internal(
            "full-text generation directory permissions invalid",
        ));
    }
    Ok(())
}

fn path_entry_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(FullTextError::io(error)),
    }
}

fn validate_snapshot_name(snapshot_name: &str) -> Result<()> {
    if snapshot_name.is_empty()
        || snapshot_name.len() > MAX_SNAPSHOT_NAME_BYTES
        || matches!(snapshot_name, "." | "..")
        || snapshot_name.starts_with('.')
        || !snapshot_name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
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
    resume_version_id: Field,
    file_name: Field,
    clean_text: Field,
}

impl IndexFields {
    fn from_schema(schema: &Schema) -> Result<Self> {
        Ok(Self {
            doc_id: schema.get_field("doc_id").map_err(FullTextError::tantivy)?,
            resume_version_id: schema
                .get_field("resume_version_id")
                .map_err(FullTextError::tantivy)?,
            file_name: schema
                .get_field("file_name")
                .map_err(FullTextError::tantivy)?,
            clean_text: schema
                .get_field("clean_text")
                .map_err(FullTextError::tantivy)?,
        })
    }
}

fn build_schema() -> Schema {
    let mut builder = Schema::builder();
    builder.add_text_field("doc_id", STRING | STORED);
    builder.add_text_field("resume_version_id", STORED);
    builder.add_text_field("file_name", TEXT | STORED);
    builder.add_text_field("clean_text", TEXT | STORED);
    builder.build()
}

fn validate_index_schema(schema: &Schema) -> Result<()> {
    if schema != &build_schema() {
        return Err(FullTextError::internal(format!(
            "full-text index schema mismatch: {FULLTEXT_INDEX_SCHEMA_VERSION} required"
        )));
    }
    Ok(())
}

fn validate_stable_id(value: &str, prefix: &str, kind: &str) -> Result<()> {
    let digest = value.strip_prefix(prefix).ok_or_else(|| {
        FullTextError::internal(format!("full-text {kind} identity prefix invalid"))
    })?;
    if digest.len() != STABLE_ID_DIGEST_LEN
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(FullTextError::internal(format!(
            "full-text {kind} identity digest invalid"
        )));
    }
    Ok(())
}

fn text_value(document: &TantivyDocument, field: Field) -> Option<String> {
    document
        .get_first(field)
        .and_then(|value| value.as_value().as_str())
        .map(str::to_string)
}

fn required_text_value(document: &TantivyDocument, field: Field, label: &str) -> Result<String> {
    text_value(document, field)
        .ok_or_else(|| FullTextError::internal(format!("full-text snapshot {label} missing")))
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
    redact_contact_values(text[start..end].trim()).into_owned()
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
    fn purge_classifier_excludes_only_exact_empty_fulltext_controls() {
        let root = temp_dir("purge-classifier");
        publish_snapshot(&root, "generation-one", Vec::<IndexDocument>::new()).unwrap();
        let canonical = fs::canonicalize(&root).unwrap();

        for path in [
            canonical.join(SNAPSHOT_READER_LOCK_FILE),
            canonical.join(SNAPSHOT_PUBLICATION_LOCK_FILE),
            canonical
                .join(GENERATION_PINS_DIR)
                .join("generation-one.lock"),
        ] {
            assert_eq!(
                classify_purge_artifact(&canonical, &path).unwrap(),
                FullTextPurgeArtifactClass::ControlPlaneFile
            );
        }
        assert_eq!(
            classify_purge_artifact(&canonical, &canonical.join(GENERATION_PINS_DIR)).unwrap(),
            FullTextPurgeArtifactClass::ControlPlaneDirectory
        );
        let similar = canonical.join("snapshot-readers.lock.backup");
        fs::write(&similar, b"ordinary data").unwrap();
        assert_eq!(
            classify_purge_artifact(&canonical, &similar).unwrap(),
            FullTextPurgeArtifactClass::Data
        );
        fs::write(
            canonical.join(SNAPSHOT_PUBLICATION_LOCK_FILE),
            b"contaminated",
        )
        .unwrap();
        assert!(classify_purge_artifact(
            &canonical,
            &canonical.join(SNAPSHOT_PUBLICATION_LOCK_FILE)
        )
        .is_err());
        assert!(
            classify_purge_artifact(&canonical, &canonical.parent().unwrap().join("outside"))
                .is_err()
        );
        remove_dir(&root);
    }

    #[test]
    fn borrowed_snapshot_publish_indexes_documents_without_taking_ownership() {
        let index_root = temp_dir("borrowed-snapshot-publish");
        let documents = [IndexDocument {
            doc_id: "doc_00000000000000000000000000000001".to_string(),
            resume_version_id: "ver_00000000000000000000000000000001".to_string(),
            file_name: "borrowed.pdf".to_string(),
            clean_text: "Borrowed snapshot Rust search".to_string(),
            sections: vec![IndexSection {
                section_type: "skills".to_string(),
                text: "Rust search".to_string(),
            }],
        }];

        publish_snapshot_refs(&index_root, "fulltext-borrowed-1-0-0", documents.iter()).unwrap();

        let index = FullTextIndex::open_snapshot(&index_root, "fulltext-borrowed-1-0-0")
            .unwrap()
            .unwrap();
        let hits = index.search(SearchQuery::new("Borrowed Rust")).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].doc_id, documents[0].doc_id);

        remove_dir(&index_root);
    }

    #[test]
    fn snapshot_publish_control_cancels_between_documents() {
        let index_root = temp_dir("snapshot-publish-control-cancel");
        let documents = (0..32)
            .map(|index| IndexDocument {
                doc_id: format!("doc_{index:03}"),
                resume_version_id: format!("ver_{index:03}"),
                file_name: format!("candidate-{index:03}.pdf"),
                clean_text: format!("Candidate {index:03} Rust search"),
                sections: vec![IndexSection {
                    section_type: "skills".to_string(),
                    text: "Rust search".to_string(),
                }],
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
        assert!(
            FullTextIndex::open_snapshot(&index_root, "fulltext-cancelled-1-0-0")
                .unwrap()
                .is_none()
        );

        remove_dir(&index_root);
    }

    #[test]
    fn snapshot_publish_control_reports_publication_subphases() {
        let index_root = temp_dir("snapshot-publish-control-phases");
        let documents = [IndexDocument {
            doc_id: "doc_00000000000000000000000000000002".to_string(),
            resume_version_id: "ver_00000000000000000000000000000002".to_string(),
            file_name: "phases.pdf".to_string(),
            clean_text: "Snapshot phase attribution Rust search".to_string(),
            sections: vec![IndexSection {
                section_type: "skills".to_string(),
                text: "Rust search".to_string(),
            }],
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
            "index_publication_atomic_commit",
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
            doc_id: "doc_00000000000000000000000000000003".to_string(),
            resume_version_id: "ver_00000000000000000000000000000003".to_string(),
            file_name: "phase-timings.pdf".to_string(),
            clean_text: "Snapshot phase timing Rust search".to_string(),
            sections: vec![IndexSection {
                section_type: "skills".to_string(),
                text: "Rust search".to_string(),
            }],
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
            "index_publication_atomic_commit",
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
    fn trusted_redacted_snapshot_publish_preserves_redacted_content() {
        let index_root = temp_dir("trusted-redacted-snapshot-publish");
        let documents = vec![IndexDocument {
            doc_id: "doc_00000000000000000000000000000004".to_string(),
            resume_version_id: "ver_00000000000000000000000000000004".to_string(),
            file_name: "<redacted-email> resume.pdf".to_string(),
            clean_text: "Email <redacted-email> Phone <redacted-phone> File <redacted-path> Rust"
                .to_string(),
            sections: Vec::new(),
        }];
        publish_trusted_redacted_snapshot_with_control(
            &index_root,
            "trusted-redacted-1-0-0",
            documents,
            SnapshotPublishControl::disabled(),
        )
        .unwrap();

        let index = FullTextIndex::open_snapshot(&index_root, "trusted-redacted-1-0-0")
            .unwrap()
            .unwrap();
        let hits = index.search(SearchQuery::new("Rust")).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].file_name.contains("<redacted-email>"));
        assert!(hits[0].snippet.contains("<redacted-phone>"));
        assert!(hits[0].snippet.contains("<redacted-path>"));

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
        let resume_version_id = schema.get_field("resume_version_id").unwrap();

        match schema.get_field_entry(doc_id).field_type() {
            FieldType::Str(options) => {
                assert!(options.is_stored());
                assert!(options.get_indexing_options().is_some());
                assert!(!options.is_fast());
            }
            other => panic!("doc_id should be a string field, got {other:?}"),
        }
        match schema.get_field_entry(resume_version_id).field_type() {
            FieldType::Str(options) => {
                assert!(options.is_stored());
                assert!(options.get_indexing_options().is_none());
                assert!(!options.is_fast());
            }
            other => panic!("resume_version_id should be a string field, got {other:?}"),
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
    fn failed_pin_cleanup_never_overwrites_the_publication_error() {
        let index_root = temp_dir("primary-publication-error");
        let staging_root = index_root.join("staging");
        let snapshots_root = index_root.join("snapshots");
        ensure_private_snapshot_directory(&staging_root).unwrap();
        ensure_private_snapshot_directory(&snapshots_root).unwrap();
        let staging_path = staging_root.join("primary-error.tmp");
        ensure_private_snapshot_directory(&staging_path).unwrap();
        let staging = PinnedSnapshotDirectory::acquire(&staging_path).unwrap();
        let cleanup_class = std::cell::Cell::new(None);

        let error = publish_pinned_staging_snapshot_with_pin_cleanup(
            &staging,
            &snapshots_root.join("primary-error"),
            &ExistingDestinationPublisher::default(),
            Duration::ZERO,
            || {
                Err(FullTextError::internal(
                    "injected generation pin cleanup failure",
                ))
            },
            |class| cleanup_class.set(Some(class)),
            |_| panic!("failed publication must not run the success observer"),
        )
        .unwrap_err();

        match error {
            FullTextError::Io { diagnostic } => assert_eq!(diagnostic, "exists"),
            other => panic!("publication primary error was overwritten: {other:?}"),
        }
        assert_eq!(
            cleanup_class.get(),
            Some(FailedSnapshotCleanupClass::Contract)
        );
        remove_dir(&index_root);
    }

    #[cfg(unix)]
    #[test]
    fn published_generation_must_keep_the_original_staging_identity() {
        let index_root = temp_dir("published-generation-identity");
        let staging_root = index_root.join("staging");
        let snapshots_root = index_root.join("snapshots");
        ensure_private_snapshot_directory(&staging_root).unwrap();
        ensure_private_snapshot_directory(&snapshots_root).unwrap();
        let staging_path = staging_root.join("identity.tmp");
        ensure_private_snapshot_directory(&staging_path).unwrap();
        write_private_file(&staging_path.join("original-marker"), b"original").unwrap();
        let staging = PinnedSnapshotDirectory::acquire(&staging_path).unwrap();
        let published = snapshots_root.join("identity");
        let displaced = index_root.join("identity-original");

        let error = publish_pinned_staging_snapshot_with_pin_cleanup(
            &staging,
            &published,
            &FsSnapshotPublisher,
            Duration::ZERO,
            || Ok(()),
            |_| panic!("successful rename must not run pin cleanup"),
            |published| {
                fs::rename(published, &displaced).unwrap();
                ensure_private_snapshot_directory(published).unwrap();
                write_private_file(&published.join("replacement-marker"), b"replacement").unwrap();
            },
        )
        .unwrap_err();

        assert!(matches!(error, FullTextError::Internal { .. }));
        assert_eq!(
            fs::read(published.join("replacement-marker")).unwrap(),
            b"replacement"
        );
        assert_eq!(
            fs::read(displaced.join("original-marker")).unwrap(),
            b"original"
        );
        remove_dir(&index_root);
    }

    #[cfg(unix)]
    #[test]
    fn staging_identity_replacement_fails_closed_without_deleting_replacement() {
        let index_root = temp_dir("staging-identity-replacement");
        let document = IndexDocument {
            doc_id: "doc_00000000000000000000000000000003".to_string(),
            resume_version_id: "ver_00000000000000000000000000000003".to_string(),
            file_name: "identity.pdf".to_string(),
            clean_text: "Synthetic identity test".to_string(),
            sections: Vec::new(),
        };
        let replaced = std::cell::Cell::new(false);
        let replacement_path = std::cell::RefCell::new(None);
        let observer = |phase: SnapshotPublishPhase| {
            if phase != SnapshotPublishPhase::DocumentIndexing || replaced.replace(true) {
                return;
            }
            let staging = fs::read_dir(index_root.join(STAGING_DIR))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            let displaced = index_root.join("staging-original");
            fs::rename(&staging, displaced).unwrap();
            ensure_private_snapshot_directory(&staging).unwrap();
            write_private_file(&staging.join("replacement-marker"), b"replacement").unwrap();
            replacement_path.replace(Some(staging));
        };
        let control = SnapshotPublishControl::disabled().with_phase_observer(&observer);

        let error = publish_snapshot_refs_with_control(
            &index_root,
            "staging-identity",
            [&document],
            control,
        )
        .unwrap_err();

        assert!(matches!(error, FullTextError::Internal { .. }));
        let replacement = replacement_path.into_inner().unwrap();
        assert_eq!(
            fs::read(replacement.join("replacement-marker")).unwrap(),
            b"replacement"
        );
        assert!(!index_root.join("snapshots/staging-identity").exists());
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

    #[test]
    fn snapshot_key_read_is_bounded_and_rejects_oversized_input() {
        let root = temp_dir("snapshot-key-bound");
        let key_path = root.join("key");
        write_new_private_file(&key_path, &[b'a'; MAX_SNAPSHOT_KEY_FILE_BYTES + 1]).unwrap();

        assert!(read_snapshot_key(&key_path).is_err());
        remove_dir(&root);
    }

    #[test]
    fn snapshot_key_read_rejects_noncanonical_trailing_whitespace() {
        let root = temp_dir("snapshot-key-whitespace");
        let key_path = root.join("key");
        let mut bytes = [b'a'; MAX_SNAPSHOT_KEY_FILE_BYTES];
        bytes[ENCODED_SNAPSHOT_KEY_LEN] = b' ';
        write_new_private_file(&key_path, &bytes).unwrap();

        assert!(read_snapshot_key(&key_path).is_err());
        remove_dir(&root);
    }

    #[cfg(windows)]
    #[test]
    fn directory_sync_uses_a_flushable_write_through_handle() {
        let root = temp_dir("windows-directory-sync");
        sync_directory(&root).unwrap();
        remove_dir(&root);
    }

    #[cfg(windows)]
    #[test]
    fn file_identity_uses_volume_and_file_index() {
        let root = temp_dir("windows-file-identity");
        let first = root.join("first");
        let second = root.join("second");
        write_new_private_file(&first, b"first").unwrap();
        write_new_private_file(&second, b"second").unwrap();
        let first_metadata = fs::metadata(&first).unwrap();
        let first_file = File::open(&first).unwrap();
        assert!(same_open_file_identity(
            &first_file,
            &first,
            &first_metadata,
            &fs::metadata(&first).unwrap()
        )
        .unwrap());
        assert!(!same_open_file_identity(
            &first_file,
            &second,
            &first_metadata,
            &fs::metadata(&second).unwrap()
        )
        .unwrap());
        remove_dir(&root);
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
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    fn remove_dir(path: &Path) {
        let _ = fs::remove_dir_all(path);
    }
}
