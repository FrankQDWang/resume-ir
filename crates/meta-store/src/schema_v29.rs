pub(super) const VERSION: u32 = 29;

macro_rules! current_repair_context_authority_sql {
    () => {
        r#"CREATE TRIGGER artifact_repair_context_insert_authority
BEFORE INSERT ON artifact_repair_context
WHEN NOT EXISTS (
    SELECT 1
    FROM search_projection_state AS head
    JOIN search_publication_journal AS publication
      ON publication.generation = head.generation
    WHERE head.state_key = NEW.state_key
      AND head.generation = NEW.generation
      AND head.visible_epoch = NEW.visible_epoch
      AND head.visible_epoch = publication.expected_visible_epoch + 1
      AND (
          (head.service_state = 'ready' AND head.repair_reason IS NULL)
          OR (head.service_state = 'repairing'
              AND head.repair_reason = 'artifact_unavailable')
          OR (head.service_state = 'repair_blocked'
              AND head.repair_reason = 'runtime_invariant')
      )
      AND publication.state = 'ready'
      AND publication.publication_fingerprint = NEW.publication_fingerprint
      AND publication.classifier_epoch = NEW.classifier_epoch
      AND publication.projection_digest = NEW.projection_digest
      AND publication.fulltext_generation = NEW.generation
      AND publication.vector_generation = NEW.generation
      AND publication.fulltext_document_count = NEW.projection_count
      AND publication.vector_projection_count = NEW.projection_count
      AND publication.fulltext_projection_digest = NEW.projection_digest
      AND publication.vector_projection_digest = NEW.projection_digest
      AND publication.vector_mode = NEW.vector_mode
      AND publication.vector_model_id IS NEW.vector_model_id
      AND publication.vector_dimension IS NEW.vector_dimension
      AND publication.fulltext_manifest_schema = 'fulltext.snapshot.v3'
      AND publication.fulltext_index_schema = 'tantivy.fulltext.v3'
      AND publication.vector_manifest_schema = 'vector.snapshot.v4'
      AND publication.vector_index_schema = 'hnsw-vector.v4'
      AND NOT EXISTS (
          SELECT 1 FROM active_search_projection AS projection
          WHERE projection.generation <> NEW.generation
      )
      AND (
          SELECT COUNT(*) FROM active_search_projection
          WHERE generation = NEW.generation
      ) = NEW.projection_count
      AND NOT EXISTS (
          SELECT 1
          FROM active_search_projection AS projection
          JOIN resume_version AS version
            ON version.id = projection.resume_version_id
           AND version.document_id = projection.document_id
          JOIN source_revision AS revision
            ON revision.id = version.source_revision_id
           AND revision.document_id = version.document_id
          LEFT JOIN resume_version_seal AS seal
            ON seal.resume_version_id = projection.resume_version_id
          LEFT JOIN resume_version_classification AS classification
            ON classification.resume_version_id = projection.resume_version_id
           AND classification.classifier_epoch = NEW.classifier_epoch
           AND classification.status = 'resume_candidate'
          WHERE projection.generation = NEW.generation
            AND (seal.resume_version_id IS NULL
              OR classification.resume_version_id IS NULL
              OR projection.is_deleted <> 0
              OR projection.status <> 'searchable'
              OR projection.content_hash <> revision.content_hash
              OR projection.byte_size <> revision.byte_size)
      )
)
BEGIN
    SELECT RAISE(ABORT, 'artifact repair context lacks exact head authority');
END;"#
    };
}

pub(super) const CURRENT_REPAIR_CONTEXT_AUTHORITY: &str = current_repair_context_authority_sql!();

