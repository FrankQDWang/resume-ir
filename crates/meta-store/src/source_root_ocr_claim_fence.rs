use std::path::Path;
use std::str::FromStr;

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::{
    classification, document_status_to_storage, ingest_job_kind_to_storage,
    ingest_job_status_to_storage, schema_v38, u32_to_i64, ClaimedOcrJob, DocumentStatus,
    IngestJobId, IngestJobKind, IngestJobStatus, MetaStoreError, OcrJobDiscardReason, Result,
    SourceRootDeletionPhase, SourceRootId, UnixTimestamp,
};

const CLAIM_CANDIDATE_LIMIT: i64 = 256;
const CLAIM_BATCH_LIMIT: usize = 2;

const CLAIM_NEXT_JOBS_SQL: &str =
    "SELECT job.id, job.document_id, job.queued_at_seconds, job.rowid AS job_rowid
     FROM ingest_job AS job INDEXED BY ingest_job_ocr_claim_queue_idx
     WHERE job.kind = 'ocr_document'
       AND (
         job.status = 'queued'
         OR (
           job.status IN ('interrupted', 'failed_retryable')
           AND job.attempt_count < job.max_attempts
         )
       )
     ORDER BY job.queued_at_seconds, job.rowid
     LIMIT :candidate_limit";

const RUNNING_JOB_SQL: &str =
    "SELECT job.id, job.document_id, job.queued_at_seconds, job.rowid AS job_rowid
     FROM ingest_job AS job
     WHERE job.id = :running_job AND job.kind = 'ocr_document' AND job.status = 'running'
     LIMIT 1";

const CLAIM_CANDIDATES_BODY_SQL: &str =
    "SELECT candidate_job.id, candidate_job.document_id, spec.source_revision_id,
            spec.triage_epoch, occurrence.root_id, occurrence.relative_path,
            root.revocation_epoch, root.canonical_path, document.normalized_path,
            deletion.phase
     FROM candidate_jobs AS candidate_job
     JOIN ocr_job_spec AS spec ON spec.ingest_job_id = candidate_job.id
     JOIN source_revision_triage AS triage
       ON triage.source_revision_id = spec.source_revision_id
      AND triage.triage_epoch = spec.triage_epoch
     JOIN source_revision AS revision ON revision.id = spec.source_revision_id
     JOIN document
       ON document.id = candidate_job.document_id
      AND document.id = revision.document_id
      AND document.content_hash = revision.content_hash
     JOIN source_occurrence AS occurrence ON occurrence.rowid = (
       SELECT ranked_occurrence.rowid
       FROM (
         SELECT candidate_occurrence.rowid,
                CASE WHEN document.normalized_path = CASE
                  WHEN candidate_root.canonical_path = '/'
                    THEN '/' || candidate_occurrence.relative_path
                  ELSE candidate_root.canonical_path || '/' || candidate_occurrence.relative_path
                END THEN 0 ELSE 1 END AS path_mismatch,
                candidate_root.id AS root_id,
                candidate_occurrence.relative_path
         FROM source_occurrence AS candidate_occurrence
         JOIN source_root AS candidate_root ON candidate_root.id = candidate_occurrence.root_id
         WHERE candidate_occurrence.document_id = document.id
           AND candidate_occurrence.source_revision_id = revision.id
           AND candidate_occurrence.state = 'present'
           AND typeof(candidate_root.revocation_epoch) = 'integer'
           AND candidate_root.revocation_epoch BETWEEN 0 AND :max_root_epoch
       ) AS ranked_occurrence
       ORDER BY ranked_occurrence.path_mismatch, ranked_occurrence.root_id,
                ranked_occurrence.relative_path
       LIMIT 1
     )
     JOIN source_root AS root ON root.id = occurrence.root_id
     LEFT JOIN source_root_deletion AS deletion ON deletion.root_id = root.id
     WHERE document.is_deleted = 0 AND document.status = :document_status
       AND triage.status = :triage_status
     ORDER BY candidate_job.queued_at_seconds, candidate_job.job_rowid";

pub(super) fn claim_next_candidates_sql() -> String {
    format!(
        "WITH candidate_jobs AS MATERIALIZED ({CLAIM_NEXT_JOBS_SQL}) {CLAIM_CANDIDATES_BODY_SQL}"
    )
}

pub(super) fn claim_candidate_jobs_sql() -> &'static str {
    CLAIM_NEXT_JOBS_SQL
}

fn running_job_candidates_sql() -> String {
    format!("WITH candidate_jobs AS MATERIALIZED ({RUNNING_JOB_SQL}) {CLAIM_CANDIDATES_BODY_SQL}")
}

