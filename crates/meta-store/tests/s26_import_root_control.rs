use meta_store::{
    ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus,
    MetaStore, UnixTimestamp,
};

#[test]
fn root_pause_is_durable_and_excludes_only_that_root_from_requeue_and_claim() {
    let store = MetaStore::open_in_memory().unwrap();
    assert_eq!(
        store.run_migrations().unwrap().applied_versions().last(),
        Some(&27)
    );

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
    let claimed = store
        .claim_next_import_task_for_worker(UnixTimestamp::from_unix_seconds(1_800_100_140))
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
            UnixTimestamp::from_unix_seconds(1_800_100_170),
        )
        .unwrap();
    assert!(!idempotent_resume.changed);
    assert!(!idempotent_resume.catch_up_queued);
}

fn insert_task_with_scope(store: &MetaStore, label: &str, root: &str, status: ImportTaskStatus) {
    let task_id = ImportTaskId::from_non_secret_parts(&["s800", label]);
    let now = UnixTimestamp::from_unix_seconds(1_800_100_000);
    let finished_at = (status == ImportTaskStatus::Completed).then_some(now);
    store
        .insert_import_task_with_scan_scope(
            &ImportTask {
                id: task_id.clone(),
                root_path: root.to_string(),
                status,
                queued_at: now,
                started_at: finished_at,
                finished_at,
                updated_at: now,
            },
            &ImportScanScope {
                import_task_id: task_id,
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
                scan_budget_kind: None,
                scan_budget_limit: None,
                scan_budget_observed: None,
                scan_budget_exhausted: false,
                updated_at: now,
            },
        )
        .unwrap();
}
