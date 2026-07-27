use rusqlite::params;

use crate::{
    EphemeralMetaStore, ImportProcessingContract, ImportRootKind, ImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus,
    MigrationRebuildContractActivation, UnixTimestamp, WriterAuthorityHealthState,
    WriterContractTransitionOutcome, WriterTransitionPhase, CLASSIFIER_EPOCH,
};

#[test]
fn restart_after_claims_fenced_continues_without_double_commit() {
    let store = ready_store_with_contract("parser-v1");
    let desired = processing_contract("parser-pdfium-v2");
    let now = UnixTimestamp::from_unix_seconds(1_900_300_000);
    // Begin a transition and stop at claims_fenced by inserting durable state.
    store.insert_import_processing_contract(&desired).unwrap();
    let transition_id = format!("sha256:{}", "a".repeat(64));
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
                ?1, ?2, ?3, '0.1.9', 34, 'synthetic-generation', 0,
                'claims_fenced', 1, 1, 0, 0, 0, ?4, ?4
             )",
            params![
                transition_id,
                processing_contract("parser-v1").id().as_str(),
                desired.id().as_str(),
                now.as_unix_seconds(),
            ],
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
                 desired_contract_id = ?2,
                 updated_at_seconds = ?3
             WHERE state_key = 'default'",
            params![transition_id, desired.id().as_str(), now.as_unix_seconds()],
        )
        .unwrap();

    assert_eq!(
        store
            .recover_writer_contract_transition_on_open(&desired, now)
            .unwrap(),
        WriterContractTransitionOutcome::TargetCommitted
    );
    let phase: String = store
        .connection
        .borrow()
        .query_row(
            "SELECT phase FROM writer_contract_transition WHERE transition_id = ?1",
            params![transition_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(phase, "writer_ready");
    let commit_count: i64 = store
        .connection
        .borrow()
        .query_row(
            "SELECT COUNT(*) FROM writer_contract_transition
             WHERE target_contract_id = ?1 AND phase = 'writer_ready'",
            params![desired.id().as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(commit_count, 1);
    assert_eq!(
        store
            .recover_writer_contract_transition_on_open(&desired, now)
            .unwrap(),
        WriterContractTransitionOutcome::AlreadyActive
    );
    let commit_count_after: i64 = store
        .connection
        .borrow()
        .query_row(
            "SELECT COUNT(*) FROM writer_contract_transition
             WHERE target_contract_id = ?1 AND phase = 'writer_ready'",
            params![desired.id().as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(commit_count_after, 1);
}

#[test]
fn running_owner_blocks_target_commit_across_restart() {
    let store = ready_store_with_contract("parser-v1");
    let previous = processing_contract("parser-v1");
    let desired = processing_contract("parser-pdfium-v2");
    let now = UnixTimestamp::from_unix_seconds(1_900_300_100);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["crash-running", "seed"]),
        root_path: "synthetic/import/crash-running".to_string(),
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
        requested_root_path: task.root_path.clone(),
        canonical_root_path: task.root_path.clone(),
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
    store
        .claim_observed_import_task_for_worker(&task, now)
        .unwrap()
        .unwrap();

    assert_eq!(
        store
            .complete_online_writer_transition(&desired, now)
            .unwrap(),
        WriterContractTransitionOutcome::BlockedByRunningOwner
    );
    assert_eq!(
        store
            .active_import_processing_contract()
            .unwrap()
            .unwrap()
            .id()
            .as_str(),
        previous.id().as_str()
    );
    let snapshot = store.writer_authority_snapshot().unwrap();
    assert_eq!(
        snapshot.health_state,
        WriterAuthorityHealthState::Transitioning
    );
    assert_eq!(
        snapshot.transition_phase,
        Some(WriterTransitionPhase::Observed)
    );
}

fn ready_store_with_contract(parser: &str) -> EphemeralMetaStore {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let contract = processing_contract(parser);
    let now = UnixTimestamp::from_unix_seconds(1_900_299_000);
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
