use meta_store::{
    EphemeralMetaStore, ImportProcessingContract, ImportRootKind, ImportRootTaskHeadOutcome,
    ImportScanBudgetKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId,
    ImportTaskPurpose, ImportTaskStatus, OwnedMetaStore, SearchRepairReason, UnixTimestamp,
};

mod support;

#[test]
fn root_pause_is_durable_and_excludes_only_that_root_from_requeue_and_claim() {
    let (_directory, store) = support::owned_store();
    store
        .activate_migration_rebuild_contract(
            &processing_contract(),
            UnixTimestamp::from_unix_seconds(1_800_099_999),
        )
        .unwrap();

    let root_a = "/synthetic/managed-root-a";
    let root_b = "/synthetic/managed-root-b";
    insert_task_with_scope(&store, "completed-a", root_a, ImportTaskStatus::Completed);
    insert_task_with_scope(&store, "completed-b", root_b, ImportTaskStatus::Completed);

    let paused = store
        .pause_import_root(root_a, UnixTimestamp::from_unix_seconds(1_800_100_110))
        .unwrap();
    assert!(paused.changed);
    assert_eq!(paused.cancellation_requests, 0);
    assert_eq!(
        store
            .completed_import_scan_scopes_due_for_requeue(UnixTimestamp::from_unix_seconds(
                1_800_100_120,
            ))
            .unwrap()
            .iter()
            .map(|scope| scope.canonical_root_path.as_str())
            .collect::<Vec<_>>(),
        vec![root_b]
    );

    insert_task_with_scope(&store, "queued-a", root_a, ImportTaskStatus::Queued);
    insert_task_with_scope(&store, "queued-b", root_b, ImportTaskStatus::Queued);
    let claim_at = UnixTimestamp::from_unix_seconds(1_800_100_140);
    let candidate = store
        .import_task_claim_candidate_for_worker_excluding_due_at(claim_at, &[])
        .unwrap()
        .unwrap();
    let claimed = store
        .claim_observed_import_task_for_worker(&candidate, claim_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.root_path, root_b);
    let idempotent_pause = store
        .pause_import_root(root_a, UnixTimestamp::from_unix_seconds(1_800_100_150))
        .unwrap();
    assert!(!idempotent_pause.changed);
    assert_eq!(idempotent_pause.cancellation_requests, 1);

    let resumed = store
        .resume_import_root(
            root_a,
            &ImportTaskId::from_non_secret_parts(&["s800", "catch-up-a"]),
            &processing_contract(),
            UnixTimestamp::from_unix_seconds(1_800_100_160),
        )
        .unwrap();
    assert!(resumed.changed);
    assert!(resumed.catch_up_queued);
    assert!(store.pending_import_task_by_root(root_a).unwrap().is_some());
    let idempotent_resume = store
        .resume_import_root(
            root_a,
            &ImportTaskId::from_non_secret_parts(&["s800", "duplicate-catch-up-a"]),
            &processing_contract(),
            UnixTimestamp::from_unix_seconds(1_800_100_170),
        )
        .unwrap();
    assert!(!idempotent_resume.changed);
    assert!(!idempotent_resume.catch_up_queued);
}

#[test]
fn full_corpus_migration_scope_is_unbounded_without_changing_root_budget() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let contract = processing_contract();
    store
        .activate_migration_rebuild_contract(
            &contract,
            UnixTimestamp::from_unix_seconds(1_800_199_999),
        )
        .unwrap();
    let root = "/synthetic/budgeted-root";
    insert_budgeted_root_configuration(&store, root, &contract);

    let rebuild_task_id = ImportTaskId::from_non_secret_parts(&["s800", "full-corpus-rebuild"]);
    assert!(matches!(
        store
            .enqueue_full_corpus_migration_rebuild_root(
                root,
                &rebuild_task_id,
                &contract,
                UnixTimestamp::from_unix_seconds(1_800_200_010),
            )
            .unwrap(),
        ImportRootTaskHeadOutcome::HeadInserted { .. }
    ));
    let rebuild_scope = store
        .import_scan_scope_by_task_id(&rebuild_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(rebuild_scope.scan_budget_kind, None);
    assert_eq!(rebuild_scope.scan_budget_limit, None);
    assert_eq!(rebuild_scope.scan_budget_observed, None);
    assert!(!rebuild_scope.scan_budget_exhausted);
    assert_eq!(
        store.import_task_purpose(&rebuild_task_id).unwrap(),
        ImportTaskPurpose::MigrationRebuildFullCorpus
    );

    store
        .cancel_import_task(
            &rebuild_task_id,
            UnixTimestamp::from_unix_seconds(1_800_200_011),
        )
        .unwrap();
    store
        .block_migration_rebuild(
            SearchRepairReason::RuntimeInvariant,
            UnixTimestamp::from_unix_seconds(1_800_200_012),
        )
        .unwrap();
    let catch_up_task_id = ImportTaskId::from_non_secret_parts(&["s800", "configured-catch-up"]);
    assert!(matches!(
        store
            .enqueue_authorized_import_root(
                root,
                &catch_up_task_id,
                &contract,
                UnixTimestamp::from_unix_seconds(1_800_200_013),
            )
            .unwrap(),
        ImportRootTaskHeadOutcome::HeadInserted { .. }
    ));
    let catch_up_scope = store
        .import_scan_scope_by_task_id(&catch_up_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        catch_up_scope.scan_budget_kind,
        Some(ImportScanBudgetKind::Files)
    );
    assert_eq!(catch_up_scope.scan_budget_limit, Some(1));
    assert_eq!(catch_up_scope.scan_budget_observed, Some(0));
    assert!(!catch_up_scope.scan_budget_exhausted);
    assert_eq!(
        store.import_task_purpose(&catch_up_task_id).unwrap(),
        ImportTaskPurpose::ConfiguredCatchUp
    );
}

fn insert_task_with_scope(
    store: &OwnedMetaStore,
    label: &str,
    root: &str,
    status: ImportTaskStatus,
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_100_000);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s800", label]),
        root_path: root.to_string(),
        status,
        queued_at: now,
        started_at: (status != ImportTaskStatus::Queued)
            .then(|| UnixTimestamp::from_unix_seconds(1_800_100_001)),
        finished_at: matches!(
            status,
            ImportTaskStatus::Completed
                | ImportTaskStatus::FailedRetryable
                | ImportTaskStatus::FailedPermanent
        )
        .then(|| UnixTimestamp::from_unix_seconds(1_800_100_002)),
        updated_at: now,
    };
    support::insert_import_task_owned(store, &task);
}

fn insert_budgeted_root_configuration(
    store: &EphemeralMetaStore,
    root: &str,
    contract: &ImportProcessingContract,
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_200_000);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s800", "budgeted-root"]),
        root_path: root.to_string(),
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
        requested_root_path: root.to_string(),
        canonical_root_path: root.to_string(),
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: Some(ImportScanBudgetKind::Files),
        scan_budget_limit: Some(1),
        scan_budget_observed: Some(0),
        scan_budget_exhausted: false,
        updated_at: now,
    };
    store
        .insert_import_task_with_scan_scope(&task, &scope, contract)
        .unwrap();
    assert!(store
        .cancel_import_task(&task.id, UnixTimestamp::from_unix_seconds(1_800_200_001),)
        .unwrap());
}

fn processing_contract() -> ImportProcessingContract {
    support::processing_contract()
}
