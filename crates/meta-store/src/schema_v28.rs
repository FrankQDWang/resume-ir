pub(super) const VERSION: u32 = 28;

/// v28 binds every import task and completed rebuild manifest to one exact
/// parser/schema/classifier contract. Existing v27 task rows are deliberately
/// not migrated into this schema: the copy-on-write migration creates a fresh
/// target and copies only stable document identity and authorized roots.
pub(super) const SCHEMA: &str = r#"
CREATE TABLE metadata_cow_staging_authority (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    target_visible_epoch INTEGER NOT NULL CHECK (target_visible_epoch >= 0)
);

CREATE TRIGGER metadata_cow_staging_authority_initial_head_only
BEFORE INSERT ON metadata_cow_staging_authority
WHEN NOT EXISTS (
    SELECT 1 FROM search_projection_state AS head
    WHERE head.state_key = 'default'
      AND head.service_state = 'repairing'
      AND head.generation IS NULL
      AND head.visible_epoch = 0
      AND head.repair_reason = 'migration_rebuild'
)
BEGIN
    SELECT RAISE(ABORT, 'COW staging authority requires initial repair head');
END;

CREATE TRIGGER metadata_cow_staging_authority_immutable_update
BEFORE UPDATE ON metadata_cow_staging_authority
BEGIN
    SELECT RAISE(ABORT, 'immutable COW staging authority');
END;

DROP TRIGGER search_projection_head_change_requires_commit_guard;

CREATE TRIGGER search_projection_head_change_requires_commit_guard
BEFORE UPDATE ON search_projection_state
WHEN NEW.generation IS NOT OLD.generation
  OR NEW.visible_epoch IS NOT OLD.visible_epoch
BEGIN
    SELECT CASE WHEN NOT (
        (
            NEW.service_state = 'ready'
            AND EXISTS (
                SELECT 1
                FROM search_publication_commit_guard AS commit_guard
                JOIN search_publication_journal AS publication
                  ON publication.generation = commit_guard.generation
                WHERE commit_guard.state_key = 'default'
                  AND publication.generation = NEW.generation
                  AND publication.state = 'ready'
                  AND publication.base_generation IS OLD.generation
                  AND publication.expected_visible_epoch = OLD.visible_epoch
                  AND NEW.visible_epoch = OLD.visible_epoch + 1
                  AND (
                      SELECT COUNT(*) FROM active_search_projection
                      WHERE generation = NEW.generation
                  ) = publication.fulltext_document_count
            )
        )
        OR (
            OLD.service_state = 'repairing'
            AND OLD.generation IS NULL
            AND OLD.visible_epoch = 0
            AND OLD.repair_reason = 'migration_rebuild'
            AND NEW.service_state = 'repairing'
            AND NEW.generation IS NULL
            AND NEW.repair_reason = 'migration_rebuild'
            AND EXISTS (
                SELECT 1 FROM metadata_cow_staging_authority AS authority
                WHERE authority.state_key = 'default'
                  AND authority.target_visible_epoch = NEW.visible_epoch
            )
        )
    ) THEN RAISE(ABORT, 'search projection head change requires guarded publication') END;
END;

