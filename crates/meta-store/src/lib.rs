//! SQLite-backed metadata store for local resume indexing state.

use core_domain::{Document, ErrorKind, RedactionLevel, Result, ResumeError, SourceComponent};
use rusqlite::{params, Connection, OptionalExtension};
use std::fmt;
use std::path::Path;

/// SQLite metadata store.
pub struct MetadataStore {
    connection: Connection,
}

/// Non-sensitive document metadata returned by default visibility queries.
#[derive(Clone, Eq, PartialEq)]
pub struct DocumentRow {
    /// Stable document identifier.
    pub doc_id: String,
    /// Local path or logical URI. Keep local.
    pub source_uri: String,
    /// Normalized local path for dedupe and search. Keep local.
    pub normalized_path: String,
    /// File name only.
    pub file_name: String,
    /// Document extension label.
    pub extension: String,
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

impl fmt::Debug for DocumentRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentRow")
            .field("doc_id", &self.doc_id)
            .field("source_uri", &"<redacted>")
            .field("normalized_path", &"<redacted>")
            .field("file_name", &"<redacted>")
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

/// Typed ingestion job lifecycle states persisted in SQLite.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobState {
    /// Work is ready to be claimed.
    Queued,
    /// Work has been claimed by a worker.
    Running,
    /// Work failed but may be retried while budget remains.
    Failed,
    /// Work completed successfully.
    Completed,
    /// Work was explicitly cancelled.
    Cancelled,
    /// Work failed permanently and must not be retried.
    PermanentFailed,
}

impl JobState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::PermanentFailed => "permanent_failed",
        }
    }
}

impl TryFrom<&str> for JobState {
    type Error = String;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "failed" => Ok(Self::Failed),
            "completed" => Ok(Self::Completed),
            "cancelled" => Ok(Self::Cancelled),
            "permanent_failed" => Ok(Self::PermanentFailed),
            _ => Err(format!("unknown ingest job state: {value}")),
        }
    }
}

/// Ingestion job row selected for recovery or retry.
#[derive(Clone, Eq, PartialEq)]
pub struct IngestJobRow {
    /// Store-assigned job identifier.
    pub job_id: i64,
    /// Source document identifier.
    pub doc_id: String,
    /// Job type, such as parsing or indexing.
    pub job_type: String,
    /// Current job state.
    pub state: JobState,
    /// Maximum retry attempts.
    pub max_attempts: u32,
    /// Attempts already consumed.
    pub attempt_count: u32,
    /// Last local diagnostic error, if any.
    pub last_error: Option<String>,
}

impl fmt::Debug for IngestJobRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IngestJobRow")
            .field("job_id", &self.job_id)
            .field("doc_id", &self.doc_id)
            .field("job_type", &self.job_type)
            .field("state", &self.state)
            .field("max_attempts", &self.max_attempts)
            .field("attempt_count", &self.attempt_count)
            .field(
                "last_error",
                &self.last_error.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Non-sensitive local status summary for operator-facing commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreStatus {
    /// SQLite schema version currently installed.
    pub schema_version: u32,
    /// Number of visible document metadata rows.
    pub visible_document_count: u64,
    /// Number of queued import-root tasks.
    pub queued_import_task_count: u64,
    /// Number of known index state rows.
    pub index_state_count: u64,
    /// Number of documents that reached the S9 searchable state.
    pub searchable_document_count: u64,
    /// Number of documents that reached the S9 OCR-required state.
    pub ocr_required_document_count: u64,
}

/// Parsed resume-version row accepted by the local ingest smoke path.
pub struct ParsedResumeRecord<'a> {
    /// Stable parsed version identifier.
    pub version_id: &'a str,
    /// Source document identifier.
    pub doc_id: &'a str,
    /// Parser version label.
    pub parse_version: &'a str,
    /// Parsed schema version label.
    pub schema_version: &'a str,
    /// Extracted raw text, if available.
    pub raw_text: Option<&'a str>,
    /// Normalized text, if available.
    pub clean_text: Option<&'a str>,
    /// Search or OCR routing state.
    pub visibility: &'a str,
}

/// Import-root task row selected by local orchestration.
#[derive(Clone, Eq, PartialEq)]
pub struct ImportTaskRow {
    /// Store-assigned import task identifier.
    pub task_id: i64,
    /// Current task state.
    pub state: JobState,
    /// First enqueue timestamp.
    pub created_at: String,
}

impl fmt::Debug for ImportTaskRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportTaskRow")
            .field("task_id", &self.task_id)
            .field("state", &self.state)
            .field("root_path", &"<redacted>")
            .field("created_at", &self.created_at)
            .finish()
    }
}

/// Import-root task claimed for local execution.
#[derive(Clone, Eq, PartialEq)]
pub struct ImportTaskClaimRow {
    /// Store-assigned import task identifier.
    pub task_id: i64,
    /// Local root path. Keep local.
    pub root_path: String,
    /// Current task state.
    pub state: JobState,
    /// Maximum retry attempts.
    pub max_attempts: u32,
    /// Attempts already consumed.
    pub attempt_count: u32,
    /// Last local diagnostic error, if any.
    pub last_error: Option<String>,
    /// Opaque worker claim token. Keep local.
    pub claim_token: String,
    /// Lease expiry timestamp in caller-provided milliseconds.
    pub lease_expires_at_ms: i64,
}

impl fmt::Debug for ImportTaskClaimRow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportTaskClaimRow")
            .field("task_id", &self.task_id)
            .field("root_path", &"<redacted>")
            .field("state", &self.state)
            .field("max_attempts", &self.max_attempts)
            .field("attempt_count", &self.attempt_count)
            .field(
                "last_error",
                &self.last_error.as_ref().map(|_| "<redacted>"),
            )
            .field("claim_token", &"<redacted>")
            .field("lease_expires_at_ms", &self.lease_expires_at_ms)
            .finish()
    }
}

