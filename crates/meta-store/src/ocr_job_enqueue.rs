use std::str::FromStr;

use rusqlite::{params, Connection, OptionalExtension};

use crate::{
    document_status_to_storage, i64_to_u32, ingest_job_kind_to_storage,
    ingest_job_status_to_storage, u32_to_i64, DocumentId, DocumentStatus, IngestJobId,
    IngestJobKind, IngestJobStatus, MetaStoreError, Result, SourceRevisionId, UnixTimestamp,
};

pub(super) fn enqueue_ocr_job_for_source_triage_in_connection(
    connection: &Connection,
    source_revision_id: &SourceRevisionId,
    triage_epoch: &str,
    queued_at: UnixTimestamp,
) -> Result<(IngestJobId, bool)> {
    let document_id = connection
        .query_row(
            "SELECT revision.document_id
             FROM source_revision_triage AS triage
             JOIN source_revision AS revision
               ON revision.id = triage.source_revision_id
             JOIN document
               ON document.id = revision.document_id
              AND document.content_hash = revision.content_hash
             WHERE triage.source_revision_id = ?1 AND triage.triage_epoch = ?2
               AND triage.status = 'ocr_backlog'
               AND document.is_deleted = 0 AND document.status = ?3",
            params![
                source_revision_id.as_str(),
                triage_epoch,
                document_status_to_storage(DocumentStatus::OcrRequired),
            ],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(MetaStoreError::storage)?
        .ok_or_else(|| MetaStoreError::not_found("source_revision_triage"))?;
    let document_id = DocumentId::from_str(&document_id)
        .map_err(|_| MetaStoreError::invalid_value("source_revision.document_id"))?;
    let job_id = IngestJobId::from_non_secret_parts(&[
        "ocr-source-triage",
        source_revision_id.as_str(),
        triage_epoch,
    ]);
    let existing = {
        let mut statement = connection
            .prepare(
                "SELECT attempt_count
                 FROM ingest_job AS job
                 JOIN ocr_job_spec AS spec ON spec.ingest_job_id = job.id
                 WHERE job.id = ?1 AND job.kind = ?2
                   AND spec.source_revision_id = ?3 AND spec.triage_epoch = ?4",
            )
            .map_err(MetaStoreError::storage)?;
        let mut rows = statement
            .query(params![
                job_id.as_str(),
                ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                source_revision_id.as_str(),
                triage_epoch,
            ])
            .map_err(MetaStoreError::storage)?;
        match rows.next().map_err(MetaStoreError::storage)? {
            Some(row) => Some(i64_to_u32(
                row.get(0).map_err(MetaStoreError::storage)?,
                "ingest_job.attempt_count",
            )?),
            None => None,
        }
    };

    let scheduled = if let Some(attempt_count) = existing {
        let renewed_max_attempts = attempt_count
            .checked_add(3)
            .ok_or_else(|| MetaStoreError::invalid_value("ingest_job.max_attempts"))?;
        let renewed = connection
            .execute(
                "UPDATE ingest_job
                 SET status = ?1, max_attempts = ?2,
                     queued_at_seconds = ?3, started_at_seconds = NULL,
                     finished_at_seconds = NULL, updated_at_seconds = ?3,
                     failure_kind = NULL
                 WHERE id = ?4 AND document_id = ?5 AND kind = ?6 AND (
                     status IN (?7, ?8)
                     OR (status IN (?9, ?10) AND attempt_count >= max_attempts)
                 ) AND EXISTS (
                     SELECT 1 FROM ocr_job_spec AS spec
                     JOIN source_revision_triage AS triage
                       ON triage.source_revision_id = spec.source_revision_id
                      AND triage.triage_epoch = spec.triage_epoch
                     JOIN source_revision AS revision ON revision.id = spec.source_revision_id
                     JOIN document
                       ON document.id = revision.document_id
                      AND document.content_hash = revision.content_hash
                     WHERE spec.ingest_job_id = ingest_job.id
                       AND spec.source_revision_id = ?11 AND spec.triage_epoch = ?12
                       AND triage.status = 'ocr_backlog'
                       AND document.is_deleted = 0 AND document.status = ?13
                 )",
                params![
                    ingest_job_status_to_storage(IngestJobStatus::Queued),
                    u32_to_i64(renewed_max_attempts),
                    queued_at.as_unix_seconds(),
                    job_id.as_str(),
                    document_id.as_str(),
                    ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                    ingest_job_status_to_storage(IngestJobStatus::Completed),
                    ingest_job_status_to_storage(IngestJobStatus::FailedPermanent),
                    ingest_job_status_to_storage(IngestJobStatus::FailedRetryable),
                    ingest_job_status_to_storage(IngestJobStatus::Interrupted),
                    source_revision_id.as_str(),
                    triage_epoch,
                    document_status_to_storage(DocumentStatus::OcrRequired),
                ],
            )
            .map_err(MetaStoreError::storage)?
            == 1;
        if renewed {
            // The renewal predicate above proved the revision/triage pair is
            // current again, so a discard tombstone recorded while the pair
            // was invalid no longer describes this job. It must be removed,
            // or claim-side stale settlement would immediately re-complete
            // the renewed attempt and the OCR work would never run.
            connection
                .execute(
                    "DELETE FROM ocr_job_discard WHERE ingest_job_id = ?1",
                    params![job_id.as_str()],
                )
                .map_err(MetaStoreError::storage)?;
        }
        renewed
    } else {
        connection
            .execute(
                "INSERT INTO ocr_job_spec (
                    ingest_job_id, source_revision_id, triage_epoch
                 ) VALUES (?1, ?2, ?3)",
                params![job_id.as_str(), source_revision_id.as_str(), triage_epoch],
            )
            .map_err(MetaStoreError::storage)?;
        connection
            .execute(
                "INSERT INTO ingest_job (
                    id, document_id, resume_version_id, kind, status, attempt_count,
                    max_attempts, queued_at_seconds, started_at_seconds,
                    finished_at_seconds, updated_at_seconds, failure_kind
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    job_id.as_str(),
                    document_id.as_str(),
                    Option::<&str>::None,
                    ingest_job_kind_to_storage(IngestJobKind::OcrDocument),
                    ingest_job_status_to_storage(IngestJobStatus::Queued),
                    0_i64,
                    3_i64,
                    queued_at.as_unix_seconds(),
                    Option::<i64>::None,
                    Option::<i64>::None,
                    queued_at.as_unix_seconds(),
                    Option::<&str>::None,
                ],
            )
            .map_err(MetaStoreError::storage)?;
        true
    };
    Ok((job_id, scheduled))
}
