use std::cell::RefCell;
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

pub use core_domain::{
    CandidateId, Document, DocumentId, DocumentStatus, EntityMention, EntityMentionId, EntityType,
    FileExtension, ImportTaskId, IndexStateStatus, IngestJobId, IngestJobKind, IngestJobStatus,
    ResumeVersion, ResumeVersionId, ResumeVisibility, SectionId, UnixTimestamp,
};
use rusqlite::{params, Connection, OptionalExtension, Row};

const SCHEMA_VERSION_V1: u32 = 1;
const SCHEMA_VERSION_V2: u32 = 2;
const SCHEMA_VERSION_V3: u32 = 3;
const SCHEMA_VERSION_V4: u32 = 4;
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
const IMPORT_TASK_COLUMNS: &str = "\
    id, root_path, status, queued_at_seconds, started_at_seconds, finished_at_seconds, \
    updated_at_seconds";
const ENTITY_MENTION_COLUMNS: &str = "\
    id, resume_version_id, section_id, entity_type, raw_value, normalized_value, \
    span_start, span_end, confidence, extractor";

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

        let mut applied_versions = Vec::new();

        for (version, schema) in [
            (SCHEMA_VERSION_V1, SCHEMA_V1),
            (SCHEMA_VERSION_V2, SCHEMA_V2),
            (SCHEMA_VERSION_V3, SCHEMA_V3),
            (SCHEMA_VERSION_V4, SCHEMA_V4),
        ] {
            if !migration_applied(&connection, version)? {
                let transaction = connection
                    .transaction()
                    .map_err(MetaStoreError::migration)?;
                transaction
                    .execute_batch(schema)
                    .map_err(MetaStoreError::migration)?;
                transaction
                    .execute(
                        "INSERT INTO schema_migrations (version, applied_at_seconds) VALUES (?1, ?2)",
                        params![i64::from(version), 0_i64],
                    )
                    .map_err(MetaStoreError::migration)?;
                transaction.commit().map_err(MetaStoreError::migration)?;
                applied_versions.push(version);
            }
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

    pub fn mark_document_deleted(
        &self,
        id: &DocumentId,
        updated_at: UnixTimestamp,
    ) -> Result<Option<Document>> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let current_document = {
            let sql = format!("SELECT {DOCUMENT_COLUMNS} FROM document WHERE id = ?1");
            let mut statement = transaction.prepare(&sql).map_err(MetaStoreError::storage)?;
            let mut rows = statement
                .query(params![id.as_str()])
                .map_err(MetaStoreError::storage)?;

            match rows.next().map_err(MetaStoreError::storage)? {
                Some(row) => Some(read_document(row)?),
                None => None,
            }
        };

        let Some(mut document) = current_document else {
            transaction.commit().map_err(MetaStoreError::storage)?;
            return Ok(None);
        };

        transaction
            .execute(
                "\
                UPDATE document
                SET is_deleted = 1, status = ?1, updated_at_seconds = ?2
                WHERE id = ?3",
                params![
                    document_status_to_storage(DocumentStatus::Deleted),
                    updated_at.as_unix_seconds(),
                    id.as_str(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "\
                UPDATE resume_version
                SET visibility = ?1
                WHERE document_id = ?2",
                params![
                    resume_visibility_to_storage(ResumeVisibility::Hidden),
                    id.as_str()
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction.commit().map_err(MetaStoreError::storage)?;

        document.is_deleted = true;
        document.status = DocumentStatus::Deleted;
        document.updated_at = updated_at;
        Ok(Some(document))
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

    pub fn resume_versions_for_document(
        &self,
        document_id: &DocumentId,
    ) -> Result<Vec<ResumeVersion>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {RESUME_VERSION_COLUMNS}
            FROM resume_version
            WHERE document_id = ?1
            ORDER BY id"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![document_id.as_str()])
            .map_err(MetaStoreError::storage)?;
        let mut versions = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            versions.push(read_resume_version(row)?);
        }

        Ok(versions)
    }

    pub fn replace_entity_mentions(
        &self,
        version_id: &ResumeVersionId,
        mentions: &[EntityMention],
    ) -> Result<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM entity_mention WHERE resume_version_id = ?1",
                params![version_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;

        for mention in mentions {
            validate_entity_mention(version_id, mention)?;
            transaction
                .execute(
                    "\
                    INSERT INTO entity_mention (
                        id, resume_version_id, section_id, entity_type, raw_value,
                        normalized_value, span_start, span_end, confidence, extractor
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        mention.id.as_str(),
                        mention.resume_version_id.as_str(),
                        mention.section_id.as_ref().map(SectionId::as_str),
                        entity_type_to_storage(&mention.entity_type),
                        mention.raw_value.as_str(),
                        mention.normalized_value.as_deref(),
                        mention
                            .span_start
                            .map(|value| usize_to_i64(value, "entity_mention.span_start"))
                            .transpose()?,
                        mention
                            .span_end
                            .map(|value| usize_to_i64(value, "entity_mention.span_end"))
                            .transpose()?,
                        f64::from(mention.confidence),
                        mention.extractor.as_str(),
                    ],
                )
                .map_err(MetaStoreError::storage)?;
        }

        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn entity_mentions_for_version(
        &self,
        version_id: &ResumeVersionId,
    ) -> Result<Vec<EntityMention>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {ENTITY_MENTION_COLUMNS}
            FROM entity_mention
            WHERE resume_version_id = ?1
            ORDER BY span_start IS NULL, span_start, rowid"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![version_id.as_str()])
            .map_err(MetaStoreError::storage)?;
        let mut mentions = Vec::new();

        while let Some(row) = rows.next().map_err(MetaStoreError::storage)? {
            mentions.push(read_entity_mention(row)?);
        }

        Ok(mentions)
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

    pub fn enqueue_ocr_job_for_document(
        &self,
        document_id: &DocumentId,
        queued_at: UnixTimestamp,
    ) -> Result<EnqueuedIngestJob> {
        let id = IngestJobId::from_non_secret_parts(&["ocr-document", document_id.as_str()]);
        let job = IngestJob {
            id,
            document_id: document_id.clone(),
            resume_version_id: None,
            kind: IngestJobKind::OcrDocument,
            status: IngestJobStatus::Queued,
            attempt_count: 0,
            max_attempts: 3,
            queued_at,
            started_at: None,
            finished_at: None,
            updated_at: queued_at,
        };
        let inserted = {
            let mut connection = self.connection.borrow_mut();
            let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
            let existing_id = {
                let mut statement = transaction
                    .prepare(
                        "\
                        SELECT id
                        FROM ingest_job
                        WHERE document_id = ?1 AND kind = ?2
                        ORDER BY rowid
                        LIMIT 1",
                    )
                    .map_err(MetaStoreError::storage)?;
                let mut rows = statement
                    .query(params![
                        document_id.as_str(),
                        ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                    ])
                    .map_err(MetaStoreError::storage)?;

                match rows.next().map_err(MetaStoreError::storage)? {
                    Some(row) => Some(read_string(row, 0)?),
                    None => None,
                }
            };

            if existing_id.is_some() {
                transaction.commit().map_err(MetaStoreError::storage)?;
                false
            } else {
                transaction
                    .execute(
                        "\
                        INSERT INTO ingest_job (
                            id, document_id, resume_version_id, kind, status, attempt_count,
                            max_attempts, queued_at_seconds, started_at_seconds,
                            finished_at_seconds, updated_at_seconds
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
                transaction.commit().map_err(MetaStoreError::storage)?;
                true
            }
        };

        let job = self
            .ocr_job_for_document(document_id)?
            .ok_or_else(|| MetaStoreError::not_found("ingest_job"))?;
        Ok(EnqueuedIngestJob { job, inserted })
    }

    fn ocr_job_for_document(&self, document_id: &DocumentId) -> Result<Option<IngestJob>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {INGEST_JOB_COLUMNS}
            FROM ingest_job
            WHERE document_id = ?1 AND kind = ?2
            ORDER BY rowid
            LIMIT 1"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                document_id.as_str(),
                ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
            ])
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
        self.claim_next_job_matching(None, now)
    }

    pub fn claim_next_job_by_kind(
        &self,
        kind: IngestJobKind,
        now: UnixTimestamp,
    ) -> Result<Option<IngestJob>> {
        self.claim_next_job_matching(Some(kind), now)
    }

    fn claim_next_job_matching(
        &self,
        kind: Option<IngestJobKind>,
        now: UnixTimestamp,
    ) -> Result<Option<IngestJob>> {
        let kind_filter = kind.map(ingest_job_kind_to_storage);
        let claimed_id = {
            let mut connection = self.connection.borrow_mut();
            let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
            let candidate_id = {
                let mut statement = transaction
                    .prepare(
                        "\
                        SELECT id
                        FROM ingest_job
                        WHERE (
                                status IN (?1, ?2)
                                OR (status = ?3 AND attempt_count < max_attempts)
                            )
                            AND (?4 IS NULL OR kind = ?4)
                        ORDER BY queued_at_seconds, rowid
                        LIMIT 1",
                    )
                    .map_err(MetaStoreError::storage)?;
                let mut rows = statement
                    .query(params![
                        ingest_job_status_to_storage(IngestJobStatus::Queued),
                        ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                        ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                        kind_filter,
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
                        )
                        AND (?7 IS NULL OR kind = ?7)",
                    params![
                        ingest_job_status_to_storage(IngestJobStatus::Running),
                        now_seconds,
                        candidate_id,
                        ingest_job_status_to_storage(IngestJobStatus::Queued),
                        ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                        ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                        kind_filter,
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

    pub fn insert_import_task(&self, task: &ImportTask) -> Result<()> {
        validate_import_task(task)?;

        let connection = self.connection.borrow();
        connection
            .execute(
                "\
                INSERT INTO import_task (
                    id, root_path, status, queued_at_seconds, started_at_seconds,
                    finished_at_seconds, updated_at_seconds
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    task.id.as_str(),
                    task.root_path,
                    import_task_status_to_storage(task.status),
                    task.queued_at.as_unix_seconds(),
                    task.started_at.map(UnixTimestamp::as_unix_seconds),
                    task.finished_at.map(UnixTimestamp::as_unix_seconds),
                    task.updated_at.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        Ok(())
    }

    pub fn import_task_by_id(&self, id: &ImportTaskId) -> Result<Option<ImportTask>> {
        let connection = self.connection.borrow();
        let sql = format!("SELECT {IMPORT_TASK_COLUMNS} FROM import_task WHERE id = ?1");
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![id.as_str()])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_import_task(row)?)),
            None => Ok(None),
        }
    }

    pub fn pending_import_task_by_root(&self, root_path: &str) -> Result<Option<ImportTask>> {
        let connection = self.connection.borrow();
        let sql = format!(
            "\
            SELECT {IMPORT_TASK_COLUMNS}
            FROM import_task
            WHERE root_path = ?1 AND status IN (?2, ?3, ?4)
            ORDER BY CASE WHEN status = ?3 THEN 0 ELSE 1 END, queued_at_seconds, rowid
            LIMIT 1"
        );
        let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                root_path,
                import_task_status_to_storage(ImportTaskStatus::Queued),
                import_task_status_to_storage(ImportTaskStatus::Running),
                import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
            ])
            .map_err(MetaStoreError::storage)?;

        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Ok(Some(read_import_task(row)?)),
            None => Ok(None),
        }
    }

    pub fn update_import_task_status(
        &self,
        id: &ImportTaskId,
        status: ImportTaskStatus,
        updated_at: UnixTimestamp,
    ) -> Result<()> {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection.transaction().map_err(MetaStoreError::storage)?;
        let current_task = {
            let sql = format!("SELECT {IMPORT_TASK_COLUMNS} FROM import_task WHERE id = ?1");
            let mut statement = transaction.prepare(&sql).map_err(MetaStoreError::storage)?;
            let mut rows = statement
                .query(params![id.as_str()])
                .map_err(MetaStoreError::storage)?;

            match rows.next().map_err(MetaStoreError::storage)? {
                Some(row) => read_import_task(row)?,
                None => return Err(MetaStoreError::not_found("import_task")),
            }
        };
        let current_status = current_task.status;

        if updated_at.as_unix_seconds() < current_task.updated_at.as_unix_seconds() {
            return Err(MetaStoreError::invalid_value("import_task.timestamps"));
        }

        if !import_task_status_transition_allowed(current_status, status) {
            return Err(MetaStoreError::invalid_transition());
        }
        let next_task = next_import_task_state(&current_task, status, updated_at);
        validate_import_task(&next_task)?;

        let updated_at_seconds = updated_at.as_unix_seconds();
        let changed = transaction
            .execute(
                "\
                UPDATE import_task
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
                    import_task_status_to_storage(status),
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    import_task_status_to_storage(ImportTaskStatus::Completed),
                    import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
                    updated_at_seconds,
                    import_task_status_to_storage(ImportTaskStatus::FailedPermanent),
                    id.as_str(),
                    import_task_status_to_storage(current_status),
                ],
            )
            .map_err(MetaStoreError::storage)?;

        if changed == 0 {
            return Err(MetaStoreError::invalid_transition());
        }

        transaction.commit().map_err(MetaStoreError::storage)?;
        Ok(())
    }

    pub fn status_summary(&self) -> Result<StoreStatusSummary> {
        let connection = self.connection.borrow();
        let document_counts = connection
            .query_row(
                "\
                SELECT
                    COALESCE(SUM(CASE WHEN status IN ('indexed_partial', 'searchable') THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'searchable' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'indexed_partial' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'failed_retryable' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'failed_permanent' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'ocr_required' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'fields_extracted' THEN 1 ELSE 0 END), 0)
                FROM document
                WHERE is_deleted = 0 AND status <> 'deleted'",
                [],
                |row| {
                    Ok(DocumentStatusCounts {
                        indexed_documents: row.get(0)?,
                        searchable_documents: row.get(1)?,
                        partial_documents: row.get(2)?,
                        failed_retryable: row.get(3)?,
                        failed_permanent: row.get(4)?,
                        ocr_queue_depth: row.get(5)?,
                        embedding_queue_depth: row.get(6)?,
                    })
                },
            )
            .map_err(MetaStoreError::storage)?;
        let recovery_queue_depth = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM ingest_job
                WHERE status IN (?1, ?2)
                    OR (status = ?3 AND attempt_count < max_attempts)",
                params![
                    ingest_job_status_to_storage(IngestJobStatus::Running),
                    ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let ocr_jobs_queued = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM ingest_job
                WHERE kind = ?1
                    AND (
                        status IN (?2, ?3)
                        OR (status = ?4 AND attempt_count < max_attempts)
                    )",
                params![
                    ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                    ingest_job_status_to_storage(IngestJobStatus::Queued),
                    ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let import_tasks_queued = connection
            .query_row(
                "SELECT COUNT(*) FROM import_task WHERE status = ?1",
                params![import_task_status_to_storage(ImportTaskStatus::Queued)],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let import_tasks_recoverable = connection
            .query_row(
                "SELECT COUNT(*) FROM import_task WHERE status IN (?1, ?2)",
                params![
                    import_task_status_to_storage(ImportTaskStatus::Running),
                    import_task_status_to_storage(ImportTaskStatus::FailedRetryable),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let entity_mentions = connection
            .query_row(
                "\
                SELECT COUNT(*)
                FROM entity_mention AS mention
                JOIN resume_version AS version ON version.id = mention.resume_version_id
                JOIN document AS document ON document.id = version.document_id
                WHERE document.is_deleted = 0
                    AND document.status <> ?1
                    AND version.visibility <> ?2",
                params![
                    document_status_to_storage(DocumentStatus::Deleted),
                    resume_visibility_to_storage(ResumeVisibility::Hidden),
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?;
        let index_state = connection
            .query_row(
                "\
                SELECT status, snapshot_token
                FROM index_state
                WHERE state_key = ?1",
                params![INDEX_STATE_KEY],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(MetaStoreError::storage)?;
        let (index_health, last_snapshot_id) = match index_state {
            Some((status, snapshot_token)) => {
                (index_state_status_from_storage(&status)?, snapshot_token)
            }
            None => (IndexStateStatus::Empty, None),
        };

        Ok(StoreStatusSummary {
            indexed_documents: i64_to_u64(
                document_counts.indexed_documents,
                "status.indexed_documents",
            )?,
            searchable_documents: i64_to_u64(
                document_counts.searchable_documents,
                "status.searchable_documents",
            )?,
            partial_documents: i64_to_u64(
                document_counts.partial_documents,
                "status.partial_documents",
            )?,
            failed_retryable: i64_to_u64(
                document_counts.failed_retryable,
                "status.failed_retryable",
            )?,
            failed_permanent: i64_to_u64(
                document_counts.failed_permanent,
                "status.failed_permanent",
            )?,
            ocr_queue_depth: i64_to_u64(document_counts.ocr_queue_depth, "status.ocr_queue_depth")?,
            embedding_queue_depth: i64_to_u64(
                document_counts.embedding_queue_depth,
                "status.embedding_queue_depth",
            )?,
            recovery_queue_depth: i64_to_u64(recovery_queue_depth, "status.recovery_queue_depth")?,
            import_tasks_queued: i64_to_u64(import_tasks_queued, "status.import_tasks_queued")?,
            import_tasks_recoverable: i64_to_u64(
                import_tasks_recoverable,
                "status.import_tasks_recoverable",
            )?,
            ocr_jobs_queued: i64_to_u64(ocr_jobs_queued, "status.ocr_jobs_queued")?,
            entity_mentions: i64_to_u64(entity_mentions, "status.entity_mentions")?,
            index_health,
            last_snapshot_id,
        })
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnqueuedIngestJob {
    pub job: IngestJob,
    pub inserted: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportTask {
    pub id: ImportTaskId,
    pub root_path: String,
    pub status: ImportTaskStatus,
    pub queued_at: UnixTimestamp,
    pub started_at: Option<UnixTimestamp>,
    pub finished_at: Option<UnixTimestamp>,
    pub updated_at: UnixTimestamp,
}

impl fmt::Debug for ImportTask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportTask")
            .field("id", &self.id)
            .field("root_path", &"<redacted>")
            .field("status", &self.status)
            .field("queued_at", &self.queued_at)
            .field("started_at", &self.started_at)
            .field("finished_at", &self.finished_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportTaskStatus {
    Queued,
    Running,
    Completed,
    FailedRetryable,
    FailedPermanent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreStatusSummary {
    pub indexed_documents: u64,
    pub searchable_documents: u64,
    pub partial_documents: u64,
    pub failed_retryable: u64,
    pub failed_permanent: u64,
    pub ocr_queue_depth: u64,
    pub embedding_queue_depth: u64,
    pub recovery_queue_depth: u64,
    pub import_tasks_queued: u64,
    pub import_tasks_recoverable: u64,
    pub ocr_jobs_queued: u64,
    pub entity_mentions: u64,
    pub index_health: IndexStateStatus,
    pub last_snapshot_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DocumentStatusCounts {
    indexed_documents: i64,
    searchable_documents: i64,
    partial_documents: i64,
    failed_retryable: i64,
    failed_permanent: i64,
    ocr_queue_depth: i64,
    embedding_queue_depth: i64,
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

const SCHEMA_V2: &str = r#"
CREATE TABLE import_task (
    id TEXT PRIMARY KEY,
    root_path TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'queued',
        'running',
        'completed',
        'failed_retryable',
        'failed_permanent'
    )),
    queued_at_seconds INTEGER NOT NULL,
    started_at_seconds INTEGER,
    finished_at_seconds INTEGER,
    updated_at_seconds INTEGER NOT NULL,
    CHECK (queued_at_seconds <= updated_at_seconds),
    CHECK (
        started_at_seconds IS NULL
        OR (queued_at_seconds <= started_at_seconds AND started_at_seconds <= updated_at_seconds)
    ),
    CHECK (
        finished_at_seconds IS NULL
        OR (
            started_at_seconds IS NOT NULL
            AND started_at_seconds <= finished_at_seconds
            AND finished_at_seconds <= updated_at_seconds
        )
    ),
    CHECK (
        (
            status = 'queued'
            AND started_at_seconds IS NULL
            AND finished_at_seconds IS NULL
        )
        OR (
            status = 'running'
            AND started_at_seconds IS NOT NULL
            AND finished_at_seconds IS NULL
        )
        OR (
            status IN ('completed', 'failed_retryable', 'failed_permanent')
            AND started_at_seconds IS NOT NULL
            AND finished_at_seconds IS NOT NULL
        )
    )
);

CREATE INDEX import_task_status_idx
    ON import_task(status, queued_at_seconds);
"#;

const SCHEMA_V3: &str = r#"
CREATE TABLE ingest_job_v3 (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    resume_version_id TEXT,
    kind TEXT NOT NULL CHECK (kind IN (
        'discover_document',
        'fingerprint_document',
        'parse_document',
        'ocr_document',
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

INSERT INTO ingest_job_v3 (
    id, document_id, resume_version_id, kind, status, attempt_count, max_attempts,
    queued_at_seconds, started_at_seconds, finished_at_seconds, updated_at_seconds
)
SELECT
    id, document_id, resume_version_id, kind, status, attempt_count, max_attempts,
    queued_at_seconds, started_at_seconds, finished_at_seconds, updated_at_seconds
FROM ingest_job;

DROP TABLE ingest_job;
ALTER TABLE ingest_job_v3 RENAME TO ingest_job;

CREATE INDEX ingest_job_recovery_idx
    ON ingest_job(status, attempt_count, max_attempts);
CREATE UNIQUE INDEX ingest_job_ocr_document_unique_idx
    ON ingest_job(document_id, kind)
    WHERE kind = 'ocr_document';
"#;

const SCHEMA_V4: &str = r#"
CREATE TABLE entity_mention (
    id TEXT PRIMARY KEY,
    resume_version_id TEXT NOT NULL,
    section_id TEXT,
    entity_type TEXT NOT NULL CHECK (
        entity_type IN (
            'name',
            'email',
            'phone',
            'school',
            'degree',
            'company',
            'title',
            'education',
            'skills',
            'skill',
            'certificate',
            'date',
            'date_range',
            'years_experience',
            'location'
        )
        OR entity_type LIKE 'other:%'
    ),
    raw_value TEXT NOT NULL,
    normalized_value TEXT,
    span_start INTEGER CHECK (span_start IS NULL OR span_start >= 0),
    span_end INTEGER CHECK (span_end IS NULL OR span_end >= 0),
    confidence REAL NOT NULL CHECK (confidence >= 0 AND confidence <= 1),
    extractor TEXT NOT NULL,
    CHECK (
        span_start IS NULL
        OR span_end IS NULL
        OR span_start <= span_end
    ),
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE
);

CREATE INDEX entity_mention_version_idx
    ON entity_mention(resume_version_id, entity_type);
CREATE INDEX entity_mention_type_value_idx
    ON entity_mention(entity_type, normalized_value, confidence);
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

fn read_entity_mention(row: &Row<'_>) -> Result<EntityMention> {
    let span_start = read_optional_i64(row, 6)?
        .map(|value| i64_to_usize(value, "entity_mention.span_start"))
        .transpose()?;
    let span_end = read_optional_i64(row, 7)?
        .map(|value| i64_to_usize(value, "entity_mention.span_end"))
        .transpose()?;

    Ok(EntityMention {
        id: read_id::<EntityMentionId>(row, 0, "entity_mention.id")?,
        resume_version_id: read_id::<ResumeVersionId>(row, 1, "entity_mention.resume_version_id")?,
        section_id: read_optional_id::<SectionId>(row, 2, "entity_mention.section_id")?,
        entity_type: entity_type_from_storage(&read_string(row, 3)?)?,
        raw_value: read_string(row, 4)?,
        normalized_value: read_optional_string(row, 5)?,
        span_start,
        span_end,
        confidence: row.get::<_, f64>(8).map_err(MetaStoreError::storage)? as f32,
        extractor: read_string(row, 9)?,
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

fn read_import_task(row: &Row<'_>) -> Result<ImportTask> {
    Ok(ImportTask {
        id: read_id::<ImportTaskId>(row, 0, "import_task.id")?,
        root_path: read_string(row, 1)?,
        status: import_task_status_from_storage(&read_string(row, 2)?)?,
        queued_at: UnixTimestamp::from_unix_seconds(read_i64(row, 3)?),
        started_at: read_optional_timestamp(row, 4)?,
        finished_at: read_optional_timestamp(row, 5)?,
        updated_at: UnixTimestamp::from_unix_seconds(read_i64(row, 6)?),
    })
}

fn validate_import_task(task: &ImportTask) -> Result<()> {
    let queued_at = task.queued_at.as_unix_seconds();
    let updated_at = task.updated_at.as_unix_seconds();
    if queued_at > updated_at {
        return Err(MetaStoreError::invalid_value("import_task.timestamps"));
    }

    let started_at = task.started_at.map(UnixTimestamp::as_unix_seconds);
    let finished_at = task.finished_at.map(UnixTimestamp::as_unix_seconds);

    if let Some(started_at) = started_at {
        if started_at < queued_at || started_at > updated_at {
            return Err(MetaStoreError::invalid_value("import_task.timestamps"));
        }
    }

    if let Some(finished_at) = finished_at {
        let Some(started_at) = started_at else {
            return Err(MetaStoreError::invalid_value("import_task.timestamps"));
        };
        if finished_at < started_at || finished_at > updated_at {
            return Err(MetaStoreError::invalid_value("import_task.timestamps"));
        }
    }

    let valid_state = match task.status {
        ImportTaskStatus::Queued => started_at.is_none() && finished_at.is_none(),
        ImportTaskStatus::Running => started_at.is_some() && finished_at.is_none(),
        ImportTaskStatus::Completed
        | ImportTaskStatus::FailedRetryable
        | ImportTaskStatus::FailedPermanent => started_at.is_some() && finished_at.is_some(),
    };

    if !valid_state {
        return Err(MetaStoreError::invalid_value("import_task.lifecycle"));
    }

    Ok(())
}

fn validate_entity_mention(version_id: &ResumeVersionId, mention: &EntityMention) -> Result<()> {
    if &mention.resume_version_id != version_id {
        return Err(MetaStoreError::invalid_value(
            "entity_mention.resume_version_id",
        ));
    }
    if mention.raw_value.trim().is_empty() {
        return Err(MetaStoreError::invalid_value("entity_mention.raw_value"));
    }
    if mention.extractor.trim().is_empty() {
        return Err(MetaStoreError::invalid_value("entity_mention.extractor"));
    }
    if !mention.confidence.is_finite() || !(0.0..=1.0).contains(&mention.confidence) {
        return Err(MetaStoreError::invalid_value("entity_mention.confidence"));
    }
    if let (Some(span_start), Some(span_end)) = (mention.span_start, mention.span_end) {
        if span_start > span_end {
            return Err(MetaStoreError::invalid_value("entity_mention.span"));
        }
    }

    Ok(())
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

fn usize_to_i64(value: usize, field: &'static str) -> Result<i64> {
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

fn i64_to_usize(value: i64, field: &'static str) -> Result<usize> {
    usize::try_from(value).map_err(|_| MetaStoreError::invalid_value(field))
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

fn entity_type_to_storage(entity_type: &EntityType) -> String {
    match entity_type {
        EntityType::Name => "name".to_string(),
        EntityType::Email => "email".to_string(),
        EntityType::Phone => "phone".to_string(),
        EntityType::School => "school".to_string(),
        EntityType::Degree => "degree".to_string(),
        EntityType::Company => "company".to_string(),
        EntityType::Title => "title".to_string(),
        EntityType::Education => "education".to_string(),
        EntityType::Skills => "skills".to_string(),
        EntityType::Skill => "skill".to_string(),
        EntityType::Certificate => "certificate".to_string(),
        EntityType::Date => "date".to_string(),
        EntityType::DateRange => "date_range".to_string(),
        EntityType::YearsExperience => "years_experience".to_string(),
        EntityType::Location => "location".to_string(),
        EntityType::Other(value) => format!("other:{value}"),
    }
}

fn entity_type_from_storage(value: &str) -> Result<EntityType> {
    match value {
        "name" => Ok(EntityType::Name),
        "email" => Ok(EntityType::Email),
        "phone" => Ok(EntityType::Phone),
        "school" => Ok(EntityType::School),
        "degree" => Ok(EntityType::Degree),
        "company" => Ok(EntityType::Company),
        "title" => Ok(EntityType::Title),
        "education" => Ok(EntityType::Education),
        "skills" => Ok(EntityType::Skills),
        "skill" => Ok(EntityType::Skill),
        "certificate" => Ok(EntityType::Certificate),
        "date" => Ok(EntityType::Date),
        "date_range" => Ok(EntityType::DateRange),
        "years_experience" => Ok(EntityType::YearsExperience),
        "location" => Ok(EntityType::Location),
        _ => value
            .strip_prefix("other:")
            .map(|value| EntityType::Other(value.to_string()))
            .ok_or_else(|| MetaStoreError::invalid_value("entity_mention.entity_type")),
    }
}

fn ingest_job_kind_to_storage(kind: IngestJobKind) -> &'static str {
    match kind {
        IngestJobKind::DiscoverDocument => "discover_document",
        IngestJobKind::FingerprintDocument => "fingerprint_document",
        IngestJobKind::ParseDocument => "parse_document",
        IngestJobKind::OcrDocument => "ocr_document",
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
        "ocr_document" => Ok(IngestJobKind::OcrDocument),
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

fn import_task_status_transition_allowed(
    current: ImportTaskStatus,
    next: ImportTaskStatus,
) -> bool {
    match current {
        ImportTaskStatus::Queued => matches!(next, ImportTaskStatus::Running),
        ImportTaskStatus::Running => matches!(
            next,
            ImportTaskStatus::Completed
                | ImportTaskStatus::FailedRetryable
                | ImportTaskStatus::FailedPermanent
        ),
        ImportTaskStatus::FailedRetryable => matches!(next, ImportTaskStatus::Running),
        ImportTaskStatus::Completed | ImportTaskStatus::FailedPermanent => false,
    }
}

fn next_import_task_state(
    current: &ImportTask,
    status: ImportTaskStatus,
    updated_at: UnixTimestamp,
) -> ImportTask {
    let mut next = current.clone();
    next.status = status;
    next.updated_at = updated_at;
    match status {
        ImportTaskStatus::Running => {
            next.started_at = Some(updated_at);
            next.finished_at = None;
        }
        ImportTaskStatus::Completed
        | ImportTaskStatus::FailedRetryable
        | ImportTaskStatus::FailedPermanent => {
            if next.started_at.is_none() {
                next.started_at = Some(updated_at);
            }
            next.finished_at = Some(updated_at);
        }
        ImportTaskStatus::Queued => {}
    }
    next
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

fn import_task_status_to_storage(status: ImportTaskStatus) -> &'static str {
    match status {
        ImportTaskStatus::Queued => "queued",
        ImportTaskStatus::Running => "running",
        ImportTaskStatus::Completed => "completed",
        ImportTaskStatus::FailedRetryable => "failed_retryable",
        ImportTaskStatus::FailedPermanent => "failed_permanent",
    }
}

fn import_task_status_from_storage(value: &str) -> Result<ImportTaskStatus> {
    match value {
        "queued" => Ok(ImportTaskStatus::Queued),
        "running" => Ok(ImportTaskStatus::Running),
        "completed" => Ok(ImportTaskStatus::Completed),
        "failed_retryable" => Ok(ImportTaskStatus::FailedRetryable),
        "failed_permanent" => Ok(ImportTaskStatus::FailedPermanent),
        _ => Err(MetaStoreError::invalid_value("import_task.status")),
    }
}