impl MetadataStore {
    /// Opens a metadata store at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path).map_err(storage_error)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(storage_error)?;
        Ok(Self { connection })
    }

    /// Opens an in-memory metadata store.
    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory().map_err(storage_error)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(storage_error)?;
        Ok(Self { connection })
    }

    /// Applies all schema migrations. Safe to call repeatedly.
    pub fn run_migrations(&self) -> Result<()> {
        let current_version = self.schema_version()?;
        if current_version > 3 {
            return Err(storage_diagnostic(format!(
                "newer metadata schema version {current_version} is not supported by this binary"
            )));
        }
        if current_version == 3 {
            return Ok(());
        }

        if current_version == 0 {
            self.connection
                .execute_batch(
                    r"
                BEGIN;

                CREATE TABLE IF NOT EXISTS document (
                    doc_id TEXT PRIMARY KEY,
                    source_uri TEXT NOT NULL,
                    normalized_path TEXT NOT NULL UNIQUE,
                    file_name TEXT NOT NULL,
                    extension TEXT NOT NULL,
                    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
                    mtime TEXT NOT NULL,
                    content_hash TEXT,
                    text_hash TEXT,
                    is_deleted INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1)),
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_document_visible_updated
                    ON document(is_deleted, updated_at);

                CREATE TABLE IF NOT EXISTS resume_version (
                    version_id TEXT PRIMARY KEY,
                    doc_id TEXT NOT NULL REFERENCES document(doc_id) ON DELETE CASCADE,
                    candidate_id TEXT,
                    parse_version TEXT NOT NULL,
                    schema_version TEXT NOT NULL,
                    language_set_json TEXT NOT NULL DEFAULT '[]',
                    page_count INTEGER CHECK (page_count IS NULL OR page_count >= 0),
                    raw_text TEXT,
                    clean_text TEXT,
                    quality_score REAL,
                    visibility TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX IF NOT EXISTS idx_resume_version_doc
                    ON resume_version(doc_id);

                CREATE TABLE IF NOT EXISTS ingest_job (
                    job_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    doc_id TEXT NOT NULL,
                    job_type TEXT NOT NULL,
                    state TEXT NOT NULL CHECK (
                        state IN (
                            'queued',
                            'running',
                            'failed',
                            'completed',
                            'cancelled',
                            'permanent_failed'
                        )
                    ),
                    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0),
                    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
                    last_error TEXT,
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX IF NOT EXISTS idx_ingest_job_recovery
                    ON ingest_job(state, attempt_count, max_attempts, job_id);

                CREATE TABLE IF NOT EXISTS index_state (
                    index_name TEXT PRIMARY KEY,
                    version_id TEXT,
                    status TEXT NOT NULL,
                    last_error TEXT,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE TABLE IF NOT EXISTS import_task (
                    task_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    root_path TEXT NOT NULL,
                    state TEXT NOT NULL CHECK (
                        state IN (
                            'queued',
                            'running',
                            'failed',
                            'completed',
                            'cancelled',
                            'permanent_failed'
                        )
                    ),
                    max_attempts INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts > 0),
                    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
                    last_error TEXT,
                    claim_token TEXT,
                    lease_expires_at_ms INTEGER CHECK (
                        lease_expires_at_ms IS NULL OR lease_expires_at_ms >= 0
                    ),
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX IF NOT EXISTS idx_import_task_state
                    ON import_task(state, task_id);

                PRAGMA user_version = 3;

                COMMIT;
                ",
                )
                .map_err(storage_error)?;
            return Ok(());
        }

        if current_version == 2 {
            self.connection
                .execute_batch(
                    r"
                    BEGIN;

                    ALTER TABLE import_task
                        ADD COLUMN max_attempts INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts > 0);
                    ALTER TABLE import_task
                        ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0);
                    ALTER TABLE import_task
                        ADD COLUMN last_error TEXT;
                    ALTER TABLE import_task
                        ADD COLUMN claim_token TEXT;
                    ALTER TABLE import_task
                        ADD COLUMN lease_expires_at_ms INTEGER CHECK (
                            lease_expires_at_ms IS NULL OR lease_expires_at_ms >= 0
                        );

                    CREATE INDEX IF NOT EXISTS idx_import_task_state
                        ON import_task(state, task_id);

                    PRAGMA user_version = 3;

                    COMMIT;
                    ",
                )
                .map_err(storage_error)?;
            return Ok(());
        }

        self.connection
            .execute_batch(
                r"
                BEGIN;

                CREATE TABLE IF NOT EXISTS import_task (
                    task_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    root_path TEXT NOT NULL,
                    state TEXT NOT NULL CHECK (
                        state IN (
                            'queued',
                            'running',
                            'failed',
                            'completed',
                            'cancelled',
                            'permanent_failed'
                        )
                    ),
                    max_attempts INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts > 0),
                    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
                    last_error TEXT,
                    claim_token TEXT,
                    lease_expires_at_ms INTEGER CHECK (
                        lease_expires_at_ms IS NULL OR lease_expires_at_ms >= 0
                    ),
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX IF NOT EXISTS idx_import_task_state
                    ON import_task(state, task_id);

                PRAGMA user_version = 3;

                COMMIT;
                ",
            )
            .map_err(storage_error)
    }

    /// Returns the current SQLite schema version.
    pub fn schema_version(&self) -> Result<u32> {
        self.connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .map_err(storage_error)
    }

    /// Inserts or updates a document metadata row.
    pub fn upsert_document(&self, document: &Document) -> Result<()> {
        let byte_size = i64::try_from(document.byte_size).map_err(|error| {
            storage_diagnostic(format!(
                "document byte_size does not fit SQLite INTEGER: {error}"
            ))
        })?;

        self.connection
            .execute(
                r"
                INSERT INTO document (
                    doc_id, source_uri, normalized_path, file_name, extension, byte_size,
                    mtime, content_hash, text_hash, is_deleted, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                ON CONFLICT(normalized_path) DO UPDATE SET
                    source_uri = excluded.source_uri,
                    file_name = excluded.file_name,
                    extension = excluded.extension,
                    byte_size = excluded.byte_size,
                    mtime = excluded.mtime,
                    content_hash = excluded.content_hash,
                    text_hash = excluded.text_hash,
                    is_deleted = CASE
                        WHEN document.is_deleted = 1 THEN 1
                        WHEN excluded.is_deleted = 1 THEN 1
                        ELSE 0
                    END,
                    updated_at = excluded.updated_at
                ",
                params![
                    document.doc_id.to_string(),
                    document.source_uri,
                    document.normalized_path,
                    document.file_name,
                    document_extension_label(document),
                    byte_size,
                    document.mtime,
                    document.content_hash,
                    document.text_hash,
                    bool_to_i64(document.is_deleted),
                    document.created_at,
                    document.updated_at,
                ],
            )
            .map(|_| ())
            .map_err(storage_error)
    }

    /// Returns documents visible to normal search and indexing flows.
    pub fn visible_documents(&self) -> Result<Vec<DocumentRow>> {
        let mut statement = self
            .connection
            .prepare(
                r"
                SELECT doc_id, source_uri, normalized_path, file_name, extension, byte_size,
                       mtime, content_hash, text_hash, is_deleted, created_at, updated_at
                FROM document
                WHERE is_deleted = 0
                ORDER BY doc_id
                ",
            )
            .map_err(storage_error)?;

        let rows = statement
            .query_map([], document_row_from_sql)
            .map_err(storage_error)?;
        let mut documents = Vec::new();
        for row in rows {
            documents.push(row.map_err(storage_error)?);
        }
        Ok(documents)
    }

    /// Returns one document by normalized local path when it exists.
    pub fn document_by_normalized_path(
        &self,
        normalized_path: &str,
    ) -> Result<Option<DocumentRow>> {
        let mut statement = self
            .connection
            .prepare(
                r"
                SELECT doc_id, source_uri, normalized_path, file_name, extension, byte_size,
                       mtime, content_hash, text_hash, is_deleted, created_at, updated_at
                FROM document
                WHERE normalized_path = ?1
                LIMIT 1
                ",
            )
            .map_err(storage_error)?;

        let mut rows = statement
            .query_map([normalized_path], document_row_from_sql)
            .map_err(storage_error)?;

        match rows.next() {
            Some(row) => Ok(Some(row.map_err(storage_error)?)),
            None => Ok(None),
        }
    }

    /// Returns one document by stable identifier, including deleted rows.
    pub fn document_by_doc_id(&self, doc_id: &str) -> Result<Option<DocumentRow>> {
        let mut statement = self
            .connection
            .prepare(
                r"
                SELECT doc_id, source_uri, normalized_path, file_name, extension, byte_size,
                       mtime, content_hash, text_hash, is_deleted, created_at, updated_at
                FROM document
                WHERE doc_id = ?1
                LIMIT 1
                ",
            )
            .map_err(storage_error)?;

        let mut rows = statement
            .query_map([doc_id], document_row_from_sql)
            .map_err(storage_error)?;

        match rows.next() {
            Some(row) => Ok(Some(row.map_err(storage_error)?)),
            None => Ok(None),
        }
    }

    /// Marks a document row deleted without touching its source file.
    pub fn mark_document_deleted(&self, doc_id: &str) -> Result<bool> {
        let changed = self
            .connection
            .execute(
                r"
                UPDATE document
                SET is_deleted = 1,
                    updated_at = CURRENT_TIMESTAMP
                WHERE doc_id = ?1
                ",
                [doc_id],
            )
            .map_err(storage_error)?;
        Ok(changed > 0)
    }

    /// Atomically records a local delete tombstone plus index-state intent.
    pub fn mark_document_deleted_with_index_state(
        &self,
        doc_id: &str,
        index_name: &str,
        version_id: Option<&str>,
        status: &str,
        last_error: Option<&str>,
    ) -> Result<bool> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(storage_error)?;

        let result = (|| {
            let changed = self.connection.execute(
                r"
                UPDATE document
                SET is_deleted = 1,
                    updated_at = CURRENT_TIMESTAMP
                WHERE doc_id = ?1
                ",
                [doc_id],
            )?;
            if changed == 0 {
                return Ok(false);
            }
            self.connection.execute(
                r"
                INSERT INTO index_state (index_name, version_id, status, last_error)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(index_name) DO UPDATE SET
                    version_id = excluded.version_id,
                    status = excluded.status,
                    last_error = excluded.last_error,
                    updated_at = CURRENT_TIMESTAMP
                ",
                params![index_name, version_id, status, last_error],
            )?;
            Ok(true)
        })();

        match result {
            Ok(changed) => {
                self.connection
                    .execute_batch("COMMIT")
                    .map_err(storage_error)?;
                Ok(changed)
            }
            Err(error) => {
                let _ = self.connection.execute_batch("ROLLBACK");
                Err(storage_error(error))
            }
        }
    }

    /// Starts a bulk write transaction for callers that need to insert many rows.
    pub fn begin_bulk_write(&self) -> Result<()> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(storage_error)
    }

    /// Commits a bulk write transaction started with [`Self::begin_bulk_write`].
    pub fn commit_bulk_write(&self) -> Result<()> {
        self.connection
            .execute_batch("COMMIT")
            .map_err(storage_error)
    }

    /// Rolls back a bulk write transaction if one is active.
    pub fn rollback_bulk_write(&self) {
        let _ = self.connection.execute_batch("ROLLBACK");
    }

    /// Returns the latest searchable clean text for a document.
    ///
    /// This returns local resume text to in-process callers only. Do not include
    /// the returned value in debug output or diagnostics.
    pub fn clean_text_by_doc_id(&self, doc_id: &str) -> Result<Option<String>> {
        let mut statement = self
            .connection
            .prepare(
                r"
                SELECT resume_version.clean_text
                FROM resume_version
                JOIN document ON document.doc_id = resume_version.doc_id
                WHERE resume_version.doc_id = ?1
                  AND document.is_deleted = 0
                  AND resume_version.visibility = 'SEARCHABLE'
                  AND resume_version.clean_text IS NOT NULL
                ORDER BY resume_version.updated_at DESC, resume_version.rowid DESC
                LIMIT 1
                ",
            )
            .map_err(storage_error)?;

        let mut rows = statement
            .query_map([doc_id], |row| row.get::<_, String>(0))
            .map_err(storage_error)?;

        match rows.next() {
            Some(row) => Ok(Some(row.map_err(storage_error)?)),
            None => Ok(None),
        }
    }

    /// Inserts or updates a parsed resume-version record.
    pub fn upsert_resume_version(&self, record: ParsedResumeRecord<'_>) -> Result<()> {
        self.connection
            .execute(
                r"
                INSERT INTO resume_version (
                    version_id, doc_id, parse_version, schema_version, language_set_json,
                    raw_text, clean_text, visibility
                )
                VALUES (?1, ?2, ?3, ?4, '[]', ?5, ?6, ?7)
                ON CONFLICT(version_id) DO UPDATE SET
                    doc_id = excluded.doc_id,
                    parse_version = excluded.parse_version,
                    schema_version = excluded.schema_version,
                    raw_text = excluded.raw_text,
                    clean_text = excluded.clean_text,
                    visibility = excluded.visibility,
                    updated_at = CURRENT_TIMESTAMP
                ",
                params![
                    record.version_id,
                    record.doc_id,
                    record.parse_version,
                    record.schema_version,
                    record.raw_text,
                    record.clean_text,
                    record.visibility,
                ],
            )
            .map(|_| ())
            .map_err(storage_error)
    }

    /// Inserts an ingest job and returns its store-assigned identifier.
    pub fn insert_ingest_job(
        &self,
        doc_id: &str,
        job_type: &str,
        state: JobState,
        max_attempts: u32,
        attempt_count: u32,
    ) -> Result<i64> {
        self.connection
            .execute(
                r"
                INSERT INTO ingest_job (doc_id, job_type, state, max_attempts, attempt_count)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
                params![
                    doc_id,
                    job_type,
                    state.as_str(),
                    i64::from(max_attempts),
                    i64::from(attempt_count)
                ],
            )
            .map_err(storage_error)?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Updates a job state and retry accounting.
    pub fn update_job_state(
        &self,
        job_id: i64,
        state: JobState,
        attempt_count: u32,
        last_error: Option<&str>,
    ) -> Result<()> {
        self.connection
            .execute(
                r"
                UPDATE ingest_job
                SET state = ?2,
                    attempt_count = ?3,
                    last_error = ?4,
                    updated_at = CURRENT_TIMESTAMP
                WHERE job_id = ?1
                ",
                params![job_id, state.as_str(), i64::from(attempt_count), last_error],
            )
            .map(|_| ())
            .map_err(storage_error)
    }

    /// Returns queued, failed, or interrupted running jobs that still have retry budget.
    pub fn retryable_jobs_for_recovery(&self) -> Result<Vec<IngestJobRow>> {
        let mut statement = self
            .connection
            .prepare(
                r"
                SELECT job_id, doc_id, job_type, state, max_attempts, attempt_count, last_error
                FROM ingest_job
                WHERE state IN ('queued', 'failed', 'running')
                  AND attempt_count < max_attempts
                ORDER BY job_id
                ",
            )
            .map_err(storage_error)?;

        let rows = statement
            .query_map([], ingest_job_row_from_sql)
            .map_err(storage_error)?;
        let mut jobs = Vec::new();
        for row in rows {
            jobs.push(row.map_err(storage_error)?);
        }
        Ok(jobs)
    }

    /// Inserts or updates the state of a local index.
    pub fn upsert_index_state(
        &self,
        index_name: &str,
        version_id: Option<&str>,
        status: &str,
        last_error: Option<&str>,
    ) -> Result<()> {
        self.connection
            .execute(
                r"
                INSERT INTO index_state (index_name, version_id, status, last_error)
                VALUES (?1, ?2, ?3, ?4)
                ON CONFLICT(index_name) DO UPDATE SET
                    version_id = excluded.version_id,
                    status = excluded.status,
                    last_error = excluded.last_error,
                    updated_at = CURRENT_TIMESTAMP
                ",
                params![index_name, version_id, status, last_error],
            )
            .map(|_| ())
            .map_err(storage_error)
    }

    /// Returns the status of one index-state row.
    pub fn index_state_status(&self, index_name: &str) -> Result<Option<String>> {
        let mut statement = self
            .connection
            .prepare(
                r"
                SELECT status
                FROM index_state
                WHERE index_name = ?1
                LIMIT 1
                ",
            )
            .map_err(storage_error)?;
        let mut rows = statement
            .query_map([index_name], |row| row.get::<_, String>(0))
            .map_err(storage_error)?;

        match rows.next() {
            Some(row) => Ok(Some(row.map_err(storage_error)?)),
            None => Ok(None),
        }
    }

    /// Inserts an import-root task and returns its store-assigned identifier.
    pub fn enqueue_import_root(&self, root_path: &Path) -> Result<i64> {
        self.connection
            .execute(
                r"
                INSERT INTO import_task (root_path, state)
                VALUES (?1, ?2)
                ",
                params![
                    root_path.to_string_lossy().as_ref(),
                    JobState::Queued.as_str()
                ],
            )
            .map_err(storage_error)?;
        Ok(self.connection.last_insert_rowid())
    }

    /// Returns queued import-root tasks without exposing local root paths.
    pub fn queued_import_tasks(&self) -> Result<Vec<ImportTaskRow>> {
        let mut statement = self
            .connection
            .prepare(
                r"
                SELECT task_id, state, created_at
                FROM import_task
                WHERE state = 'queued'
                ORDER BY task_id
                ",
            )
            .map_err(storage_error)?;

        let rows = statement
            .query_map([], import_task_row_from_sql)
            .map_err(storage_error)?;
        let mut tasks = Vec::new();
        for row in rows {
            tasks.push(row.map_err(storage_error)?);
        }
        Ok(tasks)
    }

    /// Returns import-root tasks that are eligible at the start of one daemon drain.
    ///
    /// Expired running leases are first moved into retryable or permanent-failed state.
    /// Returned rows do not expose local root paths.
    pub fn claimable_import_tasks(&self, now_ms: i64) -> Result<Vec<ImportTaskRow>> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(storage_error)?;

        let result = (|| {
            self.expire_stale_import_task_leases(now_ms)?;
            let mut statement = self.connection.prepare(
                r"
                SELECT task_id, state, created_at
                FROM import_task
                WHERE state IN ('queued', 'failed')
                  AND attempt_count < max_attempts
                ORDER BY task_id
                ",
            )?;
            let rows = statement.query_map([], import_task_row_from_sql)?;
            let mut tasks = Vec::new();
            for row in rows {
                tasks.push(row?);
            }
            Ok(tasks)
        })();

        self.finish_transaction(result)
    }

    /// Updates an import-root task state.
    pub fn update_import_task_state(&self, task_id: i64, state: JobState) -> Result<()> {
        self.connection
            .execute(
                r"
                UPDATE import_task
                SET state = ?2,
                    updated_at = CURRENT_TIMESTAMP
                WHERE task_id = ?1
                ",
                params![task_id, state.as_str()],
            )
            .map(|_| ())
            .map_err(storage_error)
    }

    /// Claims a specific eligible import-root task.
    pub fn claim_import_task(
        &self,
        task_id: i64,
        claim_token: &str,
        now_ms: i64,
        lease_expires_at_ms: i64,
    ) -> Result<Option<ImportTaskClaimRow>> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(storage_error)?;

        let result = (|| {
            self.expire_stale_import_task_leases(now_ms)?;
            let changed = self.connection.execute(
                r"
                UPDATE import_task
                SET state = 'running',
                    attempt_count = attempt_count + 1,
                    last_error = NULL,
                    claim_token = ?2,
                    lease_expires_at_ms = ?3,
                    updated_at = CURRENT_TIMESTAMP
                WHERE task_id = ?1
                  AND attempt_count < max_attempts
                  AND state IN ('queued', 'failed')
                ",
                params![task_id, claim_token, lease_expires_at_ms],
            )?;
            if changed == 0 {
                return Ok(None);
            }
            self.select_import_task_claim(task_id)
        })();

        self.finish_transaction(result)
    }

    /// Claims the next eligible import-root task by stable task order.
    pub fn claim_next_import_task(
        &self,
        claim_token: &str,
        now_ms: i64,
        lease_expires_at_ms: i64,
    ) -> Result<Option<ImportTaskClaimRow>> {
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(storage_error)?;

        let result = (|| {
            self.expire_stale_import_task_leases(now_ms)?;
            let task_id = self
                .connection
                .query_row(
                    r"
                    SELECT task_id
                    FROM import_task
                    WHERE attempt_count < max_attempts
                      AND state IN ('queued', 'failed')
                    ORDER BY task_id
                    LIMIT 1
                    ",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;

            match task_id {
                Some(task_id) => {
                    self.connection.execute(
                        r"
                        UPDATE import_task
                        SET state = 'running',
                            attempt_count = attempt_count + 1,
                            last_error = NULL,
                            claim_token = ?2,
                            lease_expires_at_ms = ?3,
                            updated_at = CURRENT_TIMESTAMP
                        WHERE task_id = ?1
                        ",
                        params![task_id, claim_token, lease_expires_at_ms],
                    )?;
                    self.select_import_task_claim(task_id)
                }
                None => Ok(None),
            }
        })();

        self.finish_transaction(result)
    }

    fn expire_stale_import_task_leases(&self, now_ms: i64) -> rusqlite::Result<()> {
        self.connection.execute(
            r"
            UPDATE import_task
            SET state = CASE
                    WHEN attempt_count >= max_attempts THEN 'permanent_failed'
                    ELSE 'failed'
                END,
                claim_token = NULL,
                lease_expires_at_ms = NULL,
                last_error = 'import task lease expired',
                updated_at = CURRENT_TIMESTAMP
            WHERE state = 'running'
              AND IFNULL(lease_expires_at_ms, -1) <= ?1
            ",
            [now_ms],
        )?;
        Ok(())
    }

    /// Completes a running import-root task only when the claim token matches.
    pub fn complete_claimed_import_task(&self, task_id: i64, claim_token: &str) -> Result<bool> {
        self.connection
            .execute(
                r"
                UPDATE import_task
                SET state = 'completed',
                    claim_token = NULL,
                    lease_expires_at_ms = NULL,
                    last_error = NULL,
                    updated_at = CURRENT_TIMESTAMP
                WHERE task_id = ?1
                  AND state = 'running'
                  AND claim_token = ?2
                ",
                params![task_id, claim_token],
            )
            .map(|changed| changed > 0)
            .map_err(storage_error)
    }

    /// Fails a running import-root task only when the claim token matches.
    pub fn fail_claimed_import_task(
        &self,
        task_id: i64,
        claim_token: &str,
        last_error: &str,
    ) -> Result<bool> {
        self.connection
            .execute(
                r"
                UPDATE import_task
                SET state = CASE
                        WHEN attempt_count >= max_attempts THEN 'permanent_failed'
                        ELSE 'failed'
                    END,
                    claim_token = NULL,
                    lease_expires_at_ms = NULL,
                    last_error = ?3,
                    updated_at = CURRENT_TIMESTAMP
                WHERE task_id = ?1
                  AND state = 'running'
                  AND claim_token = ?2
                ",
                params![task_id, claim_token, last_error],
            )
            .map(|changed| changed > 0)
            .map_err(storage_error)
    }

    /// Returns a concise status summary for local operator commands.
    pub fn status(&self) -> Result<StoreStatus> {
        Ok(StoreStatus {
            schema_version: self.schema_version()?,
            visible_document_count: self.count_rows("document", Some("is_deleted = 0"))?,
            queued_import_task_count: self.count_rows("import_task", Some("state = 'queued'"))?,
            index_state_count: self.count_rows("index_state", None)?,
            searchable_document_count: self
                .count_rows("index_state", Some("status = 'SEARCHABLE'"))?,
            ocr_required_document_count: self
                .count_rows("index_state", Some("status = 'OCR_REQUIRED'"))?,
        })
    }

    fn count_rows(&self, table_name: &str, where_clause: Option<&str>) -> Result<u64> {
        let sql = match where_clause {
            Some(clause) => format!("SELECT COUNT(*) FROM {table_name} WHERE {clause}"),
            None => format!("SELECT COUNT(*) FROM {table_name}"),
        };
        self.connection
            .query_row(&sql, [], |row| row.get::<_, u64>(0))
            .map_err(storage_error)
    }

    fn select_import_task_claim(
        &self,
        task_id: i64,
    ) -> rusqlite::Result<Option<ImportTaskClaimRow>> {
        self.connection
            .query_row(
                r"
                SELECT task_id, root_path, state, max_attempts, attempt_count,
                       last_error, claim_token, lease_expires_at_ms
                FROM import_task
                WHERE task_id = ?1
                  AND state = 'running'
                  AND claim_token IS NOT NULL
                  AND lease_expires_at_ms IS NOT NULL
                LIMIT 1
                ",
                [task_id],
                import_task_claim_row_from_sql,
            )
            .optional()
    }

    fn finish_transaction<T>(&self, result: rusqlite::Result<T>) -> Result<T> {
        match result {
            Ok(value) => {
                self.connection
                    .execute_batch("COMMIT")
                    .map_err(storage_error)?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.connection.execute_batch("ROLLBACK");
                Err(storage_error(error))
            }
        }
    }
}

