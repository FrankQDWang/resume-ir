macro_rules! retirement_trigger_schema {
    () => {
        r#"
CREATE TRIGGER search_publication_retirement_expectation_immutable
BEFORE UPDATE ON search_publication_retirement
WHEN NEW.generation IS NOT OLD.generation
  OR NEW.fulltext_expectation IS NOT OLD.fulltext_expectation
  OR NEW.vector_expectation IS NOT OLD.vector_expectation
  OR NEW.created_at_seconds IS NOT OLD.created_at_seconds
  OR NEW.fulltext_complete < OLD.fulltext_complete
  OR NEW.vector_complete < OLD.vector_complete
  OR (OLD.phase = 'complete' AND NEW.phase <> 'complete')
BEGIN
    SELECT RAISE(ABORT, 'search publication retirement is immutable');
END;

CREATE TRIGGER search_publication_retirement_completion_guard_authority
BEFORE INSERT ON search_publication_retirement_completion_guard
WHEN NOT EXISTS (
    SELECT 1
    FROM search_publication_retirement AS retirement
    JOIN search_publication_journal AS publication
      ON publication.generation = retirement.generation
    WHERE retirement.generation = NEW.generation
      AND retirement.phase = 'pending'
      AND publication.state = 'abandoned'
      AND (
          (NEW.artifact = 'fulltext' AND retirement.fulltext_complete = 0)
          OR (NEW.artifact = 'vector' AND retirement.vector_complete = 0)
      )
)
BEGIN
    SELECT RAISE(ABORT, 'search publication retirement guard lacks authority');
END;

CREATE TRIGGER search_publication_retirement_completion_guard_immutable
BEFORE UPDATE ON search_publication_retirement_completion_guard
BEGIN
    SELECT RAISE(ABORT, 'search publication retirement guard is immutable');
END;

CREATE TRIGGER search_publication_retirement_completion_requires_guard
BEFORE UPDATE ON search_publication_retirement
WHEN NEW.phase IS NOT OLD.phase
  OR NEW.fulltext_complete IS NOT OLD.fulltext_complete
  OR NEW.vector_complete IS NOT OLD.vector_complete
  OR NEW.updated_at_seconds IS NOT OLD.updated_at_seconds
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1
        FROM search_publication_retirement_completion_guard AS completion_guard
        WHERE completion_guard.generation = OLD.generation
          AND OLD.phase = 'pending'
          AND NEW.updated_at_seconds = MAX(
              OLD.updated_at_seconds,
              completion_guard.completed_at_seconds
          )
          AND (
              (
                  completion_guard.artifact = 'fulltext'
                  AND OLD.fulltext_complete = 0
                  AND NEW.fulltext_complete = 1
                  AND NEW.vector_complete = OLD.vector_complete
              )
              OR (
                  completion_guard.artifact = 'vector'
                  AND OLD.vector_complete = 0
                  AND NEW.vector_complete = 1
                  AND NEW.fulltext_complete = OLD.fulltext_complete
              )
          )
          AND (
              (NEW.fulltext_complete = 1 AND NEW.vector_complete = 1
               AND NEW.phase = 'complete')
              OR
              ((NEW.fulltext_complete = 0 OR NEW.vector_complete = 0)
               AND NEW.phase = 'pending')
          )
    ) THEN RAISE(ABORT, 'search publication retirement completion requires guard') END;
END;

CREATE TRIGGER search_publication_retirement_completion_guard_release
BEFORE DELETE ON search_publication_retirement_completion_guard
WHEN NOT EXISTS (
    SELECT 1 FROM search_publication_retirement AS retirement
    WHERE retirement.generation = OLD.generation
      AND (
          (OLD.artifact = 'fulltext' AND retirement.fulltext_complete = 1)
          OR (OLD.artifact = 'vector' AND retirement.vector_complete = 1)
      )
)
BEGIN
    SELECT RAISE(ABORT, 'search publication retirement guard released before completion');
END;

CREATE TRIGGER search_publication_retirement_insert_once
BEFORE INSERT ON search_publication_retirement
WHEN EXISTS (
    SELECT 1 FROM search_publication_retirement AS retirement
    WHERE retirement.generation = NEW.generation
)
BEGIN
    SELECT RAISE(ABORT, 'search publication retirement cannot be replaced');
END;

CREATE TRIGGER pending_search_publication_retirement_cannot_be_deleted
BEFORE DELETE ON search_publication_retirement
WHEN OLD.phase = 'pending'
BEGIN
    SELECT RAISE(ABORT, 'pending publication retirement cannot be deleted');
END;

CREATE TRIGGER search_publication_abandon_requires_retirement
BEFORE UPDATE OF state ON search_publication_journal
WHEN NEW.state = 'abandoned' AND NOT EXISTS (
    SELECT 1 FROM search_publication_retirement AS retirement
    WHERE retirement.generation = NEW.generation
)
BEGIN
    SELECT RAISE(ABORT, 'abandoned publication requires retirement intent');
END;

CREATE TRIGGER pending_search_publication_cannot_be_deleted
BEFORE DELETE ON search_publication_journal
WHEN EXISTS (
    SELECT 1 FROM search_publication_retirement AS retirement
    WHERE retirement.generation = OLD.generation AND retirement.phase = 'pending'
)
BEGIN
    SELECT RAISE(ABORT, 'pending publication retirement cannot be pruned');
END;
"#
    };
}

