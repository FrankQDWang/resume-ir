pub(super) const VERSION: u32 = 33;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE pdf_reprocess_job (
    source_revision_id TEXT PRIMARY KEY NOT NULL,
    root_id TEXT NOT NULL,
    relative_path TEXT NOT NULL CHECK (
        length(relative_path) > 0
        AND length(relative_path) <= 4096
        AND substr(relative_path, 1, 1) != '/'
        AND relative_path NOT LIKE '../%'
        AND relative_path NOT LIKE '%/../%'
    ),
    parser_contract TEXT NOT NULL CHECK (
        length(parser_contract) > 0 AND length(parser_contract) <= 64
    ),
    state TEXT NOT NULL CHECK (
        state IN ('queued', 'scheduled', 'complete', 'cancelled')
    ),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0 AND attempts <= 16),
    scheduled_task_id TEXT,
    queued_at_seconds INTEGER NOT NULL CHECK (queued_at_seconds >= 0),
    updated_at_seconds INTEGER NOT NULL CHECK (
        updated_at_seconds >= queued_at_seconds
    ),
    completed_at_seconds INTEGER CHECK (
        completed_at_seconds IS NULL
        OR completed_at_seconds >= queued_at_seconds
    ),
    FOREIGN KEY (source_revision_id)
        REFERENCES source_revision(id) ON DELETE CASCADE,
    FOREIGN KEY (root_id, relative_path)
        REFERENCES source_occurrence(root_id, relative_path) ON DELETE CASCADE
);

CREATE INDEX pdf_reprocess_job_claim_idx
    ON pdf_reprocess_job(state, attempts, queued_at_seconds, root_id);
"#;
