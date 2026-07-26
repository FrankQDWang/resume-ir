pub(super) const VERSION: u32 = 32;

pub(super) const SCHEMA: &str = r#"
CREATE TABLE source_root_deletion (
    root_id TEXT PRIMARY KEY NOT NULL CHECK (
        length(root_id) = 37
        AND substr(root_id, 1, 5) = 'root-'
        AND substr(root_id, 6) NOT GLOB '*[^0-9a-f]*'
    ),
    canonical_path TEXT NOT NULL CHECK (
        length(canonical_path) BETWEEN 1 AND 131072
        AND instr(canonical_path, char(0)) = 0
    ),
    phase TEXT NOT NULL CHECK (
        phase IN (
            'requested', 'quiescing', 'publishing', 'purging',
            'verifying', 'complete', 'failed'
        )
    ),
    affected_documents INTEGER NOT NULL DEFAULT 0
        CHECK (affected_documents >= 0),
    removed_documents INTEGER NOT NULL DEFAULT 0
        CHECK (removed_documents >= 0),
    started_at_seconds INTEGER NOT NULL CHECK (started_at_seconds >= 0),
    updated_at_seconds INTEGER NOT NULL CHECK (
        updated_at_seconds >= started_at_seconds
    ),
    completed_at_seconds INTEGER CHECK (
        completed_at_seconds IS NULL
        OR completed_at_seconds >= started_at_seconds
    )
);

CREATE INDEX source_root_deletion_phase_idx
    ON source_root_deletion(phase, updated_at_seconds);

CREATE TABLE source_root_deletion_document (
    root_id TEXT NOT NULL,
    document_id TEXT NOT NULL CHECK (
        length(document_id) = 36
        AND substr(document_id, 1, 4) = 'doc_'
        AND substr(document_id, 5) NOT GLOB '*[^0-9a-f]*'
    ),
    content_hash TEXT NOT NULL CHECK (
        length(content_hash) = 71
        AND substr(content_hash, 1, 7) = 'sha256:'
        AND substr(content_hash, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    PRIMARY KEY (root_id, document_id, content_hash),
    FOREIGN KEY (root_id) REFERENCES source_root_deletion(root_id)
        ON DELETE CASCADE
);

CREATE INDEX source_root_deletion_document_hash_idx
    ON source_root_deletion_document(content_hash);
"#;