const RETIREMENT_TRIGGER_SCHEMA: &str = retirement_trigger_schema!();

/// v29 durable authority and exact-retirement state. The migration attempt
/// table is rebuilt because SQLite cannot widen its phase CHECK in place.
pub(super) const SCHEMA: &str = concat!(
    r#"
DROP TRIGGER migration_rebuild_contract_change_clears_publication_attempt;
DROP TRIGGER migration_rebuild_ready_clears_publication_attempt;

ALTER TABLE migration_rebuild_publication_attempt
    RENAME TO migration_rebuild_publication_attempt_v28;

CREATE TABLE migration_rebuild_publication_attempt (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    processing_contract_id TEXT NOT NULL,
    barrier_digest TEXT NOT NULL CHECK (
        length(barrier_digest) = 71 AND substr(barrier_digest, 1, 7) = 'sha256:'
    ),
    attempt_id TEXT NOT NULL CHECK (
        length(attempt_id) = 71 AND substr(attempt_id, 1, 7) = 'sha256:'
    ),
    attempt_count INTEGER NOT NULL CHECK (attempt_count BETWEEN 1 AND 5),
    phase TEXT NOT NULL CHECK (phase IN ('running', 'retry_wait', 'terminal')),
    started_at_seconds INTEGER NOT NULL CHECK (started_at_seconds >= 0),
    next_retry_at_seconds INTEGER CHECK (
        next_retry_at_seconds IS NULL OR next_retry_at_seconds >= 0
    ),
    last_error_class TEXT CHECK (
        last_error_class IS NULL OR last_error_class IN (
            'fulltext', 'vector', 'metadata', 'cleanup', 'interrupted'
        )
    ),
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= 0),
    CHECK (
        (phase = 'running' AND next_retry_at_seconds IS NULL
            AND last_error_class IS NULL)
        OR
        (phase = 'retry_wait' AND next_retry_at_seconds IS NOT NULL
            AND last_error_class IS NOT NULL)
        OR
        (phase = 'terminal' AND next_retry_at_seconds IS NULL
            AND last_error_class IS NOT NULL)
    ),
    FOREIGN KEY (processing_contract_id)
        REFERENCES import_processing_contract(id) ON DELETE RESTRICT
);

INSERT INTO migration_rebuild_publication_attempt (
    state_key, processing_contract_id, barrier_digest, attempt_id,
    attempt_count, phase, started_at_seconds, next_retry_at_seconds,
    last_error_class, updated_at_seconds
)
SELECT state_key, processing_contract_id, barrier_digest, attempt_id,
       attempt_count, phase, started_at_seconds, next_retry_at_seconds,
       last_error_class, updated_at_seconds
FROM migration_rebuild_publication_attempt_v28;

DROP TABLE migration_rebuild_publication_attempt_v28;

CREATE TRIGGER migration_rebuild_contract_change_clears_publication_attempt
AFTER UPDATE OF active_contract_id ON migration_rebuild_contract_state
WHEN NEW.active_contract_id IS NOT OLD.active_contract_id
BEGIN
    DELETE FROM migration_rebuild_publication_attempt WHERE state_key = 'default';
END;

CREATE TRIGGER migration_rebuild_ready_clears_publication_attempt
AFTER UPDATE OF service_state ON search_projection_state
WHEN NEW.service_state = 'ready'
BEGIN
    DELETE FROM migration_rebuild_publication_attempt WHERE state_key = 'default';
END;

