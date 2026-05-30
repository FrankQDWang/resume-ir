use rusqlite::{Connection, params};
use std::sync::atomic::{AtomicU64, Ordering};

pub type StoreResult<T> = rusqlite::Result<T>;

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobId(String);

impl JobId {
    #[allow(clippy::new_without_default)]
    #[must_use]
    pub fn new() -> Self {
        let value = NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed);
        Self(format!("job_{value:016x}"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentRecord {
    pub doc_id: String,
    pub source_uri: String,
    pub normalized_path: String,
    pub file_name: String,
    pub extension: String,
    pub byte_size: i64,
    pub mtime_unix_ms: i64,
    pub is_deleted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumeVersionRecord {
    pub version_id: String,
    pub doc_id: String,
    pub parse_version: String,
    pub schema_version: String,
    pub visibility: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestJobStatus {
    Queued,
    Running,
    Succeeded,
    FailedRetryable,
    FailedPermanent,
}

impl IngestJobStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::FailedRetryable => "failed_retryable",
            Self::FailedPermanent => "failed_permanent",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetryableJob {
    job_id: String,
    pub doc_id: String,
    pub status: String,
    pub attempts: i64,
    pub max_attempts: i64,
}

impl RetryableJob {
    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
}

pub struct MetaStore {
    connection: Connection,
}

impl MetaStore {
    pub fn open_in_memory() -> StoreResult<Self> {
        let connection = Connection::open_in_memory()?;
        Ok(Self { connection })
    }

    pub fn apply_migrations(&self) -> StoreResult<()> {
        self.connection.execute_batch(
            r"
            CREATE TABLE IF NOT EXISTS document (
                doc_id TEXT PRIMARY KEY,
                source_uri TEXT NOT NULL,
                normalized_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                extension TEXT NOT NULL,
                byte_size INTEGER NOT NULL,
                mtime_unix_ms INTEGER NOT NULL,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                created_at_unix_ms INTEGER NOT NULL DEFAULT 0,
                updated_at_unix_ms INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS resume_version (
                version_id TEXT PRIMARY KEY,
                doc_id TEXT NOT NULL,
                parse_version TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                visibility TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (doc_id) REFERENCES document(doc_id)
            );

            CREATE TABLE IF NOT EXISTS ingest_job (
                job_id TEXT PRIMARY KEY,
                doc_id TEXT NOT NULL,
                status TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL,
                last_error TEXT,
                created_at_unix_ms INTEGER NOT NULL DEFAULT 0,
                updated_at_unix_ms INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (doc_id) REFERENCES document(doc_id)
            );

            CREATE TABLE IF NOT EXISTS index_state (
                doc_id TEXT NOT NULL,
                version_id TEXT,
                status TEXT NOT NULL,
                schema_version TEXT NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (doc_id, schema_version),
                FOREIGN KEY (doc_id) REFERENCES document(doc_id),
                FOREIGN KEY (version_id) REFERENCES resume_version(version_id)
            );

            PRAGMA user_version = 1;
            ",
        )
    }

    pub fn schema_version(&self) -> StoreResult<i64> {
        self.connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
    }

    pub fn table_exists(&self, table_name: &str) -> StoreResult<bool> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table_name],
            |row| row.get(0),
        )?;
        Ok(count == 1)
    }

    pub fn upsert_document(&self, document: &DocumentRecord) -> StoreResult<()> {
        self.connection.execute(
            r"
            INSERT INTO document (
                doc_id, source_uri, normalized_path, file_name, extension,
                byte_size, mtime_unix_ms, is_deleted
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(doc_id) DO UPDATE SET
                source_uri = excluded.source_uri,
                normalized_path = excluded.normalized_path,
                file_name = excluded.file_name,
                extension = excluded.extension,
                byte_size = excluded.byte_size,
                mtime_unix_ms = excluded.mtime_unix_ms,
                is_deleted = excluded.is_deleted
            ",
            params![
                document.doc_id,
                document.source_uri,
                document.normalized_path,
                document.file_name,
                document.extension,
                document.byte_size,
                document.mtime_unix_ms,
                document.is_deleted,
            ],
        )?;
        Ok(())
    }

    pub fn list_visible_documents(&self) -> StoreResult<Vec<DocumentRecord>> {
        let mut statement = self.connection.prepare(
            r"
            SELECT doc_id, source_uri, normalized_path, file_name, extension,
                   byte_size, mtime_unix_ms, is_deleted
            FROM document
            WHERE is_deleted = 0
            ORDER BY doc_id
            ",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(DocumentRecord {
                doc_id: row.get(0)?,
                source_uri: row.get(1)?,
                normalized_path: row.get(2)?,
                file_name: row.get(3)?,
                extension: row.get(4)?,
                byte_size: row.get(5)?,
                mtime_unix_ms: row.get(6)?,
                is_deleted: row.get(7)?,
            })
        })?;
        rows.collect()
    }

    pub fn upsert_resume_version(&self, version: &ResumeVersionRecord) -> StoreResult<()> {
        self.connection.execute(
            r"
            INSERT INTO resume_version (
                version_id, doc_id, parse_version, schema_version, visibility
            )
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(version_id) DO UPDATE SET
                doc_id = excluded.doc_id,
                parse_version = excluded.parse_version,
                schema_version = excluded.schema_version,
                visibility = excluded.visibility
            ",
            params![
                version.version_id,
                version.doc_id,
                version.parse_version,
                version.schema_version,
                version.visibility,
            ],
        )?;
        Ok(())
    }

    pub fn count_resume_versions(&self, doc_id: &str) -> StoreResult<i64> {
        self.connection.query_row(
            "SELECT COUNT(*) FROM resume_version WHERE doc_id = ?1",
            [doc_id],
            |row| row.get(0),
        )
    }

    pub fn create_ingest_job(&self, doc_id: &str, max_attempts: i64) -> StoreResult<JobId> {
        let job_id = JobId::new();
        self.connection.execute(
            r"
            INSERT INTO ingest_job (job_id, doc_id, status, max_attempts)
            VALUES (?1, ?2, ?3, ?4)
            ",
            params![
                job_id.as_str(),
                doc_id,
                IngestJobStatus::Queued.as_str(),
                max_attempts,
            ],
        )?;
        Ok(job_id)
    }

    pub fn update_job_status(&self, job_id: &JobId, status: IngestJobStatus) -> StoreResult<()> {
        self.connection.execute(
            "UPDATE ingest_job SET status = ?1 WHERE job_id = ?2",
            params![status.as_str(), job_id.as_str()],
        )?;
        Ok(())
    }

    pub fn list_retryable_jobs(&self, limit: usize) -> StoreResult<Vec<RetryableJob>> {
        let mut statement = self.connection.prepare(
            r"
            SELECT job_id, doc_id, status, attempts, max_attempts
            FROM ingest_job
            WHERE status IN ('queued', 'running', 'failed_retryable')
              AND attempts < max_attempts
            ORDER BY rowid
            LIMIT ?1
            ",
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            Ok(RetryableJob {
                job_id: row.get(0)?,
                doc_id: row.get(1)?,
                status: row.get(2)?,
                attempts: row.get(3)?,
                max_attempts: row.get(4)?,
            })
        })?;
        rows.collect()
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "meta-store"
}
