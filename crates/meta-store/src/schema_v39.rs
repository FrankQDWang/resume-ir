pub(super) const VERSION: u32 = 39;

/// Persist one current root-generation fence for each claimed OCR job.
///
/// The row is owned by the exact job and source occurrence. Deleting either
/// authority cascades the fence so source-root deletion is never blocked and
/// an orphan fence cannot survive cleanup.
pub(super) const SCHEMA: &str = r#"
CREATE TABLE ocr_claim_source_fence (
    ingest_job_id TEXT PRIMARY KEY NOT NULL,
    attempt_count INTEGER NOT NULL CHECK (
        typeof(attempt_count) = 'integer' AND attempt_count BETWEEN 1 AND 4294967295
    ),
    document_id TEXT NOT NULL,
    source_revision_id TEXT NOT NULL,
    triage_epoch TEXT NOT NULL,
    root_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    root_revocation_epoch INTEGER NOT NULL CHECK (
        typeof(root_revocation_epoch) = 'integer'
        AND root_revocation_epoch BETWEEN 0 AND 9007199254740991
    ),
    FOREIGN KEY (ingest_job_id) REFERENCES ingest_job(id) ON DELETE CASCADE,
    FOREIGN KEY (source_revision_id, triage_epoch)
        REFERENCES source_revision_triage(source_revision_id, triage_epoch)
        ON DELETE CASCADE,
    FOREIGN KEY (source_revision_id, document_id)
        REFERENCES source_revision(id, document_id) ON DELETE CASCADE,
    FOREIGN KEY (root_id, relative_path)
        REFERENCES source_occurrence(root_id, relative_path) ON DELETE CASCADE
);

CREATE INDEX ocr_claim_source_fence_revision_idx
    ON ocr_claim_source_fence(source_revision_id, root_id, relative_path);

CREATE INDEX ingest_job_ocr_claim_queue_idx
    ON ingest_job(queued_at_seconds)
    WHERE kind = 'ocr_document'
      AND (
        status = 'queued'
        OR (
          status IN ('interrupted', 'failed_retryable')
          AND attempt_count < max_attempts
        )
      );
"#;
