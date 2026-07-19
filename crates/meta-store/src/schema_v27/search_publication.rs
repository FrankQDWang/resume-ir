pub(super) const SCHEMA: &str = r#"
CREATE TABLE search_publication_journal (
    generation TEXT PRIMARY KEY NOT NULL CHECK (
        length(generation) BETWEEN 1 AND 128
        AND generation NOT IN ('.', '..')
        AND substr(generation, 1, 1) <> '.'
        AND generation NOT GLOB '*[^A-Za-z0-9._-]*'
    ),
    base_generation TEXT,
    expected_visible_epoch INTEGER NOT NULL CHECK (expected_visible_epoch >= 0),
    classifier_epoch TEXT NOT NULL CHECK (
        length(classifier_epoch) BETWEEN 1 AND 64
        AND classifier_epoch NOT GLOB '*[^a-z0-9_]*'
    ),
    projection_digest TEXT NOT NULL CHECK (
        length(projection_digest) = 71
        AND substr(projection_digest, 1, 7) = 'sha256:'
        AND substr(projection_digest, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    publication_fingerprint TEXT,
    state TEXT NOT NULL CHECK (state IN ('preparing', 'validated', 'ready', 'abandoned')),
    fulltext_generation TEXT,
    fulltext_manifest_schema TEXT,
    fulltext_index_schema TEXT,
    fulltext_document_count INTEGER CHECK (fulltext_document_count >= 0),
    fulltext_projection_digest TEXT,
    fulltext_logical_content_digest TEXT,
    vector_generation TEXT,
    vector_manifest_schema TEXT,
    vector_index_schema TEXT,
    vector_mode TEXT CHECK (vector_mode IN ('disabled', 'enabled')),
    vector_model_id TEXT,
    vector_dimension INTEGER CHECK (vector_dimension > 0),
    vector_projection_count INTEGER CHECK (vector_projection_count >= 0),
    vector_coverage_digest TEXT,
    vector_count INTEGER CHECK (vector_count >= 0),
    vector_document_count INTEGER CHECK (vector_document_count >= 0),
    vector_resume_version_count INTEGER CHECK (vector_resume_version_count >= 0),
    vector_projection_digest TEXT,
    vector_logical_content_digest TEXT,
    created_at_seconds INTEGER NOT NULL CHECK (created_at_seconds >= 0),
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= 0),
    CHECK (base_generation IS NULL OR base_generation <> generation),
    CHECK (fulltext_generation IS NULL OR fulltext_generation = generation),
    CHECK (vector_generation IS NULL OR vector_generation = generation),
    CHECK (
        (
            publication_fingerprint IS NULL
            AND fulltext_generation IS NULL
            AND fulltext_manifest_schema IS NULL
            AND fulltext_index_schema IS NULL
            AND fulltext_document_count IS NULL
            AND fulltext_projection_digest IS NULL
            AND fulltext_logical_content_digest IS NULL
            AND vector_generation IS NULL
            AND vector_manifest_schema IS NULL
            AND vector_index_schema IS NULL
            AND vector_mode IS NULL
            AND vector_model_id IS NULL
            AND vector_dimension IS NULL
            AND vector_projection_count IS NULL
            AND vector_coverage_digest IS NULL
            AND vector_count IS NULL
            AND vector_document_count IS NULL
            AND vector_resume_version_count IS NULL
            AND vector_projection_digest IS NULL
            AND vector_logical_content_digest IS NULL
        ) OR (
            publication_fingerprint IS NOT NULL
            AND fulltext_generation IS NOT NULL
            AND fulltext_manifest_schema IS NOT NULL
            AND fulltext_index_schema IS NOT NULL
            AND fulltext_document_count IS NOT NULL
            AND fulltext_projection_digest IS NOT NULL
            AND fulltext_logical_content_digest IS NOT NULL
            AND vector_generation IS NOT NULL
            AND vector_manifest_schema IS NOT NULL
            AND vector_index_schema IS NOT NULL
            AND vector_mode IS NOT NULL
            AND vector_projection_count IS NOT NULL
            AND vector_coverage_digest IS NOT NULL
            AND vector_count IS NOT NULL
            AND vector_document_count IS NOT NULL
            AND vector_resume_version_count IS NOT NULL
            AND vector_projection_digest IS NOT NULL
            AND vector_logical_content_digest IS NOT NULL
        )
    ),
    CHECK (
        state = 'preparing' AND publication_fingerprint IS NULL
        OR state IN ('validated', 'ready') AND publication_fingerprint IS NOT NULL
        OR state = 'abandoned'
    ),
    CHECK (
        vector_mode IS NULL
        OR (
            vector_mode = 'disabled'
            AND vector_model_id IS NULL
            AND vector_dimension IS NULL
            AND vector_count = 0
            AND vector_document_count = 0
            AND vector_resume_version_count = 0
        )
        OR (
            vector_mode = 'enabled'
            AND vector_model_id IS NOT NULL
            AND length(trim(vector_model_id)) > 0
            AND vector_dimension IS NOT NULL
            AND vector_count >= vector_document_count
            AND vector_document_count = vector_resume_version_count
        )
    ),
    CHECK (
        publication_fingerprint IS NULL OR (
            length(publication_fingerprint) = 71
            AND substr(publication_fingerprint, 1, 7) = 'sha256:'
            AND substr(publication_fingerprint, 8) NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        fulltext_projection_digest IS NULL OR (
            length(fulltext_projection_digest) = 71
            AND substr(fulltext_projection_digest, 1, 7) = 'sha256:'
            AND substr(fulltext_projection_digest, 8) NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        vector_projection_digest IS NULL OR (
            length(vector_projection_digest) = 71
            AND substr(vector_projection_digest, 1, 7) = 'sha256:'
            AND substr(vector_projection_digest, 8) NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        vector_coverage_digest IS NULL OR (
            length(vector_coverage_digest) = 71
            AND substr(vector_coverage_digest, 1, 7) = 'sha256:'
            AND substr(vector_coverage_digest, 8) NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        fulltext_logical_content_digest IS NULL OR (
            length(fulltext_logical_content_digest) = 71
            AND substr(fulltext_logical_content_digest, 1, 7) = 'sha256:'
            AND substr(fulltext_logical_content_digest, 8) NOT GLOB '*[^0-9a-f]*'
        )
    ),
    CHECK (
        vector_logical_content_digest IS NULL OR (
            length(vector_logical_content_digest) = 71
            AND substr(vector_logical_content_digest, 1, 7) = 'sha256:'
            AND substr(vector_logical_content_digest, 8) NOT GLOB '*[^0-9a-f]*'
        )
    )
);

CREATE INDEX search_publication_recovery_idx
    ON search_publication_journal(state, updated_at_seconds);
CREATE INDEX search_publication_fingerprint_idx
    ON search_publication_journal(publication_fingerprint)
    WHERE state = 'ready';

CREATE TABLE search_publication_commit_guard (
    state_key TEXT PRIMARY KEY NOT NULL CHECK (state_key = 'default'),
    generation TEXT NOT NULL UNIQUE,
    FOREIGN KEY (generation)
        REFERENCES search_publication_journal(generation) ON DELETE RESTRICT
);

CREATE TRIGGER search_publication_commit_guard_authority
BEFORE INSERT ON search_publication_commit_guard
WHEN NOT EXISTS (
    SELECT 1
    FROM search_publication_journal AS successor
    JOIN search_projection_state AS head
      ON successor.base_generation IS head.generation
     AND successor.expected_visible_epoch = head.visible_epoch
    WHERE NEW.state_key = 'default'
      AND head.state_key = 'default'
      AND successor.generation = NEW.generation
      AND successor.state = 'validated'
)
BEGIN
    SELECT RAISE(ABORT, 'search publication commit guard lacks CAS authority');
END;

CREATE TABLE active_search_projection (
    document_id TEXT PRIMARY KEY NOT NULL,
    resume_version_id TEXT NOT NULL,
    generation TEXT NOT NULL,
    source_uri TEXT NOT NULL,
    normalized_path TEXT NOT NULL,
    file_name TEXT NOT NULL,
    extension TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    mtime_seconds INTEGER NOT NULL,
    content_hash TEXT NOT NULL CHECK (
        length(content_hash) = 71
        AND substr(content_hash, 1, 7) = 'sha256:'
        AND substr(content_hash, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    text_hash TEXT,
    is_deleted INTEGER NOT NULL CHECK (is_deleted = 0),
    created_at_seconds INTEGER NOT NULL,
    updated_at_seconds INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status = 'searchable'),
    UNIQUE (resume_version_id),
    FOREIGN KEY (resume_version_id, document_id)
        REFERENCES resume_version(id, document_id) ON DELETE RESTRICT,
    FOREIGN KEY (generation)
        REFERENCES search_publication_journal(generation) ON DELETE RESTRICT
);

CREATE TRIGGER active_search_projection_exact_version_metadata
BEFORE INSERT ON active_search_projection
WHEN NOT EXISTS (
    SELECT 1
    FROM resume_version AS version
    JOIN source_revision AS revision
      ON revision.id = version.source_revision_id
     AND revision.document_id = version.document_id
    WHERE version.id = NEW.resume_version_id
      AND version.document_id = NEW.document_id
      AND revision.content_hash = NEW.content_hash
      AND revision.byte_size = NEW.byte_size
)
BEGIN
    SELECT RAISE(ABORT, 'active projection metadata must match exact source revision');
END;

CREATE TRIGGER active_search_projection_immutable_update
BEFORE UPDATE ON active_search_projection
BEGIN
    SELECT RAISE(ABORT, 'active projection rows are immutable');
END;

CREATE TRIGGER active_projection_delete_requires_validated_successor
BEFORE DELETE ON active_search_projection
WHEN NOT EXISTS (
    SELECT 1
    FROM search_publication_commit_guard AS commit_guard
    JOIN search_publication_journal AS successor
      ON successor.generation = commit_guard.generation
    JOIN search_projection_state AS head
      ON successor.base_generation = head.generation
     AND successor.expected_visible_epoch = head.visible_epoch
     AND successor.state = 'validated'
    WHERE commit_guard.state_key = 'default'
      AND head.state_key = 'default'
      AND head.generation = OLD.generation
)
BEGIN
    SELECT RAISE(ABORT, 'active projection delete requires validated successor');
END;

CREATE TRIGGER resume_version_seal_guarded_delete
BEFORE DELETE ON resume_version_seal
WHEN NOT EXISTS (
    SELECT 1
    FROM resume_version AS version
    JOIN document ON document.id = version.document_id
    WHERE version.id = OLD.resume_version_id
      AND document.is_deleted = 1
      AND document.status = 'deleted'
      AND NOT EXISTS (
          SELECT 1 FROM active_search_projection AS projection
          WHERE projection.resume_version_id = OLD.resume_version_id
             OR projection.document_id = document.id
      )
)
BEGIN
    SELECT RAISE(ABORT, 'sealed resume version is not purgeable');
END;

CREATE TABLE search_projection_state (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    service_state TEXT NOT NULL CHECK (
        service_state IN ('repairing', 'ready', 'repair_blocked')
    ),
    generation TEXT,
    visible_epoch INTEGER NOT NULL CHECK (visible_epoch >= 0),
    repair_reason TEXT CHECK (repair_reason IN (
        'migration_rebuild', 'artifact_unavailable', 'source_unavailable',
        'runtime_invariant'
    )),
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= 0),
    CHECK (
        (service_state = 'ready' AND generation IS NOT NULL AND repair_reason IS NULL)
        OR (service_state = 'repairing' AND repair_reason IS NOT NULL)
        OR (service_state = 'repair_blocked' AND repair_reason IS NOT NULL)
    ),
    FOREIGN KEY (generation)
        REFERENCES search_publication_journal(generation) ON DELETE RESTRICT
);

CREATE TRIGGER search_projection_state_singleton_delete
BEFORE DELETE ON search_projection_state
BEGIN
    SELECT RAISE(ABORT, 'search projection state is required');
END;

CREATE TRIGGER ready_projection_head_matches_journal
BEFORE UPDATE ON search_projection_state
WHEN NEW.service_state = 'ready' AND NOT EXISTS (
    SELECT 1 FROM search_publication_journal AS publication
    WHERE publication.generation = NEW.generation
      AND publication.state = 'ready'
      AND NEW.visible_epoch = publication.expected_visible_epoch + 1
)
BEGIN
    SELECT RAISE(ABORT, 'ready search head requires ready publication');
END;

CREATE TRIGGER active_projection_requires_validated_publication
BEFORE INSERT ON active_search_projection
WHEN NOT EXISTS (
    SELECT 1
    FROM search_publication_commit_guard AS commit_guard
    JOIN search_publication_journal AS publication
      ON publication.generation = commit_guard.generation
    JOIN search_projection_state AS head
      ON publication.base_generation IS head.generation
     AND publication.expected_visible_epoch = head.visible_epoch
    WHERE commit_guard.state_key = 'default'
      AND publication.generation = NEW.generation
      AND publication.state = 'validated'
)
BEGIN
    SELECT RAISE(ABORT, 'active projection requires guarded publication');
END;

CREATE TRIGGER search_publication_static_identity_immutable
BEFORE UPDATE ON search_publication_journal
WHEN NEW.generation IS NOT OLD.generation
  OR NEW.base_generation IS NOT OLD.base_generation
  OR NEW.expected_visible_epoch IS NOT OLD.expected_visible_epoch
  OR NEW.classifier_epoch IS NOT OLD.classifier_epoch
  OR NEW.projection_digest IS NOT OLD.projection_digest
  OR NEW.created_at_seconds IS NOT OLD.created_at_seconds
BEGIN
    SELECT RAISE(ABORT, 'search publication identity is immutable');
END;

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

CREATE TRIGGER ready_search_publication_requires_commit_guard
BEFORE UPDATE OF state ON search_publication_journal
WHEN NEW.state = 'ready' AND NOT EXISTS (
    SELECT 1 FROM search_publication_commit_guard AS commit_guard
    WHERE commit_guard.state_key = 'default'
      AND commit_guard.generation = NEW.generation
)
BEGIN
    SELECT RAISE(ABORT, 'ready search publication requires commit guard');
END;

CREATE TRIGGER ready_search_publication_immutable_update
BEFORE UPDATE ON search_publication_journal
WHEN OLD.state = 'ready'
BEGIN
    SELECT RAISE(ABORT, 'ready search publication is immutable');
END;

CREATE TRIGGER search_projection_head_change_requires_commit_guard
BEFORE UPDATE ON search_projection_state
WHEN NEW.generation IS NOT OLD.generation
  OR NEW.visible_epoch IS NOT OLD.visible_epoch
BEGIN
    SELECT CASE WHEN NEW.service_state <> 'ready' OR NOT EXISTS (
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
          AND NOT EXISTS (
              SELECT 1 FROM active_search_projection AS projection
              WHERE projection.generation <> NEW.generation
          )
          AND (
              SELECT COUNT(*) FROM active_search_projection
              WHERE generation = NEW.generation
          ) = publication.fulltext_document_count
    ) THEN RAISE(ABORT, 'search projection head change requires guarded publication') END;
END;

CREATE TRIGGER search_publication_commit_guard_release
BEFORE DELETE ON search_publication_commit_guard
WHEN NOT EXISTS (
    SELECT 1
    FROM search_publication_journal AS publication
    JOIN search_projection_state AS head
      ON head.generation = publication.generation
     AND head.visible_epoch = publication.expected_visible_epoch + 1
    WHERE OLD.state_key = 'default'
      AND publication.generation = OLD.generation
      AND publication.state = 'ready'
      AND head.state_key = 'default'
      AND head.service_state = 'ready'
      AND NOT EXISTS (
          SELECT 1 FROM active_search_projection AS projection
          WHERE projection.generation <> publication.generation
      )
      AND (
          SELECT COUNT(*) FROM active_search_projection
          WHERE generation = publication.generation
      ) = publication.fulltext_document_count
)
BEGIN
    SELECT RAISE(ABORT, 'search publication commit guard released before CAS completion');
END;

INSERT INTO search_projection_state (
    state_key, service_state, generation, visible_epoch, repair_reason,
    updated_at_seconds
) VALUES (
    'default', 'repairing', NULL,
    (SELECT visible_epoch FROM v27_legacy_search_epoch),
    'migration_rebuild', 0
);

DROP TABLE v27_legacy_search_epoch;

UPDATE document
SET status = CASE WHEN is_deleted = 1 THEN 'deleted' ELSE 'discovered' END,
    content_hash = NULL,
    text_hash = NULL;
"#;
