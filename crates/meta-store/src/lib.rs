use std::cell::RefCell;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

pub use core_domain::{
    CandidateId, Document, DocumentId, DocumentStatus, FileExtension, IndexStateStatus,
    IngestJobId, IngestJobKind, IngestJobStatus, ResumeVersion, ResumeVersionId, ResumeVisibility,
    UnixTimestamp,
};
use rusqlite::{params, Connection, Row};

const SCHEMA_VERSION_V1: u32 = 1;
const INDEX_STATE_KEY: &str = "default";
const DOCUMENT_COLUMNS: &str = "\
    id, source_uri, normalized_path, file_name, extension, byte_size, mtime_seconds, \
    content_hash, text_hash, is_deleted, created_at_seconds, updated_at_seconds, status";
const RESUME_VERSION_COLUMNS: &str = "\
    id, document_id, candidate_id, parse_version, schema_version, language_set_json, \
    page_count, raw_text, clean_text, quality_score, visibility";
const INGEST_JOB_COLUMNS: &str = "\
    id, document_id, resume_version_id, kind, status, attempt_count, max_attempts, \
    queued_at_seconds, started_at_seconds, finished_at_seconds, updated_at_seconds";

pub fn crate_name() -> &'static str {
    "meta-store"
}

pub type Result<T> = std::result::Result<T, MetaStoreError>;

pub struct MetaStore {
    connection: RefCell<Connection>,
}

