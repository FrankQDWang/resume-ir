pub(super) const VERSION: u32 = 36;

/// One bounded, durable observation row per source-root deletion receipt.
///
/// This table records worker attempts only. It does not own retry policy,
/// deletion phases, recovery decisions, or residual cleanup.
pub(super) const SCHEMA: &str = r#"
CREATE TABLE source_root_deletion_attempt_evidence (
    root_id TEXT PRIMARY KEY NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (
        attempt_count BETWEEN 0 AND 9007199254740991
    ),
    last_attempt_at_seconds INTEGER CHECK (last_attempt_at_seconds >= 0),
    last_error_phase TEXT CHECK (
        last_error_phase IS NULL
        OR last_error_phase IN (
            'requested', 'quiescing', 'publishing', 'purging', 'verifying'
        )
    ),
    last_error_code TEXT CHECK (
        last_error_code IS NULL
        OR last_error_code IN (
            'import_quiescence_timeout', 'ocr_quiescence_timeout',
            'publication_failed',
            'metadata_purge_failed', 'privacy_cleanup_failed',
            'receipt_completion_failed', 'internal'
        )
    ),
    last_error_at_seconds INTEGER CHECK (last_error_at_seconds >= 0),
    CHECK (
        (last_error_phase IS NULL
         AND last_error_code IS NULL
         AND last_error_at_seconds IS NULL)
        OR
        (last_error_phase IS NOT NULL
         AND last_error_code IS NOT NULL
         AND last_error_at_seconds IS NOT NULL)
    ),
    CHECK (
        (attempt_count = 0 AND last_attempt_at_seconds IS NULL)
        OR (attempt_count > 0 AND last_attempt_at_seconds IS NOT NULL)
    ),
    FOREIGN KEY (root_id) REFERENCES source_root_deletion(root_id)
        ON DELETE CASCADE
);

INSERT INTO source_root_deletion_attempt_evidence (root_id)
SELECT root_id FROM source_root_deletion;
"#;