pub(super) fn validate_stored_phases(connection: &Connection) -> Result<()> {
    let mut statement = connection
        .prepare(
            "SELECT deletion.phase
             FROM ocr_claim_source_fence AS fence
             LEFT JOIN source_root_deletion AS deletion ON deletion.root_id = fence.root_id
             ORDER BY fence.ingest_job_id",
        )
        .map_err(MetaStoreError::storage)?;
    let phases = statement
        .query_map([], |row| row.get::<_, Option<String>>(0))
        .map_err(MetaStoreError::storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::storage)?;
    for phase in phases.into_iter().flatten() {
        SourceRootDeletionPhase::parse(&phase)?;
    }
    Ok(())
}

struct ClaimCandidate {
    job_id: IngestJobId,
    document_id: String,
    source_revision_id: String,
    triage_epoch: String,
    root_id: SourceRootId,
    relative_path: String,
    root_epoch: i64,
    canonical_path: String,
    normalized_path: String,
    deletion_phase: Option<SourceRootDeletionPhase>,
}

pub(super) fn claim_next(
    transaction: &Transaction<'_>,
    now: UnixTimestamp,
) -> Result<Option<IngestJobId>> {
    let mut candidate = None;
    for _ in 0..CLAIM_BATCH_LIMIT {
        let job_ids = claim_candidate_job_ids(transaction)?;
        if job_ids.is_empty() {
            break;
        }
        let candidates = claim_candidates(transaction, None)?;
        if let Some(current) = candidates.into_iter().find(candidate_is_current) {
            candidate = Some(current);
            break;
        }
        let discarded =
            super::discard_unclaimable_ocr_jobs_in_connection(transaction, &job_ids, now)?;
        if discarded != job_ids.len() {
            return Err(MetaStoreError::storage_invariant());
        }
    }
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let changed = transaction
        .execute(
            "UPDATE ingest_job
             SET status = ?1, attempt_count = attempt_count + 1,
                 started_at_seconds = ?2, finished_at_seconds = NULL,
                 updated_at_seconds = ?2, failure_kind = NULL
             WHERE id = ?3 AND document_id = ?4 AND kind = ?5
               AND (status = ?6 OR (status IN (?7, ?8) AND attempt_count < max_attempts))",
            params![
                ingest_job_status_to_storage(IngestJobStatus::Running),
                now.as_unix_seconds(),
                candidate.job_id.as_str(),
                candidate.document_id,
                ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                ingest_job_status_to_storage(IngestJobStatus::Queued),
                ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if changed != 1 {
        return Ok(None);
    }
    let attempt_count = transaction
        .query_row(
            "SELECT attempt_count FROM ingest_job WHERE id = ?1",
            [candidate.job_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let inserted = transaction
        .execute(
            "INSERT INTO ocr_claim_source_fence (
                ingest_job_id, attempt_count, document_id, source_revision_id,
                triage_epoch, root_id, relative_path, root_revocation_epoch
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(ingest_job_id) DO UPDATE SET
                attempt_count = excluded.attempt_count,
                document_id = excluded.document_id,
                source_revision_id = excluded.source_revision_id,
                triage_epoch = excluded.triage_epoch,
                root_id = excluded.root_id,
                relative_path = excluded.relative_path,
                root_revocation_epoch = excluded.root_revocation_epoch",
            params![
                candidate.job_id.as_str(),
                attempt_count,
                candidate.document_id,
                candidate.source_revision_id,
                candidate.triage_epoch,
                candidate.root_id.as_str(),
                candidate.relative_path,
                candidate.root_epoch,
            ],
        )
        .map_err(MetaStoreError::storage)?;
    if inserted != 1 {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(Some(candidate.job_id))
}

fn claim_candidate_job_ids(connection: &Connection) -> Result<Vec<IngestJobId>> {
    let mut statement = connection
        .prepare(claim_candidate_jobs_sql())
        .map_err(MetaStoreError::storage)?;
    let job_ids = statement
        .query_map(
            rusqlite::named_params! {":candidate_limit": CLAIM_CANDIDATE_LIMIT},
            |row| row.get::<_, String>(0),
        )
        .map_err(MetaStoreError::storage)?
        .map(|row| {
            IngestJobId::from_str(&row.map_err(MetaStoreError::storage)?)
                .map_err(|_| MetaStoreError::invalid_value("ingest_job.id"))
        })
        .collect();
    job_ids
}

pub(super) fn is_current(connection: &Connection, claimed: &ClaimedOcrJob) -> Result<bool> {
    Ok(read_binding(connection, claimed)?.is_some_and(|binding| {
        binding
            .deletion_phase
            .is_none_or(|phase| !phase.is_active())
            && binding.path_matches()
    }))
}

pub(super) fn activation_is_current(
    connection: &Connection,
    claimed: &ClaimedOcrJob,
) -> Result<bool> {
    Ok(
        read_activation_binding(connection, claimed)?.is_some_and(|binding| {
            binding
                .deletion_phase
                .is_none_or(|phase| !phase.is_active())
                && binding.path_matches()
        }),
    )
}

pub(super) fn settle_superseded(
    connection: &Connection,
    claimed: &ClaimedOcrJob,
    now: UnixTimestamp,
) -> Result<Option<OcrJobDiscardReason>> {
    if let Some(candidate) = replacement_candidate(connection, claimed)? {
        let changed = connection
            .execute(
                "UPDATE ingest_job
                 SET status = ?1, max_attempts = max_attempts + 1,
                     started_at_seconds = NULL, finished_at_seconds = NULL,
                     updated_at_seconds = ?2, failure_kind = NULL
                 WHERE id = ?3 AND status = ?4 AND attempt_count = ?5
                   AND max_attempts < 4294967295",
                params![
                    ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                    now.as_unix_seconds(),
                    claimed.job.id.as_str(),
                    ingest_job_status_to_storage(IngestJobStatus::Running),
                    u32_to_i64(claimed.job.attempt_count),
                ],
            )
            .map_err(MetaStoreError::storage)?;
        if changed != 1 {
            return Err(MetaStoreError::invalid_transition());
        }
        connection
            .execute(
                "DELETE FROM ocr_claim_source_fence WHERE ingest_job_id = ?1",
                [candidate.job_id.as_str()],
            )
            .map_err(MetaStoreError::storage)?;
        return Ok(None);
    }
    super::discard_ocr_claim_in_connection(connection, claimed, now)
}

fn claim_candidates(
    connection: &Connection,
    running_job: Option<&IngestJobId>,
) -> Result<Vec<ClaimCandidate>> {
    let sql = if running_job.is_some() {
        running_job_candidates_sql()
    } else {
        claim_next_candidates_sql()
    };
    let mut statement = connection.prepare(&sql).map_err(MetaStoreError::storage)?;
    let document_status = document_status_to_storage(DocumentStatus::OcrRequired);
    let triage_status = crate::ClassificationStatus::OcrBacklog.as_str();
    let max_root_epoch = schema_v38::MAX_ROOT_REVOCATION_EPOCH;
    let rows = match running_job {
        Some(running_job) => statement
            .query_map(
                rusqlite::named_params! {
                    ":running_job": running_job.as_str(),
                    ":document_status": document_status,
                    ":triage_status": triage_status,
                    ":max_root_epoch": max_root_epoch,
                },
                read_candidate,
            )
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?,
        None => statement
            .query_map(
                rusqlite::named_params! {
                    ":candidate_limit": CLAIM_CANDIDATE_LIMIT,
                    ":document_status": document_status,
                    ":triage_status": triage_status,
                    ":max_root_epoch": max_root_epoch,
                },
                read_candidate,
            )
            .map_err(MetaStoreError::storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(MetaStoreError::storage)?,
    };
    rows.into_iter().map(parse_candidate).collect()
}

fn replacement_candidate(
    connection: &Connection,
    claimed: &ClaimedOcrJob,
) -> Result<Option<ClaimCandidate>> {
    let candidates = claim_candidates(connection, Some(&claimed.job.id))?;
    Ok(candidates
        .into_iter()
        .find(|candidate| candidate.job_id == claimed.job.id && candidate_is_current(candidate)))
}

type CandidateRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
    Option<String>,
);

fn read_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<CandidateRow> {
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
    ))
}

fn parse_candidate(row: CandidateRow) -> Result<ClaimCandidate> {
    classification::validate_stored_source_triage_epoch(&row.3, "ocr_job_spec.triage_epoch")?;
    Ok(ClaimCandidate {
        job_id: IngestJobId::from_str(&row.0)
            .map_err(|_| MetaStoreError::invalid_value("ingest_job.id"))?,
        document_id: row.1,
        source_revision_id: row.2,
        triage_epoch: row.3,
        root_id: SourceRootId::from_str(&row.4)
            .map_err(|_| MetaStoreError::invalid_value("source_root.id"))?,
        relative_path: row.5,
        root_epoch: row.6,
        canonical_path: row.7,
        normalized_path: row.8,
        deletion_phase: row
            .9
            .map(|phase| SourceRootDeletionPhase::parse(&phase))
            .transpose()?,
    })
}

fn candidate_is_current(candidate: &ClaimCandidate) -> bool {
    candidate
        .deletion_phase
        .is_none_or(|phase| !phase.is_active())
        && candidate.path_matches()
}

struct PersistedBinding {
    canonical_path: String,
    relative_path: String,
    normalized_path: String,
    deletion_phase: Option<SourceRootDeletionPhase>,
}

impl PersistedBinding {
    fn path_matches(&self) -> bool {
        Path::new(&self.canonical_path).join(&self.relative_path)
            == Path::new(&self.normalized_path)
    }
}

impl ClaimCandidate {
    fn path_matches(&self) -> bool {
        Path::new(&self.canonical_path).join(&self.relative_path)
            == Path::new(&self.normalized_path)
    }
}

fn read_binding(
    connection: &Connection,
    claimed: &ClaimedOcrJob,
) -> Result<Option<PersistedBinding>> {
    read_binding_with_status(connection, claimed, IngestJobStatus::Running)
}

fn read_activation_binding(
    connection: &Connection,
    claimed: &ClaimedOcrJob,
) -> Result<Option<PersistedBinding>> {
    read_binding_with_status(connection, claimed, IngestJobStatus::Completed)
}

fn read_binding_with_status(
    connection: &Connection,
    claimed: &ClaimedOcrJob,
    status: IngestJobStatus,
) -> Result<Option<PersistedBinding>> {
    let row = connection
        .query_row(
            "SELECT root.canonical_path, fence.relative_path, document.normalized_path,
                    deletion.phase
             FROM ocr_claim_source_fence AS fence
             JOIN ingest_job AS job
               ON job.id = fence.ingest_job_id
              AND job.document_id = fence.document_id
              AND job.attempt_count = fence.attempt_count
             JOIN ocr_job_spec AS spec
               ON spec.ingest_job_id = job.id
              AND spec.source_revision_id = fence.source_revision_id
              AND spec.triage_epoch = fence.triage_epoch
             JOIN source_revision_triage AS triage
               ON triage.source_revision_id = spec.source_revision_id
              AND triage.triage_epoch = spec.triage_epoch
             JOIN source_revision AS revision
               ON revision.id = spec.source_revision_id
              AND revision.document_id = fence.document_id
             JOIN document
               ON document.id = job.document_id
              AND document.content_hash = revision.content_hash
             JOIN source_occurrence AS occurrence
               ON occurrence.root_id = fence.root_id
              AND occurrence.relative_path = fence.relative_path
              AND occurrence.document_id = fence.document_id
              AND occurrence.source_revision_id = fence.source_revision_id
              AND occurrence.state = 'present'
             JOIN source_root AS root
               ON root.id = fence.root_id
              AND typeof(root.revocation_epoch) = 'integer'
              AND root.revocation_epoch = fence.root_revocation_epoch
              AND root.revocation_epoch BETWEEN 0 AND ?13
             LEFT JOIN source_root_deletion AS deletion ON deletion.root_id = root.id
             WHERE job.id = ?1 AND job.document_id = ?2 AND job.kind = ?3
               AND job.status = ?4 AND job.attempt_count = ?5 AND job.max_attempts = ?6
               AND document.is_deleted = 0 AND (?7 = 0 OR document.status = ?8)
               AND document.content_hash = ?9 AND triage.status = ?10
               AND spec.source_revision_id = ?11 AND spec.triage_epoch = ?12",
            params![
                claimed.job.id.as_str(),
                claimed.job.document_id.as_str(),
                ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                ingest_job_status_to_storage(status),
                u32_to_i64(claimed.job.attempt_count),
                u32_to_i64(claimed.job.max_attempts),
                i64::from(status == IngestJobStatus::Running),
                document_status_to_storage(DocumentStatus::OcrRequired),
                claimed.source_fingerprint(),
                crate::ClassificationStatus::OcrBacklog.as_str(),
                claimed.source_revision_id().as_str(),
                claimed.triage_epoch(),
                schema_v38::MAX_ROOT_REVOCATION_EPOCH,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(MetaStoreError::storage)?;
    row.map(|row| {
        Ok(PersistedBinding {
            canonical_path: row.0,
            relative_path: row.1,
            normalized_path: row.2,
            deletion_phase: row
                .3
                .map(|phase| SourceRootDeletionPhase::parse(&phase))
                .transpose()?,
        })
    })
    .transpose()
}