impl MetaStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path).map_err(MetaStoreError::storage)?;
        Self::from_connection(connection, true)
    }

    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory().map_err(MetaStoreError::storage)?;
        Self::from_connection(connection, false)
    }

    fn from_connection(connection: Connection, file_backed: bool) -> Result<Self> {
        connection
            .busy_timeout(Duration::from_millis(5_000))
            .map_err(MetaStoreError::storage)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(MetaStoreError::storage)?;
        if file_backed {
            connection
                .pragma_update(None, "journal_mode", "WAL")
                .map_err(MetaStoreError::storage)?;
        }

        Ok(Self {
            connection: RefCell::new(connection),
        })
    }

    pub fn run_migrations(&self) -> Result<MigrationReport> {
        let mut connection = self.connection.borrow_mut();
        connection
            .execute_batch(
                "\
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at_seconds INTEGER NOT NULL
                );",
            )
            .map_err(MetaStoreError::migration)?;

        let already_applied = migration_applied(&connection, SCHEMA_VERSION_V1)?;
        let mut applied_versions = Vec::new();

        if !already_applied {
            let transaction = connection
                .transaction()
                .map_err(MetaStoreError::migration)?;
            transaction
                .execute_batch(SCHEMA_V1)
                .map_err(MetaStoreError::migration)?;
            transaction
                .execute(
                    "INSERT INTO schema_migrations (version, applied_at_seconds) VALUES (?1, ?2)",
                    params![i64::from(SCHEMA_VERSION_V1), 0_i64],
                )
                .map_err(MetaStoreError::migration)?;
            transaction.commit().map_err(MetaStoreError::migration)?;
            applied_versions.push(SCHEMA_VERSION_V1);
        }

        Ok(MigrationReport { applied_versions })
    }

    pub fn schema_version(&self) -> Result<u32> {
        if !self.schema_table_exists("schema_migrations")? {
            return Ok(0);
        }

        let connection = self.connection.borrow();
        let version = connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;

        u32::try_from(version)
            .map_err(|_| MetaStoreError::invalid_value("schema_migrations.version"))
    }

    pub fn schema_table_exists(&self, table_name: &str) -> Result<bool> {
        let connection = self.connection.borrow();
        let exists = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                params![table_name],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;

        Ok(exists == 1)
    }

    pub fn foreign_keys_enabled(&self) -> Result<bool> {
        let connection = self.connection.borrow();
        let enabled = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .map_err(MetaStoreError::storage)?;

        Ok(enabled == 1)
    }

    pub fn busy_timeout_millis(&self) -> Result<u64> {
        let connection = self.connection.borrow();
        let timeout = connection
            .query_row("PRAGMA busy_timeout", [], |row| row.get::<_, i64>(0))
            .map_err(MetaStoreError::storage)?;

        u64::try_from(timeout).map_err(|_| MetaStoreError::invalid_value("pragma.busy_timeout"))
    }

    pub fn journal_mode(&self) -> Result<String> {
        let connection = self.connection.borrow();
        connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)
            .map(|mode| mode.to_ascii_lowercase())
    }

    pub fn upsert_document(&self, document: &Document) -> Result<()> {
        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO document (
                    id, source_uri, normalized_path, file_name, extension, byte_size,
                    mtime_seconds, content_hash, text_hash, is_deleted, created_at_seconds,
                    updated_at_seconds, status
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(id) DO UPDATE SET
                    source_uri = excluded.source_uri,
                    normalized_path = excluded.normalized_path,
                    file_name = excluded.file_name,
                    extension = excluded.extension,
                    byte_size = excluded.byte_size,
                    mtime_seconds = excluded.mtime_seconds,
                    content_hash = excluded.content_hash,
                    text_hash = excluded.text_hash,
                    is_deleted = excluded.is_deleted,
                    created_at_seconds = excluded.created_at_seconds,
                    updated_at_seconds = excluded.updated_at_seconds,
                    status = excluded.status",
                params![
                    document.id.as_str(),
                    document.source_uri,
                    document.normalized_path,
                    document.file_name,
                    file_extension_to_storage(&document.extension),
                    u64_to_i64(document.byte_size, "document.byte_size")?,
                    document.mtime.as_unix_seconds(),
                    document.content_hash,
                    document.text_hash,
                    bool_to_i64(document.is_deleted),
                    document.created_at.as_unix_seconds(),
                    document.updated_at.as_unix_seconds(),
                    document_status_to_storage(document.status),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn document_by_id(&self, id: &DocumentId) -> Result<Option<Document>> {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {DOCUMENT_COLUMNS} FROM document WHERE id = ?1");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![id.as_str()])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_document(row)?)),
            None => Ok(None),
        }
    }

    pub fn visible_documents(&self) -> Result<Vec<Document>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "SELECT {DOCUMENT_COLUMNS} FROM document WHERE is_deleted = 0 AND status <> ?1 ORDER BY id"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![document_status_to_storage(DocumentStatus::Deleted)])
            .map_err(MetaStoreError::storage)?;
        let mut documents = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            documents.push(read_document(row)?);
        }

        Ok(documents)
    }

    pub fn upsert_resume_version(&self, version: &ResumeVersion) -> Result<()> {
        let language_set_json = serde_json::to_string(&version.language_set)
            .map_err(|_| MetaStoreError::invalid_value("resume_version.language_set"))?;
        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO resume_version (
                    id, document_id, candidate_id, parse_version, schema_version,
                    language_set_json, page_count, raw_text, clean_text, quality_score, visibility
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                ON CONFLICT(id) DO UPDATE SET
                    document_id = excluded.document_id,
                    candidate_id = excluded.candidate_id,
                    parse_version = excluded.parse_version,
                    schema_version = excluded.schema_version,
                    language_set_json = excluded.language_set_json,
                    page_count = excluded.page_count,
                    raw_text = excluded.raw_text,
                    clean_text = excluded.clean_text,
                    quality_score = excluded.quality_score,
                    visibility = excluded.visibility",
                params![
                    version.id.as_str(),
                    version.document_id.as_str(),
                    version.candidate_id.as_ref().map(CandidateId::as_str),
                    version.parse_version,
                    version.schema_version,
                    language_set_json,
                    version.page_count.map(i64::from),
                    version.raw_text,
                    version.clean_text,
                    version.quality_score.map(f64::from),
                    resume_visibility_to_storage(version.visibility),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn resume_version_by_id(&self, id: &ResumeVersionId) -> Result<Option<ResumeVersion>> {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {RESUME_VERSION_COLUMNS} FROM resume_version WHERE id = ?1");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![id.as_str()])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_resume_version(row)?)),
            None => Ok(None),
        }
    }

    pub fn insert_ingest_job(&self, job: &IngestJob) -> Result<()> {
        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO ingest_job (
                    id, document_id, resume_version_id, kind, status, attempt_count, max_attempts,
                    queued_at_seconds, started_at_seconds, finished_at_seconds, updated_at_seconds
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    job.id.as_str(),
                    job.document_id.as_str(),
                    job.resume_version_id.as_ref().map(ResumeVersionId::as_str),
                    ingest_job_kind_to_storage(job.kind),
                    ingest_job_status_to_storage(job.status),
                    u32_to_i64(job.attempt_count),
                    u32_to_i64(job.max_attempts),
                    job.queued_at.as_unix_seconds(),
                    job.started_at.map(UnixTimestamp::as_unix_seconds),
                    job.finished_at.map(UnixTimestamp::as_unix_seconds),
                    job.updated_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn ingest_job_by_id(&self, id: &IngestJobId) -> Result<Option<IngestJob>> {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {INGEST_JOB_COLUMNS} FROM ingest_job WHERE id = ?1");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![id.as_str()])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_ingest_job(row)?)),
            None => Ok(None),
        }
    }

    pub fn update_job_status(
        &self,
        id: &IngestJobId,
        status: IngestJobStatus,
        updated_at: UnixTimestamp,
    ) -> Result<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let current_status = {
            let mut statement = transaction
                .prepare("SELECT status FROM ingest_job WHERE id = ?1")
                .map_err(MetaStoreError::storage)?;
            let mut rows = statement
                .query(params![id.as_str()])
                .map_err(MetaStoreError::storage)?;

            match rows.next().map_err(MetaStoreError::storage)? {
                Some(row) => ingest_job_status_from_storage(&read_string(row, 0)?)?,
                None => return Err(MetaStoreError::not_found("ingest_job")),
            }
        };

        if !job_status_transition_allowed(current_status, status) {
            return Err(MetaStoreError::invalid_transition());
        }

        let updated_at_seconds = updated_at.as_unix_seconds();
        let changed = transaction
            .execute(
                "\
                UPDATE ingest_job
                SET
                    status = ?1,
                    started_at_seconds = CASE
                        WHEN ?1 = ?2 THEN ?5
                        ELSE started_at_seconds
                    END,
                    finished_at_seconds = CASE
                        WHEN ?1 = ?2 THEN NULL
                        WHEN ?1 IN (?3, ?4, ?6) THEN ?5
                        ELSE finished_at_seconds
                    END,
                    updated_at_seconds = ?5
                WHERE id = ?7 AND status = ?8",
                params![
                    ingest_job_status_to_storage(status),
                    ingest_job_status_to_storage(IngestJobStatus::Running),
                    ingest_job_status_to_storage(IngestJobStatus::Completed),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                    updated_at_seconds,
                    ingest_job_status_to_storage(IngestJobStatus::FailedPermanent),
                    id.as_str(),
                    ingest_job_status_to_storage(current_status),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        if changed == 0 {
            return Err(MetaStoreError::invalid_transition());
        }

        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn claim_next_job(&self, now: UnixTimestamp) -> Result<Option<IngestJob>> {
        let claimed_id = {
            let mut connection = self.connection.borrow_mut();
            let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
            let candidate_id = {
                let mut statement = transaction
                    .prepare(
                        "\
                        SELECT id
                        FROM ingest_job
                        WHERE status IN (?1, ?2)
                            OR (status = ?3 AND attempt_count < max_attempts)
                        ORDER BY queued_at_seconds, rowid
                        LIMIT 1",
                    )
                    .map_err(MetaStoreError::storage)?;
                let mut rows = statement
                    .query(params![
                        ingest_job_status_to_storage(IngestJobStatus::Queued),
                        ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                        ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                    ])
                    .map_err(MetaStoreError::storage)?;

                match rows.next().map_err(MetaStoreError::storage)? {
                    Some(row) => Some(read_string(row, 0)?),
                    None => None,
                }
            };

            let Some(candidate_id) = candidate_id else {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(None);
            };

            let now_seconds = now.as_unix_seconds();
            let changed = transaction
                .execute(
                    "\
                    UPDATE ingest_job
                    SET
                        status = ?1,
                        attempt_count = attempt_count + 1,
                        started_at_seconds = ?2,
                        finished_at_seconds = NULL,
                        updated_at_seconds = ?2
                    WHERE id = ?3
                        AND (
                            status IN (?4, ?5)
                            OR (status = ?6 AND attempt_count < max_attempts)
                        )",
                    params![
                        ingest_job_status_to_storage(IngestJobStatus::Running),
                        now_seconds,
                        candidate_id,
                        ingest_job_status_to_storage(IngestJobStatus::Queued),
                        ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                        ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                    ],
                )
                .map_err(MetaStoreError::storage)?;

            if changed == 0 {
                transaction.commit().map_err(MetaStoreError::storage)?;
                return Ok(None);
            }

            transaction.commit().map_err(MetaStoreError::storage)?;
            candidate_id
        };

        let claimed_id = IngestJobId::from_str(&claimed_id)
            .map_err(|_| MetaStoreError::invalid_value("ingest_job.id"))?;

        self.ingest_job_by_id(&claimed_id)?
            .ok_or_else(|| MetaStoreError::not_found("ingest_job"))
            .map(Some)
    }

    pub fn retryable_jobs(&self) -> Result<Vec<IngestJob>> {
        self.query_jobs(
            "\
            WHERE status IN (?1, ?2)
                OR (status = ?3 AND attempt_count < max_attempts)
            ORDER BY rowid",
            params![
                ingest_job_status_to_storage(IngestJobStatus::Queued),
                ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
            ],
        )
    }

    pub fn jobs_requiring_recovery(&self) -> Result<Vec<IngestJob>> {
        self.query_jobs(
            "\
            WHERE status IN (?1, ?2)
                OR (status = ?3 AND attempt_count < max_attempts)
            ORDER BY rowid",
            params![
                ingest_job_status_to_storage(IngestJobStatus::Running),
                ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
            ],
        )
    }

    pub fn upsert_index_state(&self, state: &IndexState) -> Result<()> {
        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO index_state (
                    state_key, manifest_version, snapshot_token, status, updated_at_seconds
                )
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(state_key) DO UPDATE SET
                    manifest_version = excluded.manifest_version,
                    snapshot_token = excluded.snapshot_token,
                    status = excluded.status,
                    updated_at_seconds = excluded.updated_at_seconds",
                params![
                    INDEX_STATE_KEY,
                    state.manifest_version,
                    state.snapshot_token,
                    index_state_status_to_storage(state.status),
                    state.updated_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn index_state(&self) -> Result<Option<IndexState>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "\
                SELECT manifest_version, snapshot_token, status, updated_at_seconds
                FROM index_state
                WHERE state_key = ?1",
            )
            .map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![INDEX_STATE_KEY])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_index_state(row)?)),
            None => Ok(None),
        }
    }

    fn query_jobs<P>(&self, filter_clause: &str, params: P) -> Result<Vec<IngestJob>>
    where
        P: rusqlite::Params,
    {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {INGEST_JOB_COLUMNS} FROM ingest_job {filter_clause}");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement.query(params).map_err(MetaStoreError::storage)?;
        let mut jobs = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            jobs.push(read_ingest_job(row)?);
        }

        Ok(jobs)
    }
}

