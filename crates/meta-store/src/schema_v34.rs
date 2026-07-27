pub(super) const VERSION: u32 = 34;

/// Additive writer-authority and reprocessing-campaign tables for online
/// processing-contract transitions. Applied only through the continuous COW
/// forward-migration registry or empty-store schema history.
///
/// Contract and opaque digests use the same identity as
/// `import_processing_contract.id`: `sha256:` + 64 lowercase hex (71 chars).
pub(super) const SCHEMA: &str = r#"
CREATE TABLE writer_contract_transition (
    transition_id TEXT PRIMARY KEY NOT NULL CHECK (
        length(transition_id) = 71
        AND substr(transition_id, 1, 7) = 'sha256:'
        AND substr(transition_id, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    source_contract_id TEXT,
    target_contract_id TEXT NOT NULL CHECK (
        length(target_contract_id) = 71
        AND substr(target_contract_id, 1, 7) = 'sha256:'
        AND substr(target_contract_id, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    desired_product_version TEXT NOT NULL CHECK (
        length(desired_product_version) > 0
        AND length(desired_product_version) <= 64
    ),
    desired_schema_version INTEGER NOT NULL CHECK (desired_schema_version >= 34),
    source_generation TEXT,
    source_visible_epoch INTEGER NOT NULL CHECK (source_visible_epoch >= 0),
    phase TEXT NOT NULL CHECK (
        phase IN (
            'observed',
            'claims_fenced',
            'workers_quiesced',
            'target_committed',
            'writer_ready'
        )
    ),
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt >= 1),
    claim_fence_epoch INTEGER NOT NULL DEFAULT 0 CHECK (claim_fence_epoch >= 0),
    running_task_count INTEGER NOT NULL DEFAULT 0 CHECK (running_task_count >= 0),
    queued_task_count INTEGER NOT NULL DEFAULT 0 CHECK (queued_task_count >= 0),
    scheduled_task_count INTEGER NOT NULL DEFAULT 0 CHECK (scheduled_task_count >= 0),
    failure_class TEXT CHECK (
        failure_class IS NULL
        OR failure_class IN (
            'blocked_by_running_owner',
            'persisted_state_invalid',
            'unsupported_transition',
            'runtime_unavailable'
        )
    ),
    retryable INTEGER NOT NULL DEFAULT 0 CHECK (retryable IN (0, 1)),
    retry_after_seconds INTEGER CHECK (
        retry_after_seconds IS NULL OR retry_after_seconds >= 0
    ),
    campaign_id TEXT,
    target_publication_witness TEXT,
    created_at_seconds INTEGER NOT NULL CHECK (created_at_seconds >= 0),
    updated_at_seconds INTEGER NOT NULL CHECK (
        updated_at_seconds >= created_at_seconds
    ),
    completed_at_seconds INTEGER CHECK (
        completed_at_seconds IS NULL
        OR completed_at_seconds >= created_at_seconds
    ),
    FOREIGN KEY (source_contract_id)
        REFERENCES import_processing_contract(id),
    FOREIGN KEY (target_contract_id)
        REFERENCES import_processing_contract(id)
);

CREATE TABLE writer_authority_state (
    state_key TEXT PRIMARY KEY NOT NULL CHECK (state_key = 'default'),
    health_state TEXT NOT NULL CHECK (
        health_state IN ('ready', 'transitioning', 'unavailable', 'blocked')
    ),
    health_reason TEXT,
    transition_phase TEXT CHECK (
        transition_phase IS NULL
        OR transition_phase IN (
            'observed',
            'claims_fenced',
            'workers_quiesced',
            'target_committed',
            'writer_ready'
        )
    ),
    active_transition_id TEXT,
    claim_fence_epoch INTEGER NOT NULL DEFAULT 0 CHECK (claim_fence_epoch >= 0),
    committed_contract_id TEXT,
    desired_contract_id TEXT,
    updated_at_seconds INTEGER NOT NULL CHECK (updated_at_seconds >= 0),
    FOREIGN KEY (active_transition_id)
        REFERENCES writer_contract_transition(transition_id),
    FOREIGN KEY (committed_contract_id)
        REFERENCES import_processing_contract(id),
    FOREIGN KEY (desired_contract_id)
        REFERENCES import_processing_contract(id)
);

INSERT INTO writer_authority_state (
    state_key,
    health_state,
    health_reason,
    transition_phase,
    active_transition_id,
    claim_fence_epoch,
    committed_contract_id,
    desired_contract_id,
    updated_at_seconds
) VALUES (
    'default',
    'ready',
    NULL,
    NULL,
    NULL,
    0,
    NULL,
    NULL,
    0
);

CREATE TABLE reprocessing_campaign (
    campaign_id TEXT PRIMARY KEY NOT NULL CHECK (
        length(campaign_id) = 71
        AND substr(campaign_id, 1, 7) = 'sha256:'
        AND substr(campaign_id, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    transition_id TEXT NOT NULL,
    target_contract_id TEXT NOT NULL CHECK (
        length(target_contract_id) = 71
        AND substr(target_contract_id, 1, 7) = 'sha256:'
        AND substr(target_contract_id, 8) NOT GLOB '*[^0-9a-f]*'
    ),
    affected_domain TEXT NOT NULL CHECK (
        affected_domain IN (
            'pdf_root_rescan',
            'ocr_requeue',
            'classifier_reclassify',
            'derived_rebuild',
            'unsupported'
        )
    ),
    state TEXT NOT NULL CHECK (
        state IN ('planned', 'queued', 'running', 'complete', 'partial', 'cancelled')
    ),
    created_at_seconds INTEGER NOT NULL CHECK (created_at_seconds >= 0),
    updated_at_seconds INTEGER NOT NULL CHECK (
        updated_at_seconds >= created_at_seconds
    ),
    completed_at_seconds INTEGER CHECK (
        completed_at_seconds IS NULL
        OR completed_at_seconds >= created_at_seconds
    ),
    FOREIGN KEY (transition_id)
        REFERENCES writer_contract_transition(transition_id),
    FOREIGN KEY (target_contract_id)
        REFERENCES import_processing_contract(id)
);

CREATE INDEX reprocessing_campaign_transition_idx
    ON reprocessing_campaign(transition_id, state);

ALTER TABLE pdf_reprocess_job
    ADD COLUMN processing_contract_id TEXT
        REFERENCES import_processing_contract(id);

ALTER TABLE pdf_reprocess_job
    ADD COLUMN campaign_id TEXT
        REFERENCES reprocessing_campaign(campaign_id);
"#;