ALTER TABLE search_publication_journal ADD COLUMN authority_kind TEXT;
ALTER TABLE search_publication_journal ADD COLUMN authority_contract_id TEXT;
ALTER TABLE search_publication_journal ADD COLUMN authority_barrier_digest TEXT;
ALTER TABLE search_publication_journal ADD COLUMN authority_repair_generation TEXT;
ALTER TABLE search_publication_journal ADD COLUMN authority_repair_fingerprint TEXT;
ALTER TABLE search_publication_journal ADD COLUMN authority_repair_visible_epoch INTEGER;
ALTER TABLE search_publication_journal ADD COLUMN authority_attempt_id TEXT;
ALTER TABLE search_publication_journal ADD COLUMN authority_attempt_count INTEGER;

UPDATE search_publication_journal SET authority_kind = 'current_head';

UPDATE search_publication_journal
SET authority_kind = 'migration_rebuild',
    authority_contract_id = (
        SELECT attempt.processing_contract_id
        FROM migration_rebuild_publication_attempt AS attempt
        WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
    ),
    authority_barrier_digest = (
        SELECT attempt.barrier_digest
        FROM migration_rebuild_publication_attempt AS attempt
        WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
    ),
    authority_attempt_id = (
        SELECT attempt.attempt_id
        FROM migration_rebuild_publication_attempt AS attempt
        WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
    ),
    authority_attempt_count = (
        SELECT attempt.attempt_count
        FROM migration_rebuild_publication_attempt AS attempt
        WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
    )
WHERE base_generation IS NULL
  AND EXISTS (
      SELECT 1
      FROM search_projection_state AS head
      JOIN migration_rebuild_publication_attempt AS attempt
        ON attempt.state_key = head.state_key
      WHERE head.state_key = 'default' AND head.generation IS NULL
        AND head.visible_epoch = search_publication_journal.expected_visible_epoch
        AND head.service_state = 'repairing'
        AND head.repair_reason = 'migration_rebuild'
        AND attempt.phase = 'running'
  );

UPDATE search_publication_journal
SET authority_kind = 'artifact_repair',
    authority_repair_generation = (
        SELECT attempt.generation FROM artifact_repair_attempt AS attempt
        WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
    ),
    authority_repair_fingerprint = (
        SELECT attempt.publication_fingerprint FROM artifact_repair_attempt AS attempt
        WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
    ),
    authority_repair_visible_epoch = (
        SELECT attempt.visible_epoch FROM artifact_repair_attempt AS attempt
        WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
    ),
    authority_attempt_id = (
        SELECT attempt.attempt_id FROM artifact_repair_attempt AS attempt
        WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
    ),
    authority_attempt_count = (
        SELECT attempt.attempt_count FROM artifact_repair_attempt AS attempt
        WHERE attempt.state_key = 'default' AND attempt.phase = 'running'
    )
WHERE EXISTS (
    SELECT 1
    FROM search_projection_state AS head
    JOIN artifact_repair_attempt AS attempt ON attempt.state_key = head.state_key
    WHERE head.state_key = 'default'
      AND head.generation = search_publication_journal.base_generation
      AND head.visible_epoch = search_publication_journal.expected_visible_epoch
      AND head.service_state = 'repairing'
      AND head.repair_reason = 'artifact_unavailable'
      AND attempt.generation = head.generation
      AND attempt.visible_epoch = head.visible_epoch
      AND attempt.phase = 'running'
);

CREATE TRIGGER search_publication_authority_insert
BEFORE INSERT ON search_publication_journal
WHEN NOT (
    (NEW.authority_kind = 'current_head'
     AND NEW.authority_contract_id IS NULL
     AND NEW.authority_barrier_digest IS NULL
     AND NEW.authority_repair_generation IS NULL
     AND NEW.authority_repair_fingerprint IS NULL
     AND NEW.authority_repair_visible_epoch IS NULL
     AND NEW.authority_attempt_id IS NULL
     AND NEW.authority_attempt_count IS NULL)
    OR
    (NEW.authority_kind = 'migration_rebuild'
     AND NEW.authority_contract_id IS NOT NULL
     AND NEW.authority_barrier_digest IS NOT NULL
     AND NEW.authority_repair_generation IS NULL
     AND NEW.authority_repair_fingerprint IS NULL
     AND NEW.authority_repair_visible_epoch IS NULL
     AND NEW.authority_attempt_id IS NOT NULL
     AND NEW.authority_attempt_count BETWEEN 1 AND 5)
    OR
    (NEW.authority_kind = 'artifact_repair'
     AND NEW.authority_contract_id IS NULL
     AND NEW.authority_barrier_digest IS NULL
     AND NEW.authority_repair_generation IS NOT NULL
     AND NEW.authority_repair_fingerprint IS NOT NULL
     AND NEW.authority_repair_visible_epoch >= 0
     AND NEW.authority_attempt_id IS NOT NULL
     AND NEW.authority_attempt_count BETWEEN 1 AND 5)
)
BEGIN
    SELECT RAISE(ABORT, 'invalid search publication authority');