impl fmt::Debug for MetaStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetaStore")
            .field("connection", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationReport {
    applied_versions: Vec<u32>,
}

impl MigrationReport {
    pub fn applied_versions(&self) -> &[u32] {
        &self.applied_versions
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IngestJob {
    pub id: IngestJobId,
    pub document_id: DocumentId,
    pub resume_version_id: Option<ResumeVersionId>,
    pub kind: IngestJobKind,
    pub status: IngestJobStatus,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub queued_at: UnixTimestamp,
    pub started_at: Option<UnixTimestamp>,
    pub finished_at: Option<UnixTimestamp>,
    pub updated_at: UnixTimestamp,
}

impl fmt::Debug for IngestJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IngestJob")
            .field("id", &self.id)
            .field("document_id", &self.document_id)
            .field("resume_version_id", &self.resume_version_id)
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("attempt_count", &self.attempt_count)
            .field("max_attempts", &self.max_attempts)
            .field("queued_at", &self.queued_at)
            .field("started_at", &self.started_at)
            .field("finished_at", &self.finished_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IndexState {
    pub manifest_version: String,
    pub snapshot_token: Option<String>,
    pub status: IndexStateStatus,
    pub updated_at: UnixTimestamp,
}

impl fmt::Debug for IndexState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IndexState")
            .field("manifest_version", &"<redacted>")
            .field(
                "snapshot_token",
                &self.snapshot_token.as_ref().map(|_| "<redacted>"),
            )
            .field("status", &self.status)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MetaStoreError {
    kind: MetaStoreErrorKind,
}

impl MetaStoreError {
    fn storage(_error: rusqlite::Error) -> Self {
        Self {
            kind: MetaStoreErrorKind::Storage,
        }
    }

    fn migration(_error: rusqlite::Error) -> Self {
        Self {
            kind: MetaStoreErrorKind::Migration,
        }
    }

    fn invalid_value(field: &'static str) -> Self {
        Self {
            kind: MetaStoreErrorKind::InvalidPersistedValue { field },
        }
    }

    fn not_found(entity: &'static str) -> Self {
        Self {
            kind: MetaStoreErrorKind::NotFound { entity },
        }
    }

    fn invalid_transition() -> Self {
        Self {
            kind: MetaStoreErrorKind::InvalidTransition,
        }
    }
}

impl fmt::Debug for MetaStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetaStoreError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for MetaStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            MetaStoreErrorKind::Storage => formatter.write_str("metadata store operation failed"),
            MetaStoreErrorKind::Migration => {
                formatter.write_str("metadata schema migration failed")
            }
            MetaStoreErrorKind::InvalidPersistedValue { field } => {
                write!(
                    formatter,
                    "metadata store contains an invalid value for {field}"
                )
            }
            MetaStoreErrorKind::NotFound { entity } => {
                write!(
                    formatter,
                    "metadata store record was not found for {entity}"
                )
            }
            MetaStoreErrorKind::InvalidTransition => {
                formatter.write_str("metadata store job status transition is invalid")
            }
        }
    }
}

impl std::error::Error for MetaStoreError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MetaStoreErrorKind {
    Storage,
    Migration,
    InvalidPersistedValue { field: &'static str },
    NotFound { entity: &'static str },
    InvalidTransition,
}

const SCHEMA_V1: &str = r#"
CREATE TABLE document (
    id TEXT PRIMARY KEY,
    source_uri TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    file_name TEXT NOT NULL,
    extension TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    mtime_seconds INTEGER NOT NULL,
    content_hash TEXT,
    text_hash TEXT,
    is_deleted INTEGER NOT NULL DEFAULT 0 CHECK (is_deleted IN (0, 1)),
    created_at_seconds INTEGER NOT NULL,
    updated_at_seconds INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'discovered',
        'fingerprinted',
        'parse_queued',
        'parse_running',
        'text_extracted',
        'ocr_required',
        'ocr_running',
        'ocr_done',
        'text_cleaned',
        'fields_extracted',
        'embedding_done',
        'indexed_partial',
        'searchable',
        'failed_retryable',
        'failed_permanent',
        'deleted'
    ))
);

CREATE TABLE resume_version (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    candidate_id TEXT,
    parse_version TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    language_set_json TEXT NOT NULL DEFAULT '[]',
    page_count INTEGER CHECK (page_count IS NULL OR page_count >= 0),
    raw_text TEXT,
    clean_text TEXT,
    quality_score REAL CHECK (quality_score IS NULL OR quality_score BETWEEN 0 AND 1),
    visibility TEXT NOT NULL CHECK (visibility IN ('searchable', 'partial', 'hidden')),
    FOREIGN KEY (document_id) REFERENCES document(id) ON DELETE CASCADE
);

CREATE TABLE ingest_job (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    resume_version_id TEXT,
    kind TEXT NOT NULL CHECK (kind IN (
        'discover_document',
        'fingerprint_document',
        'parse_document',
        'clean_text',
        'extract_fields',
        'update_index'
    )),
    status TEXT NOT NULL CHECK (status IN (
        'queued',
        'running',
        'interrupted',
        'completed',
        'failed_retryable',
        'failed_permanent'
    )),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts > 0),
    queued_at_seconds INTEGER NOT NULL,
    started_at_seconds INTEGER,
    finished_at_seconds INTEGER,
    updated_at_seconds INTEGER NOT NULL,
    FOREIGN KEY (document_id) REFERENCES document(id) ON DELETE CASCADE,
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE SET NULL
);

