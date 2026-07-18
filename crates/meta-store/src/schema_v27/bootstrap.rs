pub(super) const SCHEMA: &str = r#"
CREATE TEMP TABLE v27_legacy_search_epoch (
    visible_epoch INTEGER NOT NULL CHECK (visible_epoch >= 0)
);
INSERT INTO v27_legacy_search_epoch (visible_epoch)
SELECT COALESCE((
    SELECT visible_epoch FROM index_state WHERE state_key = 'default'
), 0);

CREATE TABLE authorized_import_root (
    canonical_root_path TEXT PRIMARY KEY NOT NULL CHECK (length(canonical_root_path) > 0),
    requested_root_path TEXT NOT NULL CHECK (length(requested_root_path) > 0),
    root_kind TEXT NOT NULL CHECK (root_kind IN ('explicit', 'preset')),
    root_preset TEXT CHECK (root_preset IS NULL OR root_preset IN ('local_discovery')),
    scan_profile TEXT NOT NULL CHECK (scan_profile IN ('explicit', 'discovery')),
    scan_budget_kind TEXT CHECK (scan_budget_kind IS NULL OR scan_budget_kind = 'files'),
    scan_budget_limit INTEGER CHECK (scan_budget_limit IS NULL OR scan_budget_limit >= 0),
    paused INTEGER NOT NULL CHECK (paused IN (0, 1)),
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= 0),
    CHECK (
        (root_kind = 'explicit' AND root_preset IS NULL)
        OR (root_kind = 'preset' AND root_preset IS NOT NULL)
    ),
    CHECK (
        (scan_budget_kind IS NULL AND scan_budget_limit IS NULL)
        OR (scan_budget_kind IS NOT NULL AND scan_budget_limit IS NOT NULL)
    )
);

INSERT INTO authorized_import_root (
    canonical_root_path, requested_root_path, root_kind, root_preset,
    scan_profile, scan_budget_kind, scan_budget_limit, paused, updated_at_seconds
)
SELECT scope.canonical_root_path, scope.requested_root_path, scope.root_kind,
       scope.root_preset, scope.scan_profile, scope.scan_budget_kind,
       scope.scan_budget_limit, COALESCE(control.paused, 0),
       MAX(scope.updated_at_seconds, COALESCE(control.updated_at_seconds, 0))
FROM import_scan_scope AS scope
LEFT JOIN import_root_control AS control
  ON control.canonical_root_path = scope.canonical_root_path
WHERE NOT EXISTS (
    SELECT 1
    FROM import_scan_scope AS newer
    WHERE newer.canonical_root_path = scope.canonical_root_path
      AND (
          newer.updated_at_seconds > scope.updated_at_seconds
          OR (
              newer.updated_at_seconds = scope.updated_at_seconds
              AND newer.rowid > scope.rowid
          )
      )
);

CREATE INDEX authorized_import_root_paused_idx
    ON authorized_import_root(paused, updated_at_seconds);

DELETE FROM import_task;
DELETE FROM ingest_job;
DELETE FROM ocr_page_cache;
DELETE FROM query_observation;
DROP INDEX ingest_job_ocr_document_unique_idx;

DELETE FROM candidate_contact_conflict;
DELETE FROM embedding_job_spec;
DELETE FROM entity_mention;
DELETE FROM document_classification_reason;
DELETE FROM document_classification;
DROP TABLE index_publication;
DROP TABLE index_state;
DELETE FROM resume_version;
DELETE FROM candidate;

DROP TABLE candidate_contact_conflict;
DROP TABLE embedding_job_spec;
DROP TABLE entity_mention;
DROP TABLE document_classification_reason;
DROP TABLE document_classification;
DROP TABLE import_root_control;
DROP INDEX IF EXISTS resume_version_candidate_idx;
DROP INDEX IF EXISTS resume_version_document_idx;
DROP TABLE resume_version;

CREATE TABLE metadata_store_identity (
    state_key TEXT PRIMARY KEY CHECK (state_key = 'default'),
    store_id_digest TEXT NOT NULL CHECK (
        length(store_id_digest) = 64
        AND store_id_digest NOT GLOB '*[^0-9a-f]*'
    )
);

CREATE TRIGGER metadata_store_identity_immutable_update
BEFORE UPDATE ON metadata_store_identity
BEGIN
    SELECT RAISE(ABORT, 'immutable metadata store identity');
END;
"#;