END;

CREATE TRIGGER search_publication_authority_immutable
BEFORE UPDATE ON search_publication_journal
WHEN NEW.authority_kind IS NOT OLD.authority_kind
  OR NEW.authority_contract_id IS NOT OLD.authority_contract_id
  OR NEW.authority_barrier_digest IS NOT OLD.authority_barrier_digest
  OR NEW.authority_repair_generation IS NOT OLD.authority_repair_generation
  OR NEW.authority_repair_fingerprint IS NOT OLD.authority_repair_fingerprint
  OR NEW.authority_repair_visible_epoch IS NOT OLD.authority_repair_visible_epoch
  OR NEW.authority_attempt_id IS NOT OLD.authority_attempt_id
  OR NEW.authority_attempt_count IS NOT OLD.authority_attempt_count
BEGIN
    SELECT RAISE(ABORT, 'search publication authority is immutable');
END;

CREATE TABLE search_publication_retirement (
    generation TEXT PRIMARY KEY NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN ('pending', 'complete')),
    fulltext_expectation TEXT NOT NULL CHECK (
        fulltext_expectation IN ('none', 'may_exist', 'published')
    ),
    vector_expectation TEXT NOT NULL CHECK (
        vector_expectation IN ('none', 'may_exist', 'published')
    ),
    fulltext_complete INTEGER NOT NULL CHECK (fulltext_complete IN (0, 1)),
    vector_complete INTEGER NOT NULL CHECK (vector_complete IN (0, 1)),
    created_at_seconds INTEGER NOT NULL CHECK (created_at_seconds >= 0),
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= 0),
    FOREIGN KEY (generation)
        REFERENCES search_publication_journal(generation) ON DELETE CASCADE,
    CHECK (fulltext_expectation <> 'none' OR fulltext_complete = 1),
    CHECK (vector_expectation <> 'none' OR vector_complete = 1),
    CHECK (phase <> 'complete' OR (fulltext_complete = 1 AND vector_complete = 1)),
    CHECK (phase <> 'pending' OR fulltext_complete = 0 OR vector_complete = 0)
);

CREATE INDEX search_publication_retirement_pending_idx
    ON search_publication_retirement(phase, updated_at_seconds, generation);

INSERT INTO search_publication_retirement (
    generation, phase, fulltext_expectation, vector_expectation,
    fulltext_complete, vector_complete, created_at_seconds, updated_at_seconds
)
SELECT generation,
       CASE WHEN state IN ('preparing', 'validated') THEN 'pending' ELSE 'complete' END,
       CASE WHEN state IN ('preparing', 'validated') THEN 'may_exist' ELSE 'none' END,
       CASE WHEN state IN ('preparing', 'validated') THEN 'may_exist' ELSE 'none' END,
       CASE WHEN state IN ('preparing', 'validated') THEN 0 ELSE 1 END,
       CASE WHEN state IN ('preparing', 'validated') THEN 0 ELSE 1 END,
       created_at_seconds, updated_at_seconds
FROM search_publication_journal;

UPDATE search_publication_journal
SET state = 'abandoned'
WHERE state IN ('preparing', 'validated');

CREATE TABLE search_publication_retirement_completion_guard (
    generation TEXT PRIMARY KEY NOT NULL,
    artifact TEXT NOT NULL CHECK (artifact IN ('fulltext', 'vector')),
    completed_at_seconds INTEGER NOT NULL CHECK (completed_at_seconds >= 0),
    FOREIGN KEY (generation)
        REFERENCES search_publication_retirement(generation) ON DELETE RESTRICT
);
"#,
    retirement_trigger_schema!()
);

use rusqlite::{Connection, OptionalExtension};

use crate::{MetaStoreError, Result};

const AUTHORITY_COLUMNS: [&str; 8] = [
    "authority_kind",
    "authority_contract_id",
    "authority_barrier_digest",
    "authority_repair_generation",
    "authority_repair_fingerprint",
    "authority_repair_visible_epoch",
    "authority_attempt_id",
    "authority_attempt_count",
];