/// Returns the crate name for smoke tests and workspace metadata.
#[must_use]
pub fn crate_name() -> &'static str {
    "meta-store"
}

fn document_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<DocumentRow> {
    Ok(DocumentRow {
        doc_id: row.get(0)?,
        source_uri: row.get(1)?,
        normalized_path: row.get(2)?,
        file_name: row.get(3)?,
        extension: row.get(4)?,
        byte_size: row.get::<_, u64>(5)?,
        mtime: row.get(6)?,
        content_hash: row.get(7)?,
        text_hash: row.get(8)?,
        is_deleted: row.get::<_, i64>(9)? != 0,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
    })
}

fn ingest_job_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<IngestJobRow> {
    let state = row.get::<_, String>(3)?;
    Ok(IngestJobRow {
        job_id: row.get(0)?,
        doc_id: row.get(1)?,
        job_type: row.get(2)?,
        state: JobState::try_from(state.as_str()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
            )
        })?,
        max_attempts: row.get(4)?,
        attempt_count: row.get(5)?,
        last_error: row.get(6)?,
    })
}

fn import_task_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImportTaskRow> {
    let state = row.get::<_, String>(1)?;
    Ok(ImportTaskRow {
        task_id: row.get(0)?,
        state: JobState::try_from(state.as_str()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
            )
        })?,
        created_at: row.get(2)?,
    })
}

