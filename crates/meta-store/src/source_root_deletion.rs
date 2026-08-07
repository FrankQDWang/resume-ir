use std::str::FromStr;

use core_domain::DocumentId;
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};

use crate::{
    schema_v37, MetaStoreError, MetadataStore, MetadataStoreAccess, MetadataStoreWriteAccess,
    Result, SourceRootId, UnixTimestamp,
};

#[path = "source_root_deletion_checkpoint.rs"]
mod checkpoint;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceRootDeletionPhase {
    Requested,
    Quiescing,
    Publishing,
    Purging,
    Verifying,
    Complete,
    Failed,
}

impl SourceRootDeletionPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Quiescing => "quiescing",
            Self::Publishing => "publishing",
            Self::Purging => "purging",
            Self::Verifying => "verifying",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self> {
        match value {
            "requested" => Ok(Self::Requested),
            "quiescing" => Ok(Self::Quiescing),
            "publishing" => Ok(Self::Publishing),
            "purging" => Ok(Self::Purging),
            "verifying" => Ok(Self::Verifying),
            "complete" => Ok(Self::Complete),
            "failed" => Ok(Self::Failed),
            _ => Err(MetaStoreError::invalid_value("source_root_deletion.phase")),
        }
    }

    pub(super) fn is_active(self) -> bool {
        !matches!(self, Self::Complete | Self::Failed)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceRootDeletionErrorCode {
    ImportQuiescenceTimeout,
    OcrQuiescenceTimeout,
    PublicationFailed,
    MetadataPurgeFailed,
    PrivacyCleanupFailed,
    ReceiptCompletionFailed,
    Internal,
}

impl SourceRootDeletionErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ImportQuiescenceTimeout => "import_quiescence_timeout",
            Self::OcrQuiescenceTimeout => "ocr_quiescence_timeout",
            Self::PublicationFailed => "publication_failed",
            Self::MetadataPurgeFailed => "metadata_purge_failed",
            Self::PrivacyCleanupFailed => "privacy_cleanup_failed",
            Self::ReceiptCompletionFailed => "receipt_completion_failed",
            Self::Internal => "internal",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "import_quiescence_timeout" => Ok(Self::ImportQuiescenceTimeout),
            "ocr_quiescence_timeout" => Ok(Self::OcrQuiescenceTimeout),
            "publication_failed" => Ok(Self::PublicationFailed),
            "metadata_purge_failed" => Ok(Self::MetadataPurgeFailed),
            "privacy_cleanup_failed" => Ok(Self::PrivacyCleanupFailed),
            "receipt_completion_failed" => Ok(Self::ReceiptCompletionFailed),
            "internal" => Ok(Self::Internal),
            _ => Err(MetaStoreError::invalid_value(
                "source_root_deletion_attempt_evidence.last_error_code",
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRootDeletion {
    pub root_id: SourceRootId,
    pub phase: SourceRootDeletionPhase,
    pub affected_documents: u64,
    pub removed_documents: u64,
    pub started_at: UnixTimestamp,
    pub updated_at: UnixTimestamp,
    pub completed_at: Option<UnixTimestamp>,
    pub attempt_count: u64,
    pub last_attempt_at: Option<UnixTimestamp>,
    pub last_error_phase: Option<SourceRootDeletionPhase>,
    pub last_error_code: Option<SourceRootDeletionErrorCode>,
    pub last_error_at: Option<UnixTimestamp>,
}

type PersistedSourceRootDeletion = (
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    Option<i64>,
    i64,
    Option<i64>,
    Option<String>,
    Option<String>,
    Option<i64>,
);

const SOURCE_ROOT_DELETION_COLUMNS: &str =
    "deletion.root_id, deletion.phase, deletion.affected_documents, \
     deletion.removed_documents, deletion.started_at_seconds, \
     deletion.updated_at_seconds, deletion.completed_at_seconds, \
     evidence.attempt_count, evidence.last_attempt_at_seconds, \
     evidence.last_error_phase, evidence.last_error_code, \
     evidence.last_error_at_seconds";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct SourceRootDeletionCompletionResiduals {
    source_occurrences: u64,
    scan_snapshots: u64,
    pdf_reprocess_jobs: u64,
    import_tasks: u64,
    authorized_import_roots: u64,
    documents: u64,
    active_search_projections: u64,
    ocr_page_cache: u64,
    total: u64,
}

impl SourceRootDeletionCompletionResiduals {
    fn is_empty(self) -> bool {
        self.total == 0
    }
}

const SOURCE_ROOT_DELETION_COMPLETION_RESIDUALS_SQL: &str = r#"WITH residuals AS (
    SELECT
        (SELECT COUNT(*) FROM source_occurrence WHERE root_id = ?1) AS source_occurrences,
        (SELECT COUNT(*) FROM scan_snapshot WHERE root_id = ?1) AS scan_snapshots,
        (SELECT COUNT(*) FROM pdf_reprocess_job WHERE root_id = ?1) AS pdf_reprocess_jobs,
        (
            SELECT COUNT(*) FROM import_task
            WHERE root_path = (SELECT canonical_path FROM source_root_deletion WHERE root_id = ?1)
        ) AS import_tasks,
        (
            SELECT COUNT(*) FROM authorized_import_root
            WHERE canonical_root_path = (
                SELECT canonical_path FROM source_root_deletion WHERE root_id = ?1)
        ) AS authorized_import_roots,
        (
            SELECT COUNT(*) FROM document
            WHERE id IN (
                SELECT document_id FROM source_root_deletion_document WHERE root_id = ?1)
        ) AS documents,
        (
            SELECT COUNT(*) FROM active_search_projection AS projection
            WHERE projection.document_id IN (
                SELECT document_id FROM source_root_deletion_document WHERE root_id = ?1)
        ) AS active_search_projections,
        (
            SELECT COUNT(*) FROM ocr_page_cache AS cache
            WHERE cache.file_content_hash IN (
                SELECT DISTINCT doomed.content_hash
                FROM source_root_deletion_document AS doomed
                WHERE doomed.root_id = ?1
                  AND NOT EXISTS (
                    SELECT 1
                    FROM source_revision AS retained_revision
                    JOIN document AS retained
                      ON retained.id = retained_revision.document_id
                    WHERE retained_revision.document_id NOT IN (
                        SELECT document_id FROM source_root_deletion_document
                        WHERE root_id = ?1
                    )
                      AND retained.is_deleted = 0
                      AND retained.status <> 'deleted'
                      AND retained_revision.content_hash = doomed.content_hash
                  )
            )
        ) AS ocr_page_cache
)
SELECT source_occurrences, scan_snapshots, pdf_reprocess_jobs,
       import_tasks, authorized_import_roots, documents,
       active_search_projections, ocr_page_cache,
    source_occurrences + scan_snapshots + pdf_reprocess_jobs
        + import_tasks + authorized_import_roots + documents
        + active_search_projections + ocr_page_cache AS total
FROM residuals"#;

fn read_source_root_deletion_completion_residuals(
    connection: &Connection,
    root_id: &SourceRootId,
) -> Result<SourceRootDeletionCompletionResiduals> {
    let persisted = connection
        .query_row(
            SOURCE_ROOT_DELETION_COMPLETION_RESIDUALS_SQL,
            params![root_id.as_str()],
            |row| {
                Ok([
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                ])
            },
        )
        .map_err(MetaStoreError::storage)?;
    Ok(SourceRootDeletionCompletionResiduals {
        source_occurrences: to_u64(persisted[0])?,
        scan_snapshots: to_u64(persisted[1])?,
        pdf_reprocess_jobs: to_u64(persisted[2])?,
        import_tasks: to_u64(persisted[3])?,
        authorized_import_roots: to_u64(persisted[4])?,
        documents: to_u64(persisted[5])?,
        active_search_projections: to_u64(persisted[6])?,
        ocr_page_cache: to_u64(persisted[7])?,
        total: to_u64(persisted[8])?,
    })
}

const MAX_RECENT_DELETION_ATTEMPTS: usize = 16;

impl<Access: MetadataStoreAccess> MetadataStore<Access> {
    pub fn source_root_deletion(
        &self,
        root_id: &SourceRootId,
    ) -> Result<Option<SourceRootDeletion>> {
        self.connection
            .borrow()
            .query_row(
                &format!(
                    "SELECT {SOURCE_ROOT_DELETION_COLUMNS}
                 FROM source_root_deletion AS deletion
                 LEFT JOIN source_root_deletion_attempt_evidence AS evidence
                   ON evidence.root_id = deletion.root_id
                 WHERE deletion.root_id = ?1"
                ),
                params![root_id.as_str()],
                read_persisted_deletion,
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .map(source_root_deletion_from_persisted)
            .transpose()
    }

    pub fn recent_source_root_deletion_attempts(&self) -> Result<Vec<SourceRootDeletion>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(&format!(
                "SELECT {SOURCE_ROOT_DELETION_COLUMNS}
                 FROM source_root_deletion AS deletion
                 JOIN source_root_deletion_attempt_evidence AS evidence
                   ON evidence.root_id = deletion.root_id
                 WHERE evidence.attempt_count > 0
                   AND deletion.phase NOT IN ('complete', 'failed')
                 ORDER BY evidence.last_attempt_at_seconds DESC, deletion.root_id
                 LIMIT {MAX_RECENT_DELETION_ATTEMPTS}"
            ))
            .map_err(MetaStoreError::storage)?;
        let attempts = statement
            .query_map([], read_persisted_deletion)
            .map_err(MetaStoreError::storage)?
            .map(|row| {
                row.map_err(MetaStoreError::storage)
                    .and_then(source_root_deletion_from_persisted)
            })
            .collect();
        attempts
    }

    pub fn incomplete_source_root_deletions(&self) -> Result<Vec<SourceRootDeletion>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT root_id FROM source_root_deletion
                 WHERE phase NOT IN ('complete', 'failed')
                 ORDER BY started_at_seconds, root_id",
            )
            .map_err(MetaStoreError::storage)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?;
        ids.into_iter()
            .map(|id| {
                let id = SourceRootId::from_str(&id)?;
                self.source_root_deletion(&id)?
                    .ok_or_else(MetaStoreError::storage_invariant)
            })
            .collect()
    }

    pub fn source_root_deletion_in_progress(&self, root_id: &SourceRootId) -> Result<bool> {
        Ok(self.source_root_deletion(root_id)?.is_some_and(|receipt| {
            !matches!(
                receipt.phase,
                SourceRootDeletionPhase::Complete | SourceRootDeletionPhase::Failed
            )
        }))
    }

    pub fn begin_source_root_deletion(
        &self,
        root_id: &SourceRootId,
        now: UnixTimestamp,
    ) -> Result<SourceRootDeletion>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let root_exists = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM source_root WHERE id = ?1)",
                params![root_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(MetaStoreError::storage)?
            != 0;
        if !root_exists {
            return Err(MetaStoreError::not_found("source_root"));
        }
        crate::source_root_commit_fence::bump_epoch(&transaction, root_id)?;
        let snapshot = checkpoint::CurrentSourceRootDeletionSnapshot::read(&transaction, root_id)?;
        transaction
            .execute(
                "INSERT INTO source_root_deletion (
                    root_id, canonical_path, phase,
                    affected_documents, removed_documents,
                    started_at_seconds, updated_at_seconds,
                    checkpoint_protocol_version
                 )
                 SELECT id, canonical_path, 'requested', ?2, 0, ?3, ?3, ?4
                 FROM source_root
                 WHERE id = ?1
                 ON CONFLICT(root_id) DO UPDATE SET
                    canonical_path = excluded.canonical_path,
                    phase = 'requested',
                    affected_documents = excluded.affected_documents,
                    removed_documents = 0,
                    started_at_seconds = excluded.started_at_seconds,
                    updated_at_seconds = excluded.updated_at_seconds,
                    completed_at_seconds = NULL,
                    checkpoint_protocol_version = excluded.checkpoint_protocol_version",
                params![
                    root_id.as_str(),
                    snapshot.affected_documents()?,
                    now.as_unix_seconds(),
                    schema_v37::SNAPSHOT_INVARIANT_V2,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "INSERT INTO source_root_deletion_attempt_evidence (
                    root_id, attempt_count, last_attempt_at_seconds,
                    last_error_phase, last_error_code, last_error_at_seconds
                 ) VALUES (?1, 0, NULL, NULL, NULL, NULL)
                 ON CONFLICT(root_id) DO UPDATE SET
                    attempt_count = 0,
                    last_attempt_at_seconds = NULL,
                    last_error_phase = NULL,
                    last_error_code = NULL,
                    last_error_at_seconds = NULL",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        snapshot.replace(&transaction, root_id)?;
        transaction.commit().map_err(MetaStoreError::storage)?;
        drop(connection);
        self.source_root_deletion(root_id)?
            .ok_or_else(MetaStoreError::storage_invariant)
    }

    pub fn begin_source_root_deletion_attempt(
        &self,
        root_id: &SourceRootId,
        now: UnixTimestamp,
    ) -> Result<SourceRootDeletion>
    where
        Access: MetadataStoreWriteAccess,
    {
        checkpoint::begin_attempt(&mut self.connection.borrow_mut(), root_id, now)?;
        self.source_root_deletion(root_id)?
            .ok_or_else(MetaStoreError::storage_invariant)
    }

    pub fn record_source_root_deletion_attempt_failure(
        &self,
        root_id: &SourceRootId,
        phase: SourceRootDeletionPhase,
        code: SourceRootDeletionErrorCode,
        now: UnixTimestamp,
    ) -> Result<SourceRootDeletion>
    where
        Access: MetadataStoreWriteAccess,
    {
        if matches!(
            phase,
            SourceRootDeletionPhase::Complete | SourceRootDeletionPhase::Failed
        ) {
            return Err(MetaStoreError::invalid_transition());
        }
        let changed = self
            .connection
            .borrow()
            .execute(
                "UPDATE source_root_deletion_attempt_evidence
                 SET last_error_phase = ?2,
                     last_error_code = ?3,
                     last_error_at_seconds = MAX(last_attempt_at_seconds, ?4)
                 WHERE root_id = ?1
                   AND last_attempt_at_seconds IS NOT NULL
                   AND EXISTS (
                     SELECT 1 FROM source_root_deletion AS deletion
                     WHERE deletion.root_id = ?1 AND deletion.phase = ?2
                   )",
                params![
                    root_id.as_str(),
                    phase.as_str(),
                    code.as_str(),
                    now.as_unix_seconds(),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::invalid_transition());
        }
        self.source_root_deletion(root_id)?
            .ok_or_else(MetaStoreError::storage_invariant)
    }

    pub fn set_source_root_deletion_phase(
        &self,
        root_id: &SourceRootId,
        phase: SourceRootDeletionPhase,
        now: UnixTimestamp,
    ) -> Result<()>
    where
        Access: MetadataStoreWriteAccess,
    {
        let current = self
            .source_root_deletion(root_id)?
            .ok_or_else(|| MetaStoreError::not_found("source_root_deletion"))?
            .phase;
        if !deletion_phase_transition_allowed(current, phase) {
            return Err(MetaStoreError::invalid_transition());
        }
        if matches!(
            (current, phase),
            (
                SourceRootDeletionPhase::Requested,
                SourceRootDeletionPhase::Quiescing
            )
        ) {
            return checkpoint::advance_requested_to_quiescing(
                &mut self.connection.borrow_mut(),
                root_id,
                now,
            );
        }
        let completed =
            (phase == SourceRootDeletionPhase::Complete).then_some(now.as_unix_seconds());
        let changed = self
            .connection
            .borrow()
            .execute(
                "UPDATE source_root_deletion
                 SET phase = ?2, updated_at_seconds = MAX(updated_at_seconds, ?3),
                     completed_at_seconds = ?4
                 WHERE root_id = ?1 AND phase = ?5",
                params![
                    root_id.as_str(),
                    phase.as_str(),
                    now.as_unix_seconds(),
                    completed,
                    current.as_str()
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::not_found("source_root_deletion"));
        }
        Ok(())
    }

    pub fn source_root_unreferenced_content_hashes(
        &self,
        root_id: &SourceRootId,
    ) -> Result<Vec<String>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT doomed.content_hash
                 FROM source_root_deletion_document AS doomed
                 WHERE doomed.root_id = ?1
                   AND NOT EXISTS (
                     SELECT 1
                     FROM source_revision AS retained_revision
                     JOIN document AS retained
                       ON retained.id = retained_revision.document_id
                     WHERE retained_revision.document_id NOT IN (
                         SELECT document_id FROM source_root_deletion_document
                         WHERE root_id = ?1
                     )
                       AND retained.is_deleted = 0
                       AND retained.status <> 'deleted'
                       AND retained_revision.content_hash = doomed.content_hash
                   )
                 ORDER BY doomed.content_hash",
            )
            .map_err(MetaStoreError::storage)?;
        let hashes = statement
            .query_map(params![root_id.as_str()], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage);
        hashes
    }

    pub fn purge_source_root_data(&self, root_id: &SourceRootId, now: UnixTimestamp) -> Result<u64>
    where
        Access: MetadataStoreWriteAccess,
    {
        let documents = self.source_root_deletion_document_ids(root_id)?;
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        for document_id in &documents {
            transaction
                .execute(
                    "UPDATE document
                     SET is_deleted = 1, status = 'deleted',
                         updated_at_seconds = MAX(updated_at_seconds, ?2)
                     WHERE id = ?1",
                    params![document_id.as_str(), now.as_unix_seconds()],
                )
                .map_err(MetaStoreError::storage)?;
        }
        transaction
            .execute(
                "DELETE FROM source_occurrence WHERE root_id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM scan_snapshot WHERE root_id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM import_task
                 WHERE root_path = (
                    SELECT canonical_path FROM source_root WHERE id = ?1
                 )",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        transaction
            .execute(
                "DELETE FROM authorized_import_root
                 WHERE canonical_root_path = (
                    SELECT canonical_path FROM source_root WHERE id = ?1
                 )",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        let receipt_updated = transaction
            .execute(
                "UPDATE source_root_deletion
                 SET removed_documents = ?2, phase = 'verifying',
                     updated_at_seconds = MAX(updated_at_seconds, ?3)
                 WHERE root_id = ?1 AND phase = 'purging'",
                params![
                    root_id.as_str(),
                    i64::try_from(documents.len())
                        .map_err(|_| MetaStoreError::storage_invariant())?,
                    now.as_unix_seconds()
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if receipt_updated != 1 {
            return Err(MetaStoreError::invalid_transition());
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        u64::try_from(documents.len()).map_err(|_| MetaStoreError::storage_invariant())
    }

    pub fn complete_source_root_deletion(
        &self,
        root_id: &SourceRootId,
        now: UnixTimestamp,
    ) -> Result<SourceRootDeletion>
    where
        Access: MetadataStoreWriteAccess,
    {
        let mut connection = self.connection.borrow_mut();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(MetaStoreError::storage)?;
        let residuals = read_source_root_deletion_completion_residuals(&transaction, root_id)?;
        if !residuals.is_empty() {
            return Err(MetaStoreError::storage_invariant());
        }
        let removed = transaction
            .execute(
                "DELETE FROM source_root WHERE id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        if removed != 1 {
            return Err(MetaStoreError::not_found("source_root"));
        }
        transaction
            .execute(
                "DELETE FROM source_root_deletion_document WHERE root_id = ?1",
                params![root_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        let changed = transaction
            .execute(
                "UPDATE source_root_deletion
                 SET phase = 'complete',
                     updated_at_seconds = MAX(updated_at_seconds, ?2),
                     completed_at_seconds = ?2
                 WHERE root_id = ?1 AND phase = 'verifying'",
                params![root_id.as_str(), now.as_unix_seconds()],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::invalid_transition());
        }
        transaction.commit().map_err(MetaStoreError::storage)?;
        drop(connection);
        self.source_root_deletion(root_id)?
            .ok_or_else(MetaStoreError::storage_invariant)
    }

    pub fn source_root_deletion_document_ids(
        &self,
        root_id: &SourceRootId,
    ) -> Result<Vec<DocumentId>> {
        let connection = self.connection.borrow();
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT document_id
                 FROM source_root_deletion_document
                 WHERE root_id = ?1
                 ORDER BY document_id",
            )
            .map_err(MetaStoreError::storage)?;
        let documents = statement
            .query_map(params![root_id.as_str()], |row| row.get::<_, String>(0))
            .map_err(MetaStoreError::storage)?
            .map(|row| {
                let id = row.map_err(MetaStoreError::storage)?;
                DocumentId::from_str(&id).map_err(|_| MetaStoreError::invalid_value("document.id"))
            })
            .collect();
        documents
    }
}

fn read_persisted_deletion(row: &Row<'_>) -> rusqlite::Result<PersistedSourceRootDeletion> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
    ))
}

fn source_root_deletion_from_persisted(
    persisted: PersistedSourceRootDeletion,
) -> Result<SourceRootDeletion> {
    let (
        root_id,
        phase,
        affected,
        removed,
        started,
        updated,
        completed,
        attempt_count,
        last_attempt,
        last_error_phase,
        last_error_code,
        last_error_at,
    ) = persisted;
    Ok(SourceRootDeletion {
        root_id: SourceRootId::from_str(&root_id)?,
        phase: SourceRootDeletionPhase::parse(&phase)?,
        affected_documents: to_u64(affected)?,
        removed_documents: to_u64(removed)?,
        started_at: UnixTimestamp::from_unix_seconds(started),
        updated_at: UnixTimestamp::from_unix_seconds(updated),
        completed_at: completed.map(UnixTimestamp::from_unix_seconds),
        attempt_count: to_u64(attempt_count)?,
        last_attempt_at: last_attempt.map(UnixTimestamp::from_unix_seconds),
        last_error_phase: last_error_phase
            .as_deref()
            .map(SourceRootDeletionPhase::parse)
            .transpose()?,
        last_error_code: last_error_code
            .as_deref()
            .map(SourceRootDeletionErrorCode::parse)
            .transpose()?,
        last_error_at: last_error_at.map(UnixTimestamp::from_unix_seconds),
    })
}

fn to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| MetaStoreError::storage_invariant())
}

fn deletion_phase_transition_allowed(
    current: SourceRootDeletionPhase,
    next: SourceRootDeletionPhase,
) -> bool {
    current == next
        || matches!(
            (current, next),
            (
                SourceRootDeletionPhase::Requested,
                SourceRootDeletionPhase::Quiescing
            ) | (
                SourceRootDeletionPhase::Quiescing,
                SourceRootDeletionPhase::Publishing
            ) | (
                SourceRootDeletionPhase::Publishing,
                SourceRootDeletionPhase::Purging
            ) | (
                SourceRootDeletionPhase::Purging,
                SourceRootDeletionPhase::Verifying
            ) | (
                SourceRootDeletionPhase::Verifying,
                SourceRootDeletionPhase::Complete
            ) | (_, SourceRootDeletionPhase::Failed)
        )
}

#[cfg(test)]
#[path = "source_root_deletion_tests.rs"]
mod tests;