CREATE TABLE import_processing_contract (
    id TEXT PRIMARY KEY NOT NULL CHECK (
        length(id) = 71
        AND substr(id, 1, 7) = 'sha256:'
        AND substr(id, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    primary_parse_version TEXT NOT NULL CHECK (
        length(primary_parse_version) BETWEEN 1 AND 64
        AND instr(primary_parse_version, char(0)) = 0
        AND instr(primary_parse_version, char(10)) = 0
        AND instr(primary_parse_version, char(13)) = 0
        AND instr(primary_parse_version, char(9)) = 0
    ),
    ocr_parse_version TEXT NOT NULL CHECK (
        length(ocr_parse_version) BETWEEN 1 AND 64
        AND instr(ocr_parse_version, char(0)) = 0
        AND instr(ocr_parse_version, char(10)) = 0
        AND instr(ocr_parse_version, char(13)) = 0
        AND instr(ocr_parse_version, char(9)) = 0
    ),
    derived_schema_version TEXT NOT NULL CHECK (
        length(derived_schema_version) BETWEEN 1 AND 64
        AND instr(derived_schema_version, char(0)) = 0
        AND instr(derived_schema_version, char(10)) = 0
        AND instr(derived_schema_version, char(13)) = 0
        AND instr(derived_schema_version, char(9)) = 0
    ),
    classifier_epoch TEXT NOT NULL CHECK (
        length(classifier_epoch) BETWEEN 1 AND 64
        AND classifier_epoch NOT GLOB '*[^a-z0-9_]*'
    ),
    UNIQUE (
        primary_parse_version, ocr_parse_version,
        derived_schema_version, classifier_epoch
    )
);

CREATE TRIGGER import_processing_contract_immutable_update
BEFORE UPDATE ON import_processing_contract
BEGIN
    SELECT RAISE(ABORT, 'immutable import processing contract');
END;

CREATE TABLE migration_rebuild_contract_state (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    active_contract_id TEXT,
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= 0),
    FOREIGN KEY (active_contract_id)
        REFERENCES import_processing_contract(id) ON DELETE RESTRICT
);

INSERT INTO migration_rebuild_contract_state (
    state_key, active_contract_id, updated_at_seconds
) VALUES ('default', NULL, 0);

CREATE TRIGGER migration_rebuild_contract_state_singleton_delete
BEFORE DELETE ON migration_rebuild_contract_state
BEGIN
    SELECT RAISE(ABORT, 'migration rebuild contract state is required');
END;

CREATE TABLE import_task_contract_binding (
    import_task_id TEXT PRIMARY KEY NOT NULL,
    processing_contract_id TEXT NOT NULL,
    UNIQUE (import_task_id, processing_contract_id),
    FOREIGN KEY (import_task_id) REFERENCES import_task(id) ON DELETE CASCADE,
    FOREIGN KEY (processing_contract_id)
        REFERENCES import_processing_contract(id) ON DELETE RESTRICT
);

CREATE TRIGGER import_task_contract_binding_immutable_update
BEFORE UPDATE ON import_task_contract_binding
BEGIN
    SELECT RAISE(ABORT, 'immutable import task contract binding');
END;

CREATE TABLE migration_rebuild_full_corpus_task (
    import_task_id TEXT PRIMARY KEY NOT NULL,
    processing_contract_id TEXT NOT NULL,
    UNIQUE (import_task_id, processing_contract_id),
    FOREIGN KEY (import_task_id, processing_contract_id)
        REFERENCES import_task_contract_binding(import_task_id, processing_contract_id)
        ON DELETE CASCADE
);

CREATE TRIGGER migration_rebuild_full_corpus_task_immutable_update
BEFORE UPDATE ON migration_rebuild_full_corpus_task
BEGIN
    SELECT RAISE(ABORT, 'immutable migration rebuild task purpose');
END;

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
    phase TEXT NOT NULL CHECK (phase IN ('running', 'retry_wait')),
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
    ),
    FOREIGN KEY (processing_contract_id)
        REFERENCES import_processing_contract(id) ON DELETE RESTRICT
);

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

CREATE TRIGGER import_task_no_direct_completed_insert
BEFORE INSERT ON import_task
WHEN NEW.status = 'completed'
BEGIN
    SELECT RAISE(ABORT, 'completed import task requires sealed completion');
END;

CREATE TRIGGER import_task_completed_requires_sealed_completion
BEFORE UPDATE OF status ON import_task
WHEN NEW.status = 'completed' AND NOT EXISTS (
    SELECT 1 FROM import_task_completion AS completion
    WHERE completion.import_task_id = NEW.id
)
BEGIN
    SELECT RAISE(ABORT, 'completed import task requires sealed completion');
END;

CREATE TABLE import_task_source_disposition (
    import_task_id TEXT NOT NULL,
    processing_contract_id TEXT NOT NULL,
    source_ordinal INTEGER NOT NULL CHECK (source_ordinal >= 0),
    document_id TEXT NOT NULL,
    source_revision_id TEXT NOT NULL,
    resume_version_id TEXT,
    disposition TEXT NOT NULL CHECK (
        disposition IN ('searchable', 'excluded', 'ocr_backlog', 'failed')
    ),
    PRIMARY KEY (import_task_id, source_ordinal),
    FOREIGN KEY (import_task_id, processing_contract_id)
        REFERENCES import_task_contract_binding(import_task_id, processing_contract_id)
        ON DELETE CASCADE,
    FOREIGN KEY (source_revision_id, document_id)
        REFERENCES source_revision(id, document_id) ON DELETE CASCADE,
    FOREIGN KEY (resume_version_id, document_id, source_revision_id)
        REFERENCES resume_version(id, document_id, source_revision_id) ON DELETE CASCADE,
    CHECK (
        (disposition IN ('searchable', 'excluded') AND resume_version_id IS NOT NULL)
        OR (disposition IN ('ocr_backlog', 'failed') AND resume_version_id IS NULL)
    )
);