const REQUIRED_BASE_TRIGGERS: [&str; 4] = [
    "migration_rebuild_contract_change_clears_publication_attempt",
    "migration_rebuild_ready_clears_publication_attempt",
    "search_publication_authority_insert",
    "search_publication_authority_immutable",
];

const REQUIRED_RETIREMENT_TRIGGERS: [&str; 9] = [
    "search_publication_retirement_expectation_immutable",
    "search_publication_retirement_completion_guard_authority",
    "search_publication_retirement_completion_guard_immutable",
    "search_publication_retirement_completion_requires_guard",
    "search_publication_retirement_completion_guard_release",
    "search_publication_retirement_insert_once",
    "pending_search_publication_retirement_cannot_be_deleted",
    "search_publication_abandon_requires_retirement",
    "pending_search_publication_cannot_be_deleted",
];

pub(super) fn validate(connection: &Connection) -> Result<()> {
    let authority_column_count = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('search_publication_journal')
             WHERE name IN (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            AUTHORITY_COLUMNS,
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let required_table_count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
             AND name IN ('migration_rebuild_publication_attempt',
                          'search_publication_retirement',
                          'search_publication_retirement_completion_guard')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let required_trigger_count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger'
             AND name IN (?1, ?2, ?3, ?4)",
            REQUIRED_BASE_TRIGGERS,
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let pending_index_count = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index'
             AND name = 'search_publication_retirement_pending_idx'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let invalid_retirement_count = connection
        .query_row(
            "SELECT COUNT(*)
             FROM search_publication_journal AS publication
             LEFT JOIN search_publication_retirement AS retirement
               ON retirement.generation = publication.generation
             WHERE (publication.state = 'abandoned' AND retirement.generation IS NULL)
                OR (retirement.phase = 'pending' AND publication.state <> 'abandoned')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let pending_count = connection
        .query_row(
            "SELECT COUNT(*) FROM search_publication_retirement WHERE phase = 'pending'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    let completion_guard_count = connection
        .query_row(
            "SELECT COUNT(*) FROM search_publication_retirement_completion_guard",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if authority_column_count != AUTHORITY_COLUMNS.len() as i64
        || required_table_count != 3
        || required_trigger_count != REQUIRED_BASE_TRIGGERS.len() as i64
        || pending_index_count != 1
        || invalid_retirement_count != 0
        || completion_guard_count != 0
        || pending_count
            > i64::try_from(crate::SEARCH_PUBLICATION_RETIREMENT_REPLAY_LIMIT)
                .map_err(|_| MetaStoreError::storage_invariant())?
    {
        return Err(MetaStoreError::storage_invariant());
    }
    validate_retirement_trigger_definitions(connection)?;

    let attempt_schema = connection
        .query_row(
            "SELECT sql FROM sqlite_master
             WHERE type = 'table' AND name = 'migration_rebuild_publication_attempt'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(MetaStoreError::storage)?;
    if !attempt_schema.contains("'terminal'") || !attempt_schema.contains("'cleanup'") {
        return Err(MetaStoreError::storage_invariant());
    }

    let mut generations = connection
        .prepare("SELECT generation FROM search_publication_journal ORDER BY generation")
        .map_err(MetaStoreError::storage)?;
    let generations = generations
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(MetaStoreError::storage)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(MetaStoreError::storage)?;
    for generation in generations {
        crate::search_publication::validate_persisted_authority(connection, &generation)?;
    }
    Ok(())
}

fn validate_retirement_trigger_definitions(connection: &Connection) -> Result<()> {
    for trigger_name in REQUIRED_RETIREMENT_TRIGGERS {
        let actual = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
                [trigger_name],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(MetaStoreError::storage)?
            .ok_or_else(MetaStoreError::storage_invariant)?;
        let expected = expected_retirement_trigger_sql(trigger_name)?;
        if canonical_trigger_sql(&actual) != canonical_trigger_sql(expected) {
            return Err(MetaStoreError::storage_invariant());
        }
    }
    Ok(())
}

fn expected_retirement_trigger_sql(trigger_name: &str) -> Result<&'static str> {
    let marker = format!("CREATE TRIGGER {trigger_name}\n");
    let start = RETIREMENT_TRIGGER_SCHEMA
        .find(&marker)
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let expected = &RETIREMENT_TRIGGER_SCHEMA[start..];
    let end = expected
        .find("\nEND;")
        .ok_or_else(MetaStoreError::storage_invariant)?
        + "\nEND;".len();
    Ok(&expected[..end])
}

fn canonical_trigger_sql(sql: &str) -> String {
    sql.trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