CREATE TABLE index_state (
    state_key TEXT PRIMARY KEY,
    manifest_version TEXT NOT NULL,
    snapshot_token TEXT,
    status TEXT NOT NULL CHECK (status IN ('empty', 'building', 'ready', 'stale')),
    updated_at_seconds INTEGER NOT NULL,
    CHECK (state_key = 'default')
);

CREATE INDEX ingest_job_recovery_idx
    ON ingest_job(status, attempt_count, max_attempts);
CREATE INDEX resume_version_document_idx
    ON resume_version(document_id);
"#;

fn migration_applied(connection: &Connection, version: u32) -> Result<bool> {
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE version = ?1)",
            params![i64::from(version)],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::migration)?;

    Ok(exists == 1)
}

fn read_document(row: &Row<'_>) -> Result<Document> {
    let id = read_id::<DocumentId>(row, 0, "document.id")?;
    let byte_size = i64_to_u64(read_i64(row, 5)?, "document.byte_size")?;

    Ok(Document {
        id,
        source_uri: read_string(row, 1)?,
        normalized_path: read_string(row, 2)?,
        file_name: read_string(row, 3)?,
        extension: file_extension_from_storage(&read_string(row, 4)?),
        byte_size,
        mtime: UnixTimestamp::from_unix_seconds(read_i64(row, 6)?),
        content_hash: read_optional_string(row, 7)?,
        text_hash: read_optional_string(row, 8)?,
        is_deleted: read_i64(row, 9)? == 1,
        created_at: UnixTimestamp::from_unix_seconds(read_i64(row, 10)?),
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 11)?),
        status: document_status_from_storage(&read_string(row, 12)?)?,
    })
}