CREATE INDEX import_task_source_disposition_document_idx
    ON import_task_source_disposition(import_task_id, document_id, source_ordinal);

CREATE TRIGGER import_task_source_disposition_no_insert_after_completion
BEFORE INSERT ON import_task_source_disposition
WHEN EXISTS (
    SELECT 1 FROM import_task_completion AS completion
    WHERE completion.import_task_id = NEW.import_task_id
)
BEGIN
    SELECT RAISE(ABORT, 'completed import task disposition is sealed');
END;

CREATE TRIGGER import_task_source_disposition_no_update
BEFORE UPDATE ON import_task_source_disposition
BEGIN
    SELECT RAISE(ABORT, 'import task source disposition is immutable');
END;

CREATE TRIGGER import_task_source_disposition_no_delete_after_completion
BEFORE DELETE ON import_task_source_disposition
WHEN EXISTS (
    SELECT 1 FROM import_task_completion AS completion
    WHERE completion.import_task_id = OLD.import_task_id
)
BEGIN
    SELECT RAISE(ABORT, 'completed import task disposition is sealed');
END;

CREATE TABLE import_task_completion (
    import_task_id TEXT PRIMARY KEY NOT NULL,
    processing_contract_id TEXT NOT NULL,
    source_disposition_count INTEGER NOT NULL CHECK (source_disposition_count >= 0),
    source_manifest_digest TEXT NOT NULL CHECK (
        length(source_manifest_digest) = 71
        AND substr(source_manifest_digest, 1, 7) = 'sha256:'
        AND substr(source_manifest_digest, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    completed_at_seconds INTEGER NOT NULL CHECK (completed_at_seconds >= 0),
    FOREIGN KEY (import_task_id, processing_contract_id)
        REFERENCES import_task_contract_binding(import_task_id, processing_contract_id)
        ON DELETE CASCADE
);

CREATE TRIGGER import_task_completion_requires_exact_scope_and_manifest
BEFORE INSERT ON import_task_completion
WHEN NOT EXISTS (
    SELECT 1
    FROM import_task AS task
    JOIN import_task_contract_binding AS binding
      ON binding.import_task_id = task.id
    JOIN import_scan_scope AS scope
      ON scope.import_task_id = task.id
    WHERE task.id = NEW.import_task_id
      AND task.status = 'running'
      AND binding.processing_contract_id = NEW.processing_contract_id
      AND scope.files_discovered = NEW.source_disposition_count
      AND NEW.source_disposition_count = (
          SELECT COUNT(*) FROM import_task_source_disposition AS disposition
          WHERE disposition.import_task_id = task.id
            AND disposition.processing_contract_id = NEW.processing_contract_id
      )
)
BEGIN
    SELECT RAISE(ABORT, 'import task completion requires exact scope manifest');
END;

CREATE TRIGGER import_task_completion_immutable_update
BEFORE UPDATE ON import_task_completion
BEGIN
    SELECT RAISE(ABORT, 'immutable import task completion');
END;

CREATE TRIGGER completed_import_scan_scope_immutable_update
BEFORE UPDATE ON import_scan_scope
WHEN EXISTS (
    SELECT 1 FROM import_task_completion AS completion
    WHERE completion.import_task_id = OLD.import_task_id
)
BEGIN
    SELECT RAISE(ABORT, 'completed import scan scope is sealed');
END;

CREATE TRIGGER completed_import_scan_scope_immutable_delete
BEFORE DELETE ON import_scan_scope
WHEN EXISTS (
    SELECT 1 FROM import_task_completion AS completion
    WHERE completion.import_task_id = OLD.import_task_id
)
BEGIN
    SELECT RAISE(ABORT, 'completed import scan scope is sealed');
END;
"#;
