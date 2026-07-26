pub(super) const VERSION: u32 = 31;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE source_root (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(id) = 37
        AND substr(id, 1, 5) = 'root-'
        AND substr(id, 6) NOT GLOB '*[^0-9a-f]*'
    ),
    canonical_path TEXT NOT NULL UNIQUE CHECK (length(canonical_path) > 0),
    requested_path TEXT NOT NULL CHECK (length(requested_path) > 0),
    display_label TEXT NOT NULL CHECK (
        length(display_label) BETWEEN 1 AND 80
        AND instr(display_label, char(0)) = 0
    ),
    state TEXT NOT NULL CHECK (state IN ('active', 'offline')),
    watcher_state TEXT NOT NULL CHECK (
        watcher_state IN ('active', 'paused', 'unavailable')
    ),
    created_at_seconds INTEGER NOT NULL CHECK (created_at_seconds >= 0),
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= created_at_seconds)
);

CREATE INDEX source_root_state_idx
    ON source_root(state, updated_at_seconds);

CREATE TABLE source_occurrence (
    root_id TEXT NOT NULL,
    relative_path TEXT NOT NULL CHECK (
        length(relative_path) BETWEEN 1 AND 4096
        AND instr(relative_path, char(0)) = 0
        AND substr(relative_path, 1, 1) <> '/'
        AND relative_path <> '..'
        AND relative_path NOT LIKE '../%'
        AND relative_path NOT LIKE '%/../%'
        AND relative_path NOT LIKE '%/..'
    ),
    document_id TEXT NOT NULL,
    source_revision_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('present', 'removed')),
    first_seen_scan_id TEXT,
    last_seen_scan_id TEXT,
    observed_at_seconds INTEGER NOT NULL CHECK (observed_at_seconds >= 0),
    removed_at_seconds INTEGER CHECK (
        (state = 'present' AND removed_at_seconds IS NULL)
        OR (state = 'removed' AND removed_at_seconds IS NOT NULL)
    ),
    PRIMARY KEY (root_id, relative_path),
    FOREIGN KEY (root_id) REFERENCES source_root(id) ON DELETE CASCADE,
    FOREIGN KEY (document_id) REFERENCES document(id) ON DELETE CASCADE,
    FOREIGN KEY (source_revision_id, document_id)
        REFERENCES source_revision(id, document_id) ON DELETE CASCADE,
    FOREIGN KEY (first_seen_scan_id) REFERENCES scan_snapshot(id),
    FOREIGN KEY (last_seen_scan_id) REFERENCES scan_snapshot(id)
);

CREATE INDEX source_occurrence_document_idx
    ON source_occurrence(document_id, state);
CREATE INDEX source_occurrence_revision_idx
    ON source_occurrence(source_revision_id, state);

CREATE TABLE source_occurrence_revision (
    root_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    source_revision_id TEXT NOT NULL,
    observed_at_seconds INTEGER NOT NULL CHECK (observed_at_seconds >= 0),
    PRIMARY KEY (root_id, relative_path, source_revision_id),
    FOREIGN KEY (root_id, relative_path)
        REFERENCES source_occurrence(root_id, relative_path) ON DELETE CASCADE,
    FOREIGN KEY (source_revision_id) REFERENCES source_revision(id) ON DELETE CASCADE
);

CREATE TRIGGER source_occurrence_revision_immutable_update
BEFORE UPDATE ON source_occurrence_revision
BEGIN
    SELECT RAISE(ABORT, 'immutable source occurrence revision');
END;

CREATE TABLE scan_snapshot (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(id) BETWEEN 1 AND 80
        AND id NOT GLOB '*[^a-zA-Z0-9_-]*'
    ),
    root_id TEXT NOT NULL,
    trigger TEXT NOT NULL CHECK (
        trigger IN ('initial', 'manual', 'watcher', 'periodic', 'recovery')
    ),
    phase TEXT NOT NULL CHECK (
        phase IN (
            'queued', 'discovering', 'fingerprinting', 'classifying',
            'parsing', 'ocr', 'publishing', 'complete', 'partial', 'failed'
        )
    ),
    completeness TEXT NOT NULL CHECK (
        completeness IN ('unknown', 'complete', 'partial')
    ),
    discovered_count INTEGER NOT NULL DEFAULT 0 CHECK (discovered_count >= 0),
    searchable_count INTEGER NOT NULL DEFAULT 0 CHECK (searchable_count >= 0),
    non_resume_count INTEGER NOT NULL DEFAULT 0 CHECK (non_resume_count >= 0),
    needs_review_count INTEGER NOT NULL DEFAULT 0 CHECK (needs_review_count >= 0),
    ocr_count INTEGER NOT NULL DEFAULT 0 CHECK (ocr_count >= 0),
    failed_count INTEGER NOT NULL DEFAULT 0 CHECK (failed_count >= 0),
    ignored_count INTEGER NOT NULL DEFAULT 0 CHECK (ignored_count >= 0),
    processed_count INTEGER NOT NULL DEFAULT 0 CHECK (processed_count >= 0),
    total_count INTEGER CHECK (total_count IS NULL OR total_count >= processed_count),
    rate_per_second REAL CHECK (rate_per_second IS NULL OR rate_per_second > 0),
    eta_seconds INTEGER CHECK (eta_seconds IS NULL OR eta_seconds >= 0),
    error_count INTEGER NOT NULL DEFAULT 0 CHECK (error_count >= 0),
    started_at_seconds INTEGER NOT NULL CHECK (started_at_seconds >= 0),
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= started_at_seconds),
    completed_at_seconds INTEGER CHECK (
        completed_at_seconds IS NULL OR completed_at_seconds >= started_at_seconds
    ),
    FOREIGN KEY (root_id) REFERENCES source_root(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX scan_snapshot_one_active_root_idx
    ON scan_snapshot(root_id)
    WHERE phase IN (
        'queued', 'discovering', 'fingerprinting', 'classifying',
        'parsing', 'ocr', 'publishing'
    );
CREATE INDEX scan_snapshot_root_history_idx
    ON scan_snapshot(root_id, started_at_seconds DESC);
"#;