fn read_resume_version(row: &Row<'_>) -> Result<ResumeVersion> {
    let language_set_json = read_string(row, 5)?;
    let language_set = serde_json::from_str::<Vec<String>>(&language_set_json)
        .map_err(|_| MetaStoreError::invalid_value("resume_version.language_set"))?;
    let page_count = read_optional_i64(row, 6)?
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| MetaStoreError::invalid_value("resume_version.page_count"))
        })
        .transpose()?;
    let quality_score = read_optional_f64(row, 9)?.map(|value| value as f32);

    Ok(ResumeVersion {
        id: read_id::<ResumeVersionId>(row, 0, "resume_version.id")?,
        document_id: read_id::<DocumentId>(row, 1, "resume_version.document_id")?,
        candidate_id: read_optional_id::<CandidateId>(row, 2, "resume_version.candidate_id")?,
        parse_version: read_string(row, 3)?,
        schema_version: read_string(row, 4)?,
        language_set,
        page_count,
        raw_text: read_optional_string(row, 7)?,
        clean_text: read_optional_string(row, 8)?,
        quality_score,
        visibility: resume_visibility_from_storage(&read_string(row, 10)?)?,
    })
}

fn read_ingest_job(row: &Row<'_>) -> Result<IngestJob> {
    let attempt_count = i64_to_u32(read_i64(row, 5)?, "ingest_job.attempt_count")?;
    let max_attempts = i64_to_u32(read_i64(row, 6)?, "ingest_job.max_attempts")?;

    Ok(IngestJob {
        id: read_id::<IngestJobId>(row, 0, "ingest_job.id")?,
        document_id: read_id::<DocumentId>(row, 1, "ingest_job.document_id")?,
        resume_version_id: read_optional_id::<ResumeVersionId>(
            row,
            2,
            "ingest_job.resume_version_id",
        )?,
        kind: ingest_job_kind_from_storage(&read_string(row, 3)?)?,
        status: ingest_job_status_from_storage(&read_string(row, 4)?)?,
        attempt_count,
        max_attempts,
        queued_at: UnixTimestamp::from_unix_seconds(read_i64(row, 7)?),
        started_at: read_optional_timestamp(row, 8)?,
        finished_at: read_optional_timestamp(row, 9)?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 10)?),
    })
}

