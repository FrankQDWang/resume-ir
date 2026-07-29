pub(super) const VERSION: u32 = 35;

/// Persisted source-occurrence observations for the correctness-gated macOS
/// metadata fast path. These rows are only written after a successful strong
/// content verification and are removed with their occurrence.
pub(super) const SCHEMA: &str = r#"
CREATE TABLE source_file_observation (
    root_id TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    source_revision_id TEXT NOT NULL,
    assurance_kind TEXT NOT NULL CHECK (assurance_kind = 'macos_stat_v1'),
    stable_file_id TEXT NOT NULL CHECK (
        length(stable_file_id) = 36
        AND substr(stable_file_id, 1, 4) = 'sfi_'
        AND substr(stable_file_id, 5) NOT GLOB '*[^0-9a-f]*'
    ),
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    mtime_seconds INTEGER NOT NULL,
    mtime_nanoseconds INTEGER NOT NULL CHECK (
        mtime_nanoseconds BETWEEN 0 AND 999999999
    ),
    ctime_seconds INTEGER NOT NULL,
    ctime_nanoseconds INTEGER NOT NULL CHECK (
        ctime_nanoseconds BETWEEN 0 AND 999999999
    ),
    strongly_verified_at_seconds INTEGER NOT NULL CHECK (
        strongly_verified_at_seconds >= 0
    ),
    next_strong_verification_at_seconds INTEGER NOT NULL CHECK (
        next_strong_verification_at_seconds > strongly_verified_at_seconds
    ),
    PRIMARY KEY (root_id, relative_path),
    FOREIGN KEY (root_id, relative_path)
        REFERENCES source_occurrence(root_id, relative_path) ON DELETE CASCADE,
    FOREIGN KEY (source_revision_id)
        REFERENCES source_revision(id) ON DELETE CASCADE
);

CREATE INDEX source_file_observation_audit_idx
    ON source_file_observation(next_strong_verification_at_seconds);
"#;