fn import_task_claim_row_from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImportTaskClaimRow> {
    let state = row.get::<_, String>(2)?;
    Ok(ImportTaskClaimRow {
        task_id: row.get(0)?,
        root_path: row.get(1)?,
        state: JobState::try_from(state.as_str()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
            )
        })?,
        max_attempts: row.get(3)?,
        attempt_count: row.get(4)?,
        last_error: row.get(5)?,
        claim_token: row.get(6)?,
        lease_expires_at_ms: row.get(7)?,
    })
}

fn document_extension_label(document: &Document) -> String {
    match &document.extension {
        core_domain::DocumentExtension::Docx => "docx".to_string(),
        core_domain::DocumentExtension::Pdf => "pdf".to_string(),
        core_domain::DocumentExtension::Doc => "doc".to_string(),
        core_domain::DocumentExtension::Txt => "txt".to_string(),
        core_domain::DocumentExtension::Image => "image".to_string(),
        core_domain::DocumentExtension::Other(extension) => extension.clone(),
    }
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn storage_error(error: rusqlite::Error) -> ResumeError {
    storage_diagnostic(format!("SQLite metadata store error: {error}"))
}

fn storage_diagnostic(diagnostic_message: String) -> ResumeError {
    ResumeError::new(
        ErrorKind::Storage,
        false,
        "local metadata store operation failed",
        diagnostic_message,
        RedactionLevel::LocalDiagnostic,
        SourceComponent::MetaStore,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_domain::{DocumentExtension, DocumentId};

    fn test_document(is_deleted: bool) -> Document {
        Document {
            doc_id: DocumentId::new(),
            source_uri: "local://redacted/resume-a.pdf".to_string(),
            normalized_path: "/local/redacted/resume-a.pdf".to_string(),
            file_name: "resume-a.pdf".to_string(),
            extension: DocumentExtension::Pdf,
            byte_size: 128,
            mtime: "2026-01-01T00:00:00Z".to_string(),
            content_hash: Some("hash_content_a".to_string()),
            text_hash: None,
            is_deleted,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn import_task_columns(store: &MetadataStore) -> Result<Vec<String>> {
        let mut statement = store
            .connection
            .prepare("PRAGMA table_info(import_task)")
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(storage_error)?;
        let mut columns = Vec::new();
        for row in rows {
            columns.push(row.map_err(storage_error)?);
        }
        Ok(columns)
    }

    fn import_task_state(store: &MetadataStore, task_id: i64) -> Result<JobState> {
        let state = store
            .connection
            .query_row(
                "SELECT state FROM import_task WHERE task_id = ?1",
                [task_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(storage_error)?;
        JobState::try_from(state.as_str()).map_err(storage_diagnostic)
    }

    fn import_task_last_error(store: &MetadataStore, task_id: i64) -> Result<Option<String>> {
        store
            .connection
            .query_row(
                "SELECT last_error FROM import_task WHERE task_id = ?1",
                [task_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .map_err(storage_error)
    }

    #[test]
    fn migrations_are_idempotent_and_record_schema_version() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;

        store.run_migrations()?;
        store.run_migrations()?;

        assert_eq!(store.schema_version()?, 3);
        Ok(())
    }

    #[test]
    fn migrations_reject_future_schema_versions_without_downgrade() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store
            .connection
            .execute_batch("PRAGMA user_version = 4;")
            .map_err(storage_error)?;

        let Err(error) = store.run_migrations() else {
            return Err(storage_diagnostic(
                "future schema migration unexpectedly succeeded".to_string(),
            ));
        };

        assert_eq!(store.schema_version()?, 4);
        assert!(error
            .local_diagnostic_message()
            .contains("newer metadata schema version"));
        Ok(())
    }

    #[test]
    fn status_counts_visible_documents_import_tasks_and_indexes() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let document = test_document(false);

        store.upsert_document(&document)?;
        store.enqueue_import_root(Path::new("/local/redacted/import-root"))?;
        store.upsert_index_state("tantivy", None, "missing", None)?;

        let status = store.status()?;

        assert_eq!(status.schema_version, 3);
        assert_eq!(status.visible_document_count, 1);
        assert_eq!(status.queued_import_task_count, 1);
        assert_eq!(status.index_state_count, 1);
        assert_eq!(status.searchable_document_count, 0);
        assert_eq!(status.ocr_required_document_count, 0);
        Ok(())
    }

    #[test]
    fn import_task_debug_redacts_root_path() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;

        let task_id = store.enqueue_import_root(Path::new("/synthetic/private/root"))?;
        let tasks = store.queued_import_tasks()?;

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_id, task_id);
        assert_eq!(tasks[0].state, JobState::Queued);
        let debug = format!("{:?}", tasks[0]);
        assert!(debug.contains("root_path: \"<redacted>\""));
        assert!(!debug.contains("/synthetic/private/root"));
        Ok(())
    }

    #[test]
    fn migrations_upgrade_v2_import_tasks_to_claim_schema() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store
            .connection
            .execute_batch(
                r"
                CREATE TABLE import_task (
                    task_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    root_path TEXT NOT NULL,
                    state TEXT NOT NULL CHECK (
                        state IN (
                            'queued',
                            'running',
                            'failed',
                            'completed',
                            'cancelled',
                            'permanent_failed'
                        )
                    ),
                    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
                INSERT INTO import_task (root_path, state)
                VALUES ('/synthetic/private/root', 'queued');
                PRAGMA user_version = 2;
                ",
            )
            .map_err(storage_error)?;

        store.run_migrations()?;

        assert_eq!(store.schema_version()?, 3);
        let columns = import_task_columns(&store)?;
        for column in [
            "max_attempts",
            "attempt_count",
            "last_error",
            "claim_token",
            "lease_expires_at_ms",
        ] {
            assert!(columns.contains(&column.to_string()), "missing {column}");
        }
        let claim = store.claim_import_task(1, "claim_secret_token", 1_000, 2_000)?;
        assert_eq!(claim.map(|row| row.task_id), Some(1));
        Ok(())
    }

    #[test]
    fn migrations_upgrade_v1_by_creating_v3_import_task_schema() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store
            .connection
            .execute_batch("PRAGMA user_version = 1;")
            .map_err(storage_error)?;

        store.run_migrations()?;

        assert_eq!(store.schema_version()?, 3);
        let columns = import_task_columns(&store)?;
        for column in [
            "root_path",
            "state",
            "max_attempts",
            "attempt_count",
            "last_error",
            "claim_token",
            "lease_expires_at_ms",
        ] {
            assert!(columns.contains(&column.to_string()), "missing {column}");
        }
        Ok(())
    }

    #[test]
    fn claiming_import_task_sets_lease_and_attempt_without_exposing_root_or_token() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let task_id = store.enqueue_import_root(Path::new("/synthetic/private/root"))?;

        let claim = store
            .claim_import_task(task_id, "claim_secret_token", 1_000, 2_000)?
            .ok_or_else(|| storage_diagnostic("claim unexpectedly missing".to_string()))?;

        assert_eq!(claim.task_id, task_id);
        assert_eq!(claim.state, JobState::Running);
        assert_eq!(claim.max_attempts, 3);
        assert_eq!(claim.attempt_count, 1);
        assert_eq!(claim.lease_expires_at_ms, 2_000);
        let debug = format!("{claim:?}");
        assert!(debug.contains("root_path: \"<redacted>\""));
        assert!(debug.contains("claim_token: \"<redacted>\""));
        assert!(!debug.contains("/synthetic/private/root"));
        assert!(!debug.contains("claim_secret_token"));
        Ok(())
    }

    #[test]
    fn claim_next_import_task_skips_unexpired_running_and_reclaims_expired_work() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let running = store.enqueue_import_root(Path::new("/synthetic/private/running"))?;
        let queued = store.enqueue_import_root(Path::new("/synthetic/private/queued"))?;

        store.claim_import_task(running, "first_token", 1_000, 5_000)?;
        let next = store
            .claim_next_import_task("next_token", 2_000, 6_000)?
            .ok_or_else(|| storage_diagnostic("next claim unexpectedly missing".to_string()))?;

        assert_eq!(next.task_id, queued);
        assert_eq!(next.attempt_count, 1);

        let reclaimed = store
            .claim_next_import_task("reclaim_token", 5_001, 7_000)?
            .ok_or_else(|| storage_diagnostic("expired claim unexpectedly missing".to_string()))?;

        assert_eq!(reclaimed.task_id, running);
        assert_eq!(reclaimed.attempt_count, 2);
        Ok(())
    }

    #[test]
    fn claimable_import_tasks_expire_stale_running_leases_without_paths() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let expired = store.enqueue_import_root(Path::new("/synthetic/private/expired"))?;
        let running = store.enqueue_import_root(Path::new("/synthetic/private/running"))?;
        let queued = store.enqueue_import_root(Path::new("/synthetic/private/queued"))?;

        store.claim_import_task(expired, "expired_token", 1_000, 2_000)?;
        store.claim_import_task(running, "running_token", 1_000, 5_000)?;

        let claimable = store.claimable_import_tasks(2_001)?;
        let ids = claimable
            .iter()
            .map(|task| task.task_id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec![expired, queued]);
        assert_eq!(import_task_state(&store, expired)?, JobState::Failed);
        assert_eq!(import_task_state(&store, running)?, JobState::Running);
        let debug = format!("{claimable:?}");
        assert!(!debug.contains("/synthetic/private"));
        assert!(!debug.contains("expired_token"));
        assert!(!debug.contains("running_token"));
        Ok(())
    }

    #[test]
    fn complete_and_fail_claimed_import_task_require_matching_token() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let complete_id = store.enqueue_import_root(Path::new("/synthetic/private/complete"))?;
        let fail_id = store.enqueue_import_root(Path::new("/synthetic/private/fail"))?;

        store.claim_import_task(complete_id, "complete_token", 1_000, 2_000)?;
        assert!(!store.complete_claimed_import_task(complete_id, "wrong_token")?);
        assert!(store.complete_claimed_import_task(complete_id, "complete_token")?);

        store.claim_import_task(fail_id, "fail_token", 1_000, 2_000)?;
        assert!(!store.fail_claimed_import_task(fail_id, "wrong_token", "private error")?);
        let private_error = "private error containing /synthetic/private/fail";
        assert!(store.fail_claimed_import_task(fail_id, "fail_token", private_error)?);
        assert_eq!(
            import_task_last_error(&store, fail_id)?.as_deref(),
            Some(private_error)
        );

        let debug = format!(
            "{:?}",
            store.claim_import_task(fail_id, "retry", 2_001, 3_000)?
        );
        assert!(!debug.contains("private error"));
        assert!(!debug.contains("/synthetic/private/fail"));
        Ok(())
    }

    #[test]
    fn failed_claim_becomes_permanent_after_attempt_budget_is_exhausted() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let task_id = store.enqueue_import_root(Path::new("/synthetic/private/root"))?;

        store
            .connection
            .execute(
                "UPDATE import_task SET max_attempts = 1 WHERE task_id = ?1",
                [task_id],
            )
            .map_err(storage_error)?;
        store.claim_import_task(task_id, "claim_token", 1_000, 2_000)?;

        assert!(store.fail_claimed_import_task(
            task_id,
            "claim_token",
            "private exhausted error"
        )?);
        assert!(store
            .claim_import_task(task_id, "retry_token", 2_001, 3_000)?
            .is_none());
        assert_eq!(
            import_task_state(&store, task_id)?,
            JobState::PermanentFailed
        );
        Ok(())
    }

    #[test]
    fn expired_final_attempt_import_task_becomes_permanent_before_next_claim() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let task_id = store.enqueue_import_root(Path::new("/synthetic/private/root"))?;

        store
            .connection
            .execute(
                "UPDATE import_task SET max_attempts = 1 WHERE task_id = ?1",
                [task_id],
            )
            .map_err(storage_error)?;
        let claim = store
            .claim_import_task(task_id, "claim_token", 1_000, 2_000)?
            .ok_or_else(|| storage_diagnostic("claim unexpectedly missing".to_string()))?;
        assert_eq!(claim.attempt_count, 1);

        assert!(store
            .claim_next_import_task("next_token", 2_001, 3_000)?
            .is_none());
        assert!(store
            .claim_import_task(task_id, "retry_token", 2_001, 3_000)?
            .is_none());
        assert_eq!(
            import_task_state(&store, task_id)?,
            JobState::PermanentFailed
        );
        assert_eq!(
            import_task_last_error(&store, task_id)?.as_deref(),
            Some("import task lease expired")
        );
        Ok(())
    }

    #[test]
    fn import_task_claim_debug_redacts_root_token_and_last_error() {
        let row = ImportTaskClaimRow {
            task_id: 42,
            root_path: "/synthetic/private/root".to_string(),
            state: JobState::Running,
            max_attempts: 3,
            attempt_count: 1,
            last_error: Some("private failure under /synthetic/private/root".to_string()),
            claim_token: "private_claim_token".to_string(),
            lease_expires_at_ms: 2_000,
        };

        let debug = format!("{row:?}");

        assert!(debug.contains("root_path: \"<redacted>\""));
        assert!(debug.contains("last_error: Some(\"<redacted>\")"));
        assert!(debug.contains("claim_token: \"<redacted>\""));
        assert!(!debug.contains("/synthetic/private/root"));
        assert!(!debug.contains("private failure"));
        assert!(!debug.contains("private_claim_token"));
    }

    #[test]
    fn document_debug_redacts_local_paths() {
        let row = DocumentRow {
            doc_id: "doc_debug".to_string(),
            source_uri: "file://synthetic/private/document.pdf".to_string(),
            normalized_path: "/synthetic/private/document.pdf".to_string(),
            file_name: "document.pdf".to_string(),
            extension: "pdf".to_string(),
            byte_size: 128,
            mtime: "2026-01-01T00:00:00Z".to_string(),
            content_hash: Some("hash_content_a".to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let debug = format!("{row:?}");

        assert!(debug.contains("source_uri: \"<redacted>\""));
        assert!(debug.contains("normalized_path: \"<redacted>\""));
        assert!(!debug.contains("/synthetic/private"));
        assert!(!debug.contains("document.pdf"));
    }

    #[test]
    fn ingest_job_debug_redacts_last_error() {
        let row = IngestJobRow {
            job_id: 7,
            doc_id: "doc_debug".to_string(),
            job_type: "parse".to_string(),
            state: JobState::Failed,
            max_attempts: 3,
            attempt_count: 1,
            last_error: Some("failed to read synthetic private document".to_string()),
        };

        let debug = format!("{row:?}");

        assert!(debug.contains("last_error: Some(\"<redacted>\")"));
        assert!(!debug.contains("synthetic private document"));
    }

    #[test]
    fn upsert_document_rediscovery_uses_normalized_path_as_dedupe_key() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let first = test_document(false);
        let mut rediscovered = first.clone();
        rediscovered.doc_id = DocumentId::new();
        rediscovered.source_uri = "local://redacted/resume-a-renamed.pdf".to_string();
        rediscovered.file_name = "resume-a-renamed.pdf".to_string();
        rediscovered.byte_size = 256;
        rediscovered.updated_at = "2026-01-02T00:00:00Z".to_string();
        let original_id = first.doc_id.to_string();

        store.upsert_document(&first)?;
        store.upsert_document(&rediscovered)?;

        let documents = store.visible_documents()?;
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].doc_id, original_id);
        assert_eq!(documents[0].file_name, "resume-a-renamed.pdf");
        assert_eq!(documents[0].byte_size, 256);
        Ok(())
    }

    #[test]
    fn upsert_document_rediscovery_preserves_deleted_tombstone() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let first = test_document(false);
        let mut rediscovered = first.clone();
        rediscovered.doc_id = DocumentId::new();
        rediscovered.file_name = "resume-a-rediscovered.pdf".to_string();
        rediscovered.byte_size = 512;
        rediscovered.is_deleted = false;
        let original_id = first.doc_id.to_string();

        store.upsert_document(&first)?;
        assert!(store.mark_document_deleted(&original_id)?);
        store.upsert_document(&rediscovered)?;

        assert!(store.visible_documents()?.is_empty());
        let stored = store
            .document_by_normalized_path(&first.normalized_path)?
            .ok_or_else(|| storage_diagnostic("rediscovered document missing".to_string()))?;
        assert_eq!(stored.doc_id, original_id);
        assert!(stored.is_deleted);
        assert_eq!(stored.file_name, "resume-a-rediscovered.pdf");
        assert_eq!(stored.byte_size, 512);
        Ok(())
    }

    #[test]
    fn retryable_jobs_for_recovery_include_interrupted_retryable_work() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;

        let retryable = store.insert_ingest_job("doc_retry", "parse", JobState::Queued, 3, 0)?;
        let running = store.insert_ingest_job("doc_running", "parse", JobState::Running, 3, 1)?;
        let exhausted =
            store.insert_ingest_job("doc_exhausted", "parse", JobState::Failed, 2, 2)?;
        let completed = store.insert_ingest_job("doc_done", "parse", JobState::Completed, 3, 0)?;
        let cancelled =
            store.insert_ingest_job("doc_cancelled", "parse", JobState::Cancelled, 3, 0)?;
        let permanent_failed = store.insert_ingest_job(
            "doc_permanent_failed",
            "parse",
            JobState::PermanentFailed,
            3,
            0,
        )?;

        store.update_job_state(
            retryable,
            JobState::Failed,
            1,
            Some("synthetic transient failure"),
        )?;
        store.update_job_state(running, JobState::Running, 2, Some("worker stopped"))?;

        let jobs = store.retryable_jobs_for_recovery()?;
        let job_ids = jobs.iter().map(|job| job.job_id).collect::<Vec<_>>();

        assert_eq!(job_ids, vec![retryable, running]);
        assert!(!job_ids.contains(&exhausted));
        assert!(!job_ids.contains(&completed));
        assert!(!job_ids.contains(&cancelled));
        assert!(!job_ids.contains(&permanent_failed));
        Ok(())
    }

    #[test]
    fn ingest_job_schema_rejects_invalid_state_values() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;

        let result = store.connection.execute(
            r"
            INSERT INTO ingest_job (doc_id, job_type, state, max_attempts, attempt_count)
            VALUES ('doc_invalid', 'parse', 'not_a_state', 3, 0)
            ",
            [],
        );

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn visible_documents_excludes_deleted_documents_by_default() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let visible = test_document(false);
        let mut deleted = test_document(true);
        deleted.normalized_path = "/local/redacted/resume-b.pdf".to_string();
        let visible_id = visible.doc_id.to_string();
        let deleted_id = deleted.doc_id.to_string();

        store.upsert_document(&visible)?;
        store.upsert_document(&deleted)?;

        let documents = store.visible_documents()?;
        let document_ids = documents
            .iter()
            .map(|document| document.doc_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(document_ids, vec![visible_id.as_str()]);
        assert!(!document_ids.contains(&deleted_id.as_str()));
        Ok(())
    }

    #[test]
    fn mark_document_deleted_hides_metadata_and_searchable_clean_text() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let document = test_document(false);
        let doc_id = document.doc_id.to_string();

        store.upsert_document(&document)?;
        store.upsert_resume_version(ParsedResumeRecord {
            version_id: "ver_deleted",
            doc_id: &doc_id,
            parse_version: "test",
            schema_version: "test",
            raw_text: Some("raw text that must stay local"),
            clean_text: Some("clean Java text that must not be resurrected"),
            visibility: "SEARCHABLE",
        })?;

        assert!(store.mark_document_deleted(&doc_id)?);

        assert!(store.visible_documents()?.is_empty());
        assert_eq!(store.clean_text_by_doc_id(&doc_id)?, None);
        assert_eq!(
            store.document_by_doc_id(&doc_id)?.map(|row| row.is_deleted),
            Some(true)
        );
        Ok(())
    }

    #[test]
    fn bulk_write_commit_and_rollback_control_visibility() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let rolled_back = test_document(false);
        let committed = test_document(false);

        store.begin_bulk_write()?;
        store.upsert_document(&rolled_back)?;
        store.rollback_bulk_write();
        assert!(store.visible_documents()?.is_empty());

        store.begin_bulk_write()?;
        store.upsert_document(&committed)?;
        store.commit_bulk_write()?;

        let visible = store.visible_documents()?;
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].doc_id, committed.doc_id.to_string());
        Ok(())
    }

    #[test]
    fn clean_text_lookup_excludes_previously_deleted_document_rows() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let document = test_document(true);
        let doc_id = document.doc_id.to_string();

        store.upsert_document(&document)?;
        store.upsert_resume_version(ParsedResumeRecord {
            version_id: "ver_deleted_existing",
            doc_id: &doc_id,
            parse_version: "test",
            schema_version: "test",
            raw_text: Some("deleted raw"),
            clean_text: Some("deleted clean text"),
            visibility: "SEARCHABLE",
        })?;

        assert_eq!(store.clean_text_by_doc_id(&doc_id)?, None);
        Ok(())
    }

    #[test]
    fn clean_text_lookup_returns_latest_searchable_text_for_doc_id() -> Result<()> {
        let store = MetadataStore::open_in_memory()?;
        store.run_migrations()?;
        let document = test_document(false);
        let doc_id = document.doc_id.to_string();

        store.upsert_document(&document)?;
        store.upsert_resume_version(ParsedResumeRecord {
            version_id: "ver_old",
            doc_id: &doc_id,
            parse_version: "test",
            schema_version: "test",
            raw_text: Some("old raw"),
            clean_text: Some("old Java associate text"),
            visibility: "SEARCHABLE",
        })?;
        store.upsert_resume_version(ParsedResumeRecord {
            version_id: "ver_new",
            doc_id: &doc_id,
            parse_version: "test",
            schema_version: "test",
            raw_text: Some("new raw"),
            clean_text: Some("new Java bachelor text"),
            visibility: "SEARCHABLE",
        })?;

        assert_eq!(
            store.clean_text_by_doc_id(&doc_id)?,
            Some("new Java bachelor text".to_string())
        );
        assert_eq!(store.clean_text_by_doc_id("missing-doc")?, None);
        Ok(())
    }
}