fn read_index_state(row: &Row<'_>) -> Result<IndexState> {
    Ok(IndexState {
        manifest_version: read_string(row, 0)?,
        snapshot_token: read_optional_string(row, 1)?,
        status: index_state_status_from_storage(&read_string(row, 2)?)?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 3)?),
    })
}

fn read_string(row: &Row<'_>, index: usize) -> Result<String> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_optional_string(row: &Row<'_>, index: usize) -> Result<Option<String>> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_i64(row: &Row<'_>, index: usize) -> Result<i64> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_optional_i64(row: &Row<'_>, index: usize) -> Result<Option<i64>> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_optional_f64(row: &Row<'_>, index: usize) -> Result<Option<f64>> {
    row.get(index).map_err(MetaStoreError::storage)
}

fn read_optional_timestamp(row: &Row<'_>, index: usize) -> Result<Option<UnixTimestamp>> {
    Ok(read_optional_i64(row, index)?.map(UnixTimestamp::from_unix_seconds))
}

fn read_id<T>(row: &Row<'_>, index: usize, field: &'static str) -> Result<T>
where
    T: FromStr,
{
    let value = read_string(row, index)?;
    T::from_str(&value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn read_optional_id<T>(row: &Row<'_>, index: usize, field: &'static str) -> Result<Option<T>>
where
    T: FromStr,
{
    read_optional_string(row, index)?
        .map(|value| T::from_str(&value).map_err(|_| MetaStoreError::invalid_value(field)))
        .transpose()
}

fn u64_to_i64(value: u64, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn u32_to_i64(value: u32) -> i64 {
    i64::from(value)
}

fn i64_to_u64(value: i64, field: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn i64_to_u32(value: i64, field: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
}

fn bool_to_i64(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn file_extension_to_storage(extension: &FileExtension) -> String {
    match extension {
        FileExtension::Docx => "docx".to_string(),
        FileExtension::Pdf => "pdf".to_string(),
        FileExtension::Doc => "doc".to_string(),
        FileExtension::Txt => "txt".to_string(),
        FileExtension::Image => "image".to_string(),
        FileExtension::Other(value) => format!("other:{value}"),
    }
}

fn file_extension_from_storage(value: &str) -> FileExtension {
    match value {
        "docx" => FileExtension::Docx,
        "pdf" => FileExtension::Pdf,
        "doc" => FileExtension::Doc,
        "txt" => FileExtension::Txt,
        "image" => FileExtension::Image,
        _ => FileExtension::Other(value.strip_prefix("other:").unwrap_or(value).to_string()),
    }
}

fn document_status_to_storage(status: DocumentStatus) -> &'static str {
    match status {
        DocumentStatus::Discovered => "discovered",
        DocumentStatus::Fingerprinted => "fingerprinted",
        DocumentStatus::ParseQueued => "parse_queued",
        DocumentStatus::ParseRunning => "parse_running",
        DocumentStatus::TextExtracted => "text_extracted",
        DocumentStatus::OcrRequired => "ocr_required",
        DocumentStatus::OcrRunning => "ocr_running",
        DocumentStatus::OcrDone => "ocr_done",
        DocumentStatus::TextCleaned => "text_cleaned",
        DocumentStatus::FieldsExtracted => "fields_extracted",
        DocumentStatus::EmbeddingDone => "embedding_done",
        DocumentStatus::IndexedPartial => "indexed_partial",
        DocumentStatus::Searchable => "searchable",
        DocumentStatus::FailedRetryable => "failed_retryable",
        DocumentStatus::FailedPermanent => "failed_permanent",
        DocumentStatus::Deleted => "deleted",
    }
}

fn document_status_from_storage(value: &str) -> Result<DocumentStatus> {
    match value {
        "discovered" => Ok(DocumentStatus::Discovered),
        "fingerprinted" => Ok(DocumentStatus::Fingerprinted),
        "parse_queued" => Ok(DocumentStatus::ParseQueued),
        "parse_running" => Ok(DocumentStatus::ParseRunning),
        "text_extracted" => Ok(DocumentStatus::TextExtracted),
        "ocr_required" => Ok(DocumentStatus::OcrRequired),
        "ocr_running" => Ok(DocumentStatus::OcrRunning),
        "ocr_done" => Ok(DocumentStatus::OcrDone),
        "text_cleaned" => Ok(DocumentStatus::TextCleaned),
        "fields_extracted" => Ok(DocumentStatus::FieldsExtracted),
        "embedding_done" => Ok(DocumentStatus::EmbeddingDone),
        "indexed_partial" => Ok(DocumentStatus::IndexedPartial),
        "searchable" => Ok(DocumentStatus::Searchable),
        "failed_retryable" => Ok(DocumentStatus::FailedRetryable),
        "failed_permanent" => Ok(DocumentStatus::FailedPermanent),
        "deleted" => Ok(DocumentStatus::Deleted),
        _ => Err(MetaStoreError::invalid_value("document.status")),
    }
}

fn resume_visibility_to_storage(visibility: ResumeVisibility) -> &'static str {
    match visibility {
        ResumeVisibility::Searchable => "searchable",
        ResumeVisibility::Partial => "partial",
        ResumeVisibility::Hidden => "hidden",
    }
}

fn resume_visibility_from_storage(value: &str) -> Result<ResumeVisibility> {
    match value {
        "searchable" => Ok(ResumeVisibility::Searchable),
        "partial" => Ok(ResumeVisibility::Partial),
        "hidden" => Ok(ResumeVisibility::Hidden),
        _ => Err(MetaStoreError::invalid_value("resume_version.visibility")),
    }
}

fn ingest_job_kind_to_storage(kind: IngestJobKind) -> &'static str {
    match kind {
        IngestJobKind::DiscoverDocument => "discover_document",
        IngestJobKind::FingerprintDocument => "fingerprint_document",
        IngestJobKind::ParseDocument => "parse_document",
        IngestJobKind::CleanText => "clean_text",
        IngestJobKind::ExtractFields => "extract_fields",
        IngestJobKind::UpdateIndex => "update_index",
    }
}

fn ingest_job_kind_from_storage(value: &str) -> Result<IngestJobKind> {
    match value {
        "discover_document" => Ok(IngestJobKind::DiscoverDocument),
        "fingerprint_document" => Ok(IngestJobKind::FingerprintDocument),
        "parse_document" => Ok(IngestJobKind::ParseDocument),
        "clean_text" => Ok(IngestJobKind::CleanText),
        "extract_fields" => Ok(IngestJobKind::ExtractFields),
        "update_index" => Ok(IngestJobKind::UpdateIndex),
        _ => Err(MetaStoreError::invalid_value("ingest_job.kind")),
    }
}

fn ingest_job_status_to_storage(status: IngestJobStatus) -> &'static str {
    match status {
        IngestJobStatus::Queued => "queued",
        IngestJobStatus::Running => "running",
        IngestJobStatus::Interrupted => "interrupted",
        IngestJobStatus::Completed => "completed",
        IngestJobStatus::FailedRetryable => "failed_retryable",
        IngestJobStatus::FailedPermanent => "failed_permanent",
    }
}

fn ingest_job_status_from_storage(value: &str) -> Result<IngestJobStatus> {
    match value {
        "queued" => Ok(IngestJobStatus::Queued),
        "running" => Ok(IngestJobStatus::Running),
        "interrupted" => Ok(IngestJobStatus::Interrupted),
        "completed" => Ok(IngestJobStatus::Completed),
        "failed_retryable" => Ok(IngestJobStatus::FailedRetryable),
        "failed_permanent" => Ok(IngestJobStatus::FailedPermanent),
        _ => Err(MetaStoreError::invalid_value("ingest_job.status")),
    }
}

fn job_status_transition_allowed(current: IngestJobStatus, next: IngestJobStatus) -> bool {
    match current {
        IngestJobStatus::Queued => matches!(
            next,
            IngestJobStatus::Queued | IngestJobStatus::Running | IngestJobStatus::Interrupted
        ),
        IngestJobStatus::Running => matches!(
            next,
            IngestJobStatus::Running
                | IngestJobStatus::Interrupted
                | IngestJobStatus::Completed
                | IngestJobStatus::FailedRetryable
                | IngestJobStatus::FailedPermanent
        ),
        IngestJobStatus::Interrupted => matches!(
            next,
            IngestJobStatus::Interrupted
                | IngestJobStatus::Running
                | IngestJobStatus::FailedPermanent
        ),
        IngestJobStatus::FailedRetryable => matches!(
            next,
            IngestJobStatus::FailedRetryable
                | IngestJobStatus::Running
                | IngestJobStatus::FailedPermanent
        ),
        IngestJobStatus::Completed => matches!(next, IngestJobStatus::Completed),
        IngestJobStatus::FailedPermanent => matches!(next, IngestJobStatus::FailedPermanent),
    }
}

fn index_state_status_to_storage(status: IndexStateStatus) -> &'static str {
    match status {
        IndexStateStatus::Empty => "empty",
        IndexStateStatus::Building => "building",
        IndexStateStatus::Ready => "ready",
        IndexStateStatus::Stale => "stale",
    }
}

fn index_state_status_from_storage(value: &str) -> Result<IndexStateStatus> {
    match value {
        "empty" => Ok(IndexStateStatus::Empty),
        "building" => Ok(IndexStateStatus::Building),
        "ready" => Ok(IndexStateStatus::Ready),
        "stale" => Ok(IndexStateStatus::Stale),
        _ => Err(MetaStoreError::invalid_value("index_state.status")),
    }
}