/// v29 adds the durable, exact repair context and bounded attempt ledger used
/// to rebuild an already-published immutable search generation. File-backed
/// v28 stores reach this schema only through the additive COW migration; this
/// SQL is never used as a runtime compatibility fallback.
pub(super) const SCHEMA: &str = concat!(
    r#"
CREATE TABLE artifact_repair_context (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    generation TEXT NOT NULL,
    publication_fingerprint TEXT NOT NULL CHECK (
        length(publication_fingerprint) = 71
        AND substr(publication_fingerprint, 1, 7) = 'sha256:'
        AND substr(publication_fingerprint, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    visible_epoch INTEGER NOT NULL CHECK (visible_epoch >= 0),
    classifier_epoch TEXT NOT NULL CHECK (
        length(classifier_epoch) BETWEEN 1 AND 64
        AND classifier_epoch NOT GLOB '*[^a-z0-9_]*'
    ),
    projection_digest TEXT NOT NULL CHECK (
        length(projection_digest) = 71
        AND substr(projection_digest, 1, 7) = 'sha256:'
        AND substr(projection_digest, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    projection_count INTEGER NOT NULL CHECK (projection_count >= 0),
    vector_mode TEXT NOT NULL CHECK (vector_mode IN ('disabled', 'enabled')),
    vector_model_id TEXT,
    vector_dimension INTEGER,
    created_at_seconds INTEGER NOT NULL CHECK (created_at_seconds >= 0),
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= 0),
    UNIQUE (generation, publication_fingerprint, visible_epoch),
    FOREIGN KEY (generation)
        REFERENCES search_publication_journal(generation) ON DELETE RESTRICT,
    CHECK (
        (vector_mode = 'disabled'
         AND vector_model_id IS NULL
         AND vector_dimension IS NULL)
        OR
        (vector_mode = 'enabled'
         AND vector_model_id IS NOT NULL
         AND length(trim(vector_model_id)) BETWEEN 1 AND 128
         AND vector_dimension BETWEEN 1 AND 65536)
    )
);
"#,
    current_repair_context_authority_sql!(),
    r#"
CREATE TRIGGER artifact_repair_context_immutable_update
BEFORE UPDATE ON artifact_repair_context
BEGIN
    SELECT RAISE(ABORT, 'artifact repair context is immutable');
END;

CREATE TABLE artifact_repair_attempt (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    generation TEXT NOT NULL,
    publication_fingerprint TEXT NOT NULL,
    visible_epoch INTEGER NOT NULL CHECK (visible_epoch >= 0),
    attempt_id TEXT NOT NULL CHECK (
        length(attempt_id) = 71
        AND substr(attempt_id, 1, 7) = 'sha256:'
        AND substr(attempt_id, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    attempt_count INTEGER NOT NULL CHECK (attempt_count BETWEEN 1 AND 5),
    phase TEXT NOT NULL CHECK (phase IN ('running', 'retry_wait', 'terminal')),
    started_at_seconds INTEGER NOT NULL CHECK (started_at_seconds >= 0),
    next_retry_at_seconds INTEGER,
    last_error_kind TEXT CHECK (
        last_error_kind IN (
            'fulltext_publication_busy',
            'fulltext_failure',
            'vector_publication_busy',
            'vector_failure',
            'metadata_failure',
            'cleanup',
            'interrupted'
        )
    ),
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= 0),
    FOREIGN KEY (generation, publication_fingerprint, visible_epoch)
        REFERENCES artifact_repair_context(
            generation, publication_fingerprint, visible_epoch
        ) ON DELETE CASCADE,
    CHECK (
        (phase = 'running'
         AND next_retry_at_seconds IS NULL
         AND last_error_kind IS NULL)
        OR
        (phase = 'retry_wait'
         AND next_retry_at_seconds IS NOT NULL
         AND last_error_kind IS NOT NULL)
        OR
        (phase = 'terminal'
         AND next_retry_at_seconds IS NULL
         AND last_error_kind IS NOT NULL)
    )
);

CREATE TRIGGER artifact_repair_attempt_insert_authority
BEFORE INSERT ON artifact_repair_attempt
WHEN NOT EXISTS (
    SELECT 1
    FROM artifact_repair_context AS context
    JOIN search_projection_state AS head ON head.state_key = context.state_key
    WHERE context.state_key = NEW.state_key
      AND context.generation = NEW.generation
      AND context.publication_fingerprint = NEW.publication_fingerprint
      AND context.visible_epoch = NEW.visible_epoch
      AND head.generation = context.generation
      AND head.visible_epoch = context.visible_epoch
      AND head.service_state = 'repairing'
      AND head.repair_reason = 'artifact_unavailable'
)
BEGIN
    SELECT RAISE(ABORT, 'artifact repair attempt lacks exact head authority');
END;

CREATE TRIGGER artifact_repair_context_head_change_cleanup
AFTER UPDATE OF service_state, generation, visible_epoch, repair_reason
ON search_projection_state
WHEN NEW.service_state = 'ready'
  OR NEW.generation IS NOT (
      SELECT generation FROM artifact_repair_context WHERE state_key = 'default'
  )
  OR NEW.visible_epoch IS NOT (
      SELECT visible_epoch FROM artifact_repair_context WHERE state_key = 'default'
  )
  OR (
      NEW.service_state = 'repairing'
      AND NEW.repair_reason <> 'artifact_unavailable'
  )
  OR (
      NEW.service_state = 'repair_blocked'
      AND NEW.repair_reason <> 'runtime_invariant'
  )
BEGIN
    DELETE FROM artifact_repair_context WHERE state_key = 'default';
END;
"#,
);

/// Replaces the permanent current-only authority only inside the v28-to-v29
/// COW transaction. The migration trigger accepts the one exact legacy
/// fulltext-v2/vector-v3 contract long enough to seal its repair context; the
/// caller must restore the captured permanent definition before commit.
pub(super) const INSTALL_MIGRATION_REPAIR_CONTEXT_AUTHORITY: &str = r#"
DROP TRIGGER artifact_repair_context_insert_authority;

CREATE TRIGGER artifact_repair_context_insert_migration_authority
BEFORE INSERT ON artifact_repair_context
WHEN NOT EXISTS (
    SELECT 1
    FROM search_projection_state AS head
    JOIN search_publication_journal AS publication
      ON publication.generation = head.generation
    WHERE head.state_key = NEW.state_key
      AND head.generation = NEW.generation
      AND head.visible_epoch = NEW.visible_epoch
      AND head.visible_epoch = publication.expected_visible_epoch + 1
      AND (
          (head.service_state = 'ready' AND head.repair_reason IS NULL)
          OR (head.service_state = 'repairing'
              AND head.repair_reason = 'artifact_unavailable')
          OR (head.service_state = 'repair_blocked'
              AND head.repair_reason = 'runtime_invariant')
      )
      AND publication.state = 'ready'
      AND publication.publication_fingerprint = NEW.publication_fingerprint
      AND publication.classifier_epoch = NEW.classifier_epoch
      AND publication.projection_digest = NEW.projection_digest
      AND publication.fulltext_generation = NEW.generation
      AND publication.vector_generation = NEW.generation
      AND publication.fulltext_document_count = NEW.projection_count
      AND publication.vector_projection_count = NEW.projection_count
      AND publication.fulltext_projection_digest = NEW.projection_digest
      AND publication.vector_projection_digest = NEW.projection_digest
      AND publication.vector_mode = NEW.vector_mode
      AND publication.vector_model_id IS NEW.vector_model_id
      AND publication.vector_dimension IS NEW.vector_dimension
      AND (
          (
              publication.fulltext_manifest_schema = 'fulltext.snapshot.v3'
              AND publication.fulltext_index_schema = 'tantivy.fulltext.v3'
              AND publication.vector_manifest_schema = 'vector.snapshot.v4'
              AND publication.vector_index_schema = 'hnsw-vector.v4'
          )
          OR
          (
              publication.fulltext_manifest_schema = 'fulltext.snapshot.v2'
              AND publication.fulltext_index_schema = 'tantivy.fulltext.v2'
              AND publication.vector_manifest_schema = 'vector.snapshot.v3'
              AND publication.vector_index_schema = 'hnsw-vector.v3'
          )
      )
      AND NOT EXISTS (
          SELECT 1 FROM active_search_projection AS projection
          WHERE projection.generation <> NEW.generation
      )
      AND (
          SELECT COUNT(*) FROM active_search_projection
          WHERE generation = NEW.generation
      ) = NEW.projection_count
      AND NOT EXISTS (
          SELECT 1
          FROM active_search_projection AS projection
          JOIN resume_version AS version
            ON version.id = projection.resume_version_id
           AND version.document_id = projection.document_id
          JOIN source_revision AS revision
            ON revision.id = version.source_revision_id
           AND revision.document_id = version.document_id
          LEFT JOIN resume_version_seal AS seal
            ON seal.resume_version_id = projection.resume_version_id
          LEFT JOIN resume_version_classification AS classification
            ON classification.resume_version_id = projection.resume_version_id
           AND classification.classifier_epoch = NEW.classifier_epoch
           AND classification.status = 'resume_candidate'
          WHERE projection.generation = NEW.generation
            AND (seal.resume_version_id IS NULL
              OR classification.resume_version_id IS NULL
              OR projection.is_deleted <> 0
              OR projection.status <> 'searchable'
              OR projection.content_hash <> revision.content_hash
              OR projection.byte_size <> revision.byte_size)
      )
)
BEGIN
    SELECT RAISE(ABORT, 'artifact repair context lacks exact migration authority');
END;
"#;

/// Immutable publication triggers temporarily removed only inside the v29
/// staging transaction while exact v2/v3 descriptor payloads are isolated.
pub(super) const DROP_LEGACY_ISOLATION_TRIGGERS: &str = r#"
DROP TRIGGER search_publication_payload_immutable_after_validation;
DROP TRIGGER search_publication_same_state_immutable;
DROP TRIGGER search_publication_transition;
DROP TRIGGER ready_search_publication_immutable_update;
"#;

pub(super) const RESTORE_LEGACY_ISOLATION_TRIGGERS: &str = r#"
CREATE TRIGGER search_publication_payload_immutable_after_validation
BEFORE UPDATE ON search_publication_journal
WHEN OLD.state IN ('validated', 'ready', 'abandoned') AND (
    NEW.publication_fingerprint IS NOT OLD.publication_fingerprint
    OR NEW.fulltext_generation IS NOT OLD.fulltext_generation
    OR NEW.fulltext_manifest_schema IS NOT OLD.fulltext_manifest_schema
    OR NEW.fulltext_index_schema IS NOT OLD.fulltext_index_schema
    OR NEW.fulltext_document_count IS NOT OLD.fulltext_document_count
    OR NEW.fulltext_projection_digest IS NOT OLD.fulltext_projection_digest
    OR NEW.fulltext_logical_content_digest IS NOT OLD.fulltext_logical_content_digest
    OR NEW.vector_generation IS NOT OLD.vector_generation
    OR NEW.vector_manifest_schema IS NOT OLD.vector_manifest_schema
    OR NEW.vector_index_schema IS NOT OLD.vector_index_schema
    OR NEW.vector_mode IS NOT OLD.vector_mode
    OR NEW.vector_model_id IS NOT OLD.vector_model_id
    OR NEW.vector_dimension IS NOT OLD.vector_dimension
    OR NEW.vector_projection_count IS NOT OLD.vector_projection_count
    OR NEW.vector_coverage_digest IS NOT OLD.vector_coverage_digest
    OR NEW.vector_count IS NOT OLD.vector_count
    OR NEW.vector_document_count IS NOT OLD.vector_document_count
    OR NEW.vector_resume_version_count IS NOT OLD.vector_resume_version_count
    OR NEW.vector_projection_digest IS NOT OLD.vector_projection_digest
    OR NEW.vector_logical_content_digest IS NOT OLD.vector_logical_content_digest
)
BEGIN
    SELECT RAISE(ABORT, 'validated search publication payload is immutable');
END;

CREATE TRIGGER search_publication_same_state_immutable
BEFORE UPDATE ON search_publication_journal
WHEN NEW.state = OLD.state
BEGIN
    SELECT RAISE(ABORT, 'search publication update requires a state transition');
END;

CREATE TRIGGER search_publication_transition
BEFORE UPDATE OF state ON search_publication_journal
WHEN NOT (
    OLD.state = 'preparing' AND NEW.state IN ('validated', 'abandoned')
    OR OLD.state = 'validated' AND NEW.state IN ('ready', 'abandoned')
)
BEGIN
    SELECT RAISE(ABORT, 'invalid search publication transition');
END;

CREATE TRIGGER ready_search_publication_immutable_update
BEFORE UPDATE ON search_publication_journal
WHEN OLD.state = 'ready'
BEGIN
    SELECT RAISE(ABORT, 'ready search publication is immutable');
END;
"#;
