pub(super) const SCHEMA: &str = r#"
CREATE TABLE source_revision (
    id TEXT PRIMARY KEY NOT NULL,
    document_id TEXT NOT NULL,
    content_hash TEXT NOT NULL CHECK (
        length(content_hash) = 71
        AND substr(content_hash, 1, 7) = 'sha256:'
        AND substr(content_hash, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    UNIQUE (id, document_id),
    UNIQUE (document_id, content_hash),
    FOREIGN KEY (document_id) REFERENCES document(id) ON DELETE CASCADE
);

CREATE TRIGGER source_revision_immutable_update
BEFORE UPDATE ON source_revision
BEGIN
    SELECT RAISE(ABORT, 'immutable source revision');
END;

CREATE TABLE source_revision_triage (
    source_revision_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ocr_backlog', 'failed')),
    triage_epoch TEXT NOT NULL CHECK (
        length(triage_epoch) BETWEEN 1 AND 64
        AND triage_epoch NOT GLOB '*[^a-z0-9_]*'
    ),
    triaged_at_seconds INTEGER NOT NULL CHECK (triaged_at_seconds >= 0),
    PRIMARY KEY (source_revision_id, triage_epoch),
    FOREIGN KEY (source_revision_id) REFERENCES source_revision(id) ON DELETE CASCADE
);

CREATE TRIGGER source_revision_triage_immutable_update
BEFORE UPDATE ON source_revision_triage
BEGIN
    SELECT RAISE(ABORT, 'immutable source revision triage');
END;

CREATE TABLE source_revision_triage_reason (
    source_revision_id TEXT NOT NULL,
    triage_epoch TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 7),
    reason_code TEXT NOT NULL CHECK (reason_code IN ('ocr_required', 'parser_failed')),
    PRIMARY KEY (source_revision_id, triage_epoch, ordinal),
    UNIQUE (source_revision_id, triage_epoch, reason_code),
    FOREIGN KEY (source_revision_id, triage_epoch)
        REFERENCES source_revision_triage(source_revision_id, triage_epoch)
        ON DELETE CASCADE
);

CREATE TABLE ocr_job_spec (
    ingest_job_id TEXT PRIMARY KEY NOT NULL,
    source_revision_id TEXT NOT NULL,
    triage_epoch TEXT NOT NULL,
    UNIQUE (source_revision_id, triage_epoch),
    FOREIGN KEY (ingest_job_id) REFERENCES ingest_job(id) ON DELETE CASCADE
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (source_revision_id, triage_epoch)
        REFERENCES source_revision_triage(source_revision_id, triage_epoch)
        ON DELETE RESTRICT
);

CREATE TRIGGER ocr_job_spec_immutable_update
BEFORE UPDATE ON ocr_job_spec
BEGIN
    SELECT RAISE(ABORT, 'immutable OCR job specification');
END;

CREATE TABLE ocr_job_discard (
    ingest_job_id TEXT PRIMARY KEY NOT NULL,
    reason TEXT NOT NULL CHECK (reason = 'source_revision_no_longer_current'),
    discarded_at_seconds INTEGER NOT NULL CHECK (discarded_at_seconds >= 0),
    FOREIGN KEY (ingest_job_id) REFERENCES ocr_job_spec(ingest_job_id) ON DELETE CASCADE
);

CREATE TRIGGER ocr_job_discard_immutable_update
BEFORE UPDATE ON ocr_job_discard
BEGIN
    SELECT RAISE(ABORT, 'immutable OCR job discard');
END;

CREATE TRIGGER ingest_job_requires_exact_ocr_spec
AFTER INSERT ON ingest_job
WHEN (
    NEW.kind = 'ocr_document'
    AND NOT EXISTS (
        SELECT 1 FROM ocr_job_spec WHERE ingest_job_id = NEW.id
    )
) OR (
    NEW.kind <> 'ocr_document'
    AND EXISTS (
        SELECT 1 FROM ocr_job_spec WHERE ingest_job_id = NEW.id
    )
)
BEGIN
    SELECT RAISE(ABORT, 'invalid OCR job specification');
END;

CREATE TABLE resume_version (
    id TEXT PRIMARY KEY NOT NULL,
    document_id TEXT NOT NULL,
    source_revision_id TEXT NOT NULL,
    normalized_text_hash TEXT NOT NULL CHECK (
        length(normalized_text_hash) = 71
        AND substr(normalized_text_hash, 1, 7) = 'sha256:'
        AND substr(normalized_text_hash, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    parse_version TEXT NOT NULL CHECK (length(trim(parse_version)) > 0),
    schema_version TEXT NOT NULL CHECK (length(trim(schema_version)) > 0),
    language_set_json TEXT NOT NULL DEFAULT '[]',
    page_count INTEGER CHECK (page_count IS NULL OR page_count >= 0),
    raw_text TEXT,
    clean_text TEXT CHECK (clean_text IS NULL OR instr(clean_text, char(0)) = 0),
    quality_score REAL CHECK (quality_score IS NULL OR quality_score BETWEEN 0 AND 1),
    UNIQUE (id, document_id),
    UNIQUE (id, document_id, source_revision_id),
    FOREIGN KEY (source_revision_id, document_id)
        REFERENCES source_revision(id, document_id) ON DELETE CASCADE
);

CREATE INDEX resume_version_document_idx
    ON resume_version(document_id, id);

CREATE TRIGGER resume_version_immutable_update
BEFORE UPDATE ON resume_version
BEGIN
    SELECT RAISE(ABORT, 'immutable resume version');
END;

CREATE TABLE resume_version_classification (
    resume_version_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN (
        'resume_candidate', 'non_resume', 'needs_review', 'failed'
    )),
    classifier_epoch TEXT NOT NULL CHECK (
        length(classifier_epoch) BETWEEN 1 AND 64
        AND classifier_epoch NOT GLOB '*[^a-z0-9_]*'
    ),
    classified_at_seconds INTEGER NOT NULL CHECK (classified_at_seconds >= 0),
    review_disposition TEXT NOT NULL CHECK (review_disposition IN ('not_required', 'pending')),
    CHECK (
        (status = 'needs_review' AND review_disposition = 'pending')
        OR (status <> 'needs_review' AND review_disposition = 'not_required')
    ),
    PRIMARY KEY (resume_version_id, classifier_epoch),
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE
);

CREATE TRIGGER resume_version_classification_immutable_update
BEFORE UPDATE ON resume_version_classification
BEGIN
    SELECT RAISE(ABORT, 'immutable resume version classification');
END;

CREATE TABLE resume_version_classification_reason (
    resume_version_id TEXT NOT NULL,
    classifier_epoch TEXT NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal BETWEEN 0 AND 7),
    reason_code TEXT NOT NULL CHECK (reason_code IN (
        'profile_heading', 'experience_heading', 'education_heading', 'skills_heading',
        'career_history_detail', 'invoice_heading', 'invoice_terms', 'meeting_heading',
        'meeting_workflow', 'manual_heading', 'manual_instructions',
        'corroborated_resume_signals', 'corroborated_non_resume_signals',
        'conflicting_signal_families', 'insufficient_signal_families',
        'empty_normalized_text', 'parser_failed'
    )),
    PRIMARY KEY (resume_version_id, classifier_epoch, ordinal),
    UNIQUE (resume_version_id, classifier_epoch, reason_code),
    FOREIGN KEY (resume_version_id, classifier_epoch)
        REFERENCES resume_version_classification(resume_version_id, classifier_epoch)
        ON DELETE CASCADE
);

CREATE TABLE resume_version_candidate (
    resume_version_id TEXT PRIMARY KEY NOT NULL,
    candidate_id TEXT NOT NULL,
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE,
    FOREIGN KEY (candidate_id) REFERENCES candidate(id) ON DELETE CASCADE
);

CREATE INDEX resume_version_candidate_candidate_idx
    ON resume_version_candidate(candidate_id, resume_version_id);

CREATE TRIGGER resume_version_candidate_immutable_update
BEFORE UPDATE ON resume_version_candidate
BEGIN
    SELECT RAISE(ABORT, 'immutable candidate assignment');
END;

CREATE TABLE entity_mention (
    id TEXT PRIMARY KEY NOT NULL,
    resume_version_id TEXT NOT NULL,
    section_id TEXT,
    entity_type TEXT NOT NULL CHECK (
        entity_type IN (
            'name', 'email', 'phone', 'wechat', 'school', 'school_tier',
            'degree', 'major', 'company', 'title', 'education', 'skills',
            'skill', 'certificate', 'date', 'date_range', 'years_experience',
            'location'
        ) OR entity_type LIKE 'other:%'
    ),
    raw_value TEXT NOT NULL,
    normalized_value TEXT,
    span_start INTEGER CHECK (span_start IS NULL OR span_start >= 0),
    span_end INTEGER CHECK (span_end IS NULL OR span_end >= 0),
    confidence REAL NOT NULL CHECK (confidence BETWEEN 0 AND 1),
    extractor TEXT NOT NULL,
    CHECK (span_start IS NULL OR span_end IS NULL OR span_start <= span_end),
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE
);

CREATE INDEX entity_mention_version_idx
    ON entity_mention(resume_version_id, entity_type);
CREATE INDEX entity_mention_type_value_idx
    ON entity_mention(entity_type, normalized_value, confidence);

CREATE TRIGGER entity_mention_immutable_update
BEFORE UPDATE ON entity_mention
BEGIN
    SELECT RAISE(ABORT, 'immutable entity mention');
END;

CREATE TABLE embedding_job_spec (
    ingest_job_id TEXT PRIMARY KEY,
    resume_version_id TEXT NOT NULL,
    model_id TEXT NOT NULL CHECK (
        length(trim(model_id)) > 0
        AND instr(model_id, char(10)) = 0
        AND instr(model_id, char(13)) = 0
        AND instr(model_id, char(9)) = 0
    ),
    dimension INTEGER NOT NULL CHECK (dimension > 0),
    updated_at_seconds INTEGER NOT NULL,
    FOREIGN KEY (ingest_job_id) REFERENCES ingest_job(id) ON DELETE CASCADE,
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX embedding_job_spec_unique_idx
    ON embedding_job_spec(resume_version_id, model_id, dimension);
CREATE INDEX embedding_job_spec_model_idx
    ON embedding_job_spec(model_id, dimension, resume_version_id);

CREATE TABLE candidate_contact_conflict (
    resume_version_id TEXT PRIMARY KEY,
    email_candidate_id TEXT NOT NULL,
    phone_candidate_id TEXT NOT NULL,
    updated_at_seconds INTEGER NOT NULL,
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE CASCADE,
    FOREIGN KEY (email_candidate_id) REFERENCES candidate(id) ON DELETE CASCADE,
    FOREIGN KEY (phone_candidate_id) REFERENCES candidate(id) ON DELETE CASCADE,
    CHECK (email_candidate_id <> phone_candidate_id)
);

CREATE INDEX candidate_contact_conflict_updated_idx
    ON candidate_contact_conflict(updated_at_seconds);

CREATE TRIGGER candidate_contact_conflict_immutable_update
BEFORE UPDATE ON candidate_contact_conflict
BEGIN
    SELECT RAISE(ABORT, 'immutable candidate contact conflict');
END;

CREATE TABLE resume_version_seal (
    resume_version_id TEXT PRIMARY KEY NOT NULL,
    sealed_at_seconds INTEGER NOT NULL CHECK (sealed_at_seconds >= 0),
    entity_mention_count INTEGER NOT NULL CHECK (entity_mention_count BETWEEN 0 AND 256),
    candidate_id TEXT,
    FOREIGN KEY (resume_version_id) REFERENCES resume_version(id) ON DELETE RESTRICT,
    FOREIGN KEY (candidate_id) REFERENCES candidate(id) ON DELETE RESTRICT
);

CREATE TRIGGER resume_version_seal_immutable_update
BEFORE UPDATE ON resume_version_seal
BEGIN
    SELECT RAISE(ABORT, 'immutable resume version seal');
END;

CREATE TRIGGER sealed_entity_mention_insert
BEFORE INSERT ON entity_mention
WHEN EXISTS (
    SELECT 1 FROM resume_version_seal
    WHERE resume_version_id = NEW.resume_version_id
)
BEGIN
    SELECT RAISE(ABORT, 'sealed resume version derived data');
END;

CREATE TRIGGER entity_mention_count_limit
BEFORE INSERT ON entity_mention
WHEN (
    SELECT COUNT(*) FROM entity_mention
    WHERE resume_version_id = NEW.resume_version_id
) >= 256
BEGIN
    SELECT RAISE(ABORT, 'entity mention count exceeds limit');
END;

CREATE TRIGGER sealed_entity_mention_delete
BEFORE DELETE ON entity_mention
WHEN EXISTS (
    SELECT 1 FROM resume_version_seal
    WHERE resume_version_id = OLD.resume_version_id
)
BEGIN
    SELECT RAISE(ABORT, 'sealed resume version derived data');
END;

CREATE TRIGGER sealed_candidate_assignment_insert
BEFORE INSERT ON resume_version_candidate
WHEN EXISTS (
    SELECT 1 FROM resume_version_seal
    WHERE resume_version_id = NEW.resume_version_id
)
BEGIN
    SELECT RAISE(ABORT, 'sealed resume version derived data');
END;

CREATE TRIGGER sealed_candidate_assignment_delete
BEFORE DELETE ON resume_version_candidate
WHEN EXISTS (
    SELECT 1 FROM resume_version_seal
    WHERE resume_version_id = OLD.resume_version_id
)
BEGIN
    SELECT RAISE(ABORT, 'sealed resume version derived data');
END;

CREATE TRIGGER sealed_candidate_conflict_insert
BEFORE INSERT ON candidate_contact_conflict
WHEN EXISTS (
    SELECT 1 FROM resume_version_seal
    WHERE resume_version_id = NEW.resume_version_id
)
BEGIN
    SELECT RAISE(ABORT, 'sealed resume version derived data');
END;

CREATE TRIGGER sealed_candidate_conflict_delete
BEFORE DELETE ON candidate_contact_conflict
WHEN EXISTS (
    SELECT 1 FROM resume_version_seal
    WHERE resume_version_id = OLD.resume_version_id
)
BEGIN
    SELECT RAISE(ABORT, 'sealed resume version derived data');
END;
"#;
