use rusqlite::params;

use crate::{
    EphemeralMetaStore, ImportProcessingContract, ImportRootKind, ImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus,
    MigrationRebuildContractActivation, UnixTimestamp, WriterAuthorityHealthState,
    WriterContractTransitionOutcome, CLASSIFIER_EPOCH,
};

#[test]
fn online_transition_inserts_transition_and_campaign_with_digest_ids() {
    let store = ready_store_with_contract("parser-v1");
    let desired = processing_contract("parser-pdfium-v2");
    let now = UnixTimestamp::from_unix_seconds(1_900_200_000);
    assert_eq!(
        store
            .complete_online_writer_transition(&desired, now)
            .unwrap(),
        WriterContractTransitionOutcome::TargetCommitted
    );
    let transition_count: i64 = store
        .connection
        .borrow()
        .query_row(
            "SELECT COUNT(*) FROM writer_contract_transition",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(transition_count, 1);
    let (transition_id, target_id, phase, campaign_id): (String, String, String, Option<String>) =
        store
            .connection
            .borrow()
            .query_row(
                "SELECT transition_id, target_contract_id, phase, campaign_id
                 FROM writer_contract_transition",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
    assert_eq!(transition_id.len(), 71);
    assert!(transition_id.starts_with("sha256:"));
    assert_eq!(target_id, desired.id().as_str());
    assert_eq!(target_id.len(), 71);
    assert_eq!(phase, "writer_ready");
    let campaign_id = campaign_id.expect("campaign materialized");
    assert_eq!(campaign_id.len(), 71);
    let campaign_count: i64 = store
        .connection
        .borrow()
        .query_row(
            "SELECT COUNT(*) FROM reprocessing_campaign WHERE campaign_id = ?1",
            params![campaign_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(campaign_count, 1);
    let snapshot = store.writer_authority_snapshot().unwrap();
    assert_eq!(snapshot.health_state, WriterAuthorityHealthState::Ready);
    assert_eq!(
        snapshot
            .committed_contract_id
            .as_ref()
            .map(|id| id.as_str()),
        Some(desired.id().as_str())
    );
    assert_eq!(
        snapshot.last_completed_transition_id.as_deref(),
        Some(transition_id.as_str())
    );
    assert_eq!(
        store
            .complete_online_writer_transition(&desired, now)
            .unwrap(),
        WriterContractTransitionOutcome::AlreadyActive
    );
}

#[test]
fn runtime_unavailable_latch_clears_when_probe_recovers() {
    let store = ready_store_with_contract("parser-v1");
    let now = UnixTimestamp::from_unix_seconds(1_900_200_050);
    store.mark_writer_runtime_unavailable(now).unwrap();
    assert_eq!(
        store.writer_authority_snapshot().unwrap().health_state,
        WriterAuthorityHealthState::Unavailable
    );
    assert!(!store.public_writer_claims_admitted().unwrap());
    assert!(store
        .reconcile_writer_runtime_availability(/*runtime_healthy*/ true, now)
        .unwrap());
    assert_eq!(
        store.writer_authority_snapshot().unwrap().health_state,
        WriterAuthorityHealthState::Ready
    );
    assert!(store.public_writer_claims_admitted().unwrap());
}

#[test]
fn unsupported_transition_latch_blocks_admission_until_the_next_recheck() {
    let store = ready_store_with_contract("parser-v1");
    let now = UnixTimestamp::from_unix_seconds(1_900_200_060);
    assert!(store.mark_writer_unsupported_transition(now).unwrap());
    let unavailable = store.writer_authority_snapshot().unwrap();
    assert_eq!(
        unavailable.health_state,
        WriterAuthorityHealthState::Unavailable
    );
    assert_eq!(
        unavailable.health_reason.as_deref(),
        Some("unsupported_transition")
    );
    assert!(!store.public_writer_claims_admitted().unwrap());
    assert!(!store.mark_writer_unsupported_transition(now).unwrap());

    assert!(store.clear_writer_unsupported_transition(now).unwrap());
    let ready = store.writer_authority_snapshot().unwrap();
    assert_eq!(ready.health_state, WriterAuthorityHealthState::Ready);
    assert_eq!(ready.health_reason, None);
    assert!(store.public_writer_claims_admitted().unwrap());
    assert!(!store.clear_writer_unsupported_transition(now).unwrap());
}

#[test]
fn hard_cut_syncs_writer_authority_committed_contract() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let contract = processing_contract("parser-v1");
    let now = UnixTimestamp::from_unix_seconds(1_900_199_000);
    assert_eq!(
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap(),
        MigrationRebuildContractActivation::Activated
    );
    let snapshot = store.writer_authority_snapshot().unwrap();
    assert_eq!(snapshot.health_state, WriterAuthorityHealthState::Ready);
    assert_eq!(
        snapshot
            .committed_contract_id
            .as_ref()
            .map(|id| id.as_str()),
        Some(contract.id().as_str())
    );
}

#[test]
fn online_transition_retires_old_queued_and_rebuilds_under_target() {
    let store = ready_store_with_contract("parser-v1");
    let previous = processing_contract("parser-v1");
    let now = UnixTimestamp::from_unix_seconds(1_900_200_100);
    let root_path = "synthetic/import/writer-rebuild";
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["writer-rebuild", "seed"]),
        root_path: root_path.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: root_path.to_string(),
        canonical_root_path: root_path.to_string(),
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: None,
        scan_budget_limit: None,
        scan_budget_observed: None,
        scan_budget_exhausted: false,
        updated_at: now,
    };
    store
        .insert_import_task_with_scan_scope(&task, &scope, &previous)
        .unwrap();

    let desired = processing_contract("parser-pdfium-v2");
    assert_eq!(
        store
            .complete_online_writer_transition(&desired, now)
            .unwrap(),
        WriterContractTransitionOutcome::TargetCommitted
    );

    let cancelled: i64 = store
        .connection
        .borrow()
        .query_row(
            "SELECT COUNT(*) FROM import_task_cancellation WHERE import_task_id = ?1",
            params![task.id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cancelled, 1);

    let rebuilt_binding: String = store
        .connection
        .borrow()
        .query_row(
            "SELECT binding.processing_contract_id
             FROM import_task AS task
             JOIN import_task_contract_binding AS binding
               ON binding.import_task_id = task.id
             WHERE task.root_path = ?1
               AND task.status = 'queued'
               AND NOT EXISTS (
                   SELECT 1 FROM import_task_cancellation AS cancellation
                   WHERE cancellation.import_task_id = task.id
               )",
            params![root_path],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(rebuilt_binding, desired.id().as_str());
}

#[test]
fn claim_fence_blocks_ordinary_claims_while_transitioning() {
    let store = ready_store_with_contract("parser-v1");
    let previous = processing_contract("parser-v1");
    let now = UnixTimestamp::from_unix_seconds(1_900_200_200);
    let root_path = "synthetic/import/writer-fence";
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["writer-fence", "seed"]),
        root_path: root_path.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: root_path.to_string(),
        canonical_root_path: root_path.to_string(),
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: None,
        scan_budget_limit: None,
        scan_budget_observed: None,
        scan_budget_exhausted: false,
        updated_at: now,
    };
    store
        .insert_import_task_with_scan_scope(&task, &scope, &previous)
        .unwrap();

    // Force a transitioning authority without completing the machine.
    let transition_id = format!("sha256:{}", "c".repeat(64));
    store
        .connection
        .borrow()
        .execute(
            "INSERT INTO writer_contract_transition (
                transition_id, source_contract_id, target_contract_id,
                desired_product_version, desired_schema_version,
                source_generation, source_visible_epoch, phase, attempt,
                claim_fence_epoch, running_task_count, queued_task_count,
                scheduled_task_count, created_at_seconds, updated_at_seconds
             ) VALUES (
                ?1, ?2, ?2, '0.1.9', 34, 'synthetic-generation', 0,
                'claims_fenced', 1, 1, 0, 0, 0, ?3, ?3
             )",
            params![transition_id, previous.id().as_str(), now.as_unix_seconds()],
        )
        .unwrap();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE writer_authority_state
             SET health_state = 'transitioning',
                 health_reason = 'transition_in_progress',
                 transition_phase = 'claims_fenced',
                 active_transition_id = ?1,
                 claim_fence_epoch = 1,
                 updated_at_seconds = ?2
             WHERE state_key = 'default'",
            params![transition_id, now.as_unix_seconds()],
        )
        .unwrap();
    assert!(!store.public_writer_claims_admitted().unwrap());
    assert!(store
        .claim_observed_import_task_for_worker(&task, now)
        .unwrap()
        .is_none());
}

fn ready_store_with_contract(parser: &str) -> EphemeralMetaStore {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let contract = processing_contract(parser);
    let now = UnixTimestamp::from_unix_seconds(1_900_199_000);
    assert_eq!(
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap(),
        MigrationRebuildContractActivation::Activated
    );
    force_generation_bearing_state(&store, "ready", None);
    store
}

fn force_generation_bearing_state(
    store: &EphemeralMetaStore,
    service_state: &str,
    repair_reason: Option<&str>,
) {
    let connection = store.connection.borrow();
    connection
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER search_projection_head_change_requires_commit_guard;
             DROP TRIGGER ready_projection_head_matches_journal;",
        )
        .unwrap();
    connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = ?1, generation = 'synthetic-generation',
                 repair_reason = ?2
             WHERE state_key = 'default'",
            params![service_state, repair_reason],
        )
        .unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .unwrap();
}

fn processing_contract(parser: &str) -> ImportProcessingContract {
    ImportProcessingContract::new(parser, "ocr-parser-v1", "schema-v28", CLASSIFIER_EPOCH).unwrap()
}
