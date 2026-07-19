#![allow(dead_code)]

use import_pipeline::{
    current_import_processing_contract, finalize_migration_rebuild,
    prepare_migration_rebuild_artifacts, ImportOptions, SearchPublicationVectorization,
};
use meta_store::{
    ImportProcessingContract, ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask,
    ImportTaskStatus, OwnedMetaStore, SearchProjectionServiceState, UnixTimestamp,
};

pub fn default_processing_contract() -> ImportProcessingContract {
    current_import_processing_contract(&ImportOptions::default()).unwrap()
}

pub fn activate_default_processing_contract(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
) -> ImportProcessingContract {
    let contract = default_processing_contract();
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    contract
}

pub fn empty_import_scan_scope(task: &ImportTask) -> ImportScanScope {
    ImportScanScope {
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
        updated_at: task.updated_at,
    }
}

pub fn insert_import_task(store: &OwnedMetaStore, task: &ImportTask) -> ImportProcessingContract {
    insert_import_task_with_scope(store, task, &empty_import_scan_scope(task))
}

pub fn insert_import_task_with_scope(
    store: &OwnedMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
) -> ImportProcessingContract {
    assert_ne!(task.status, ImportTaskStatus::Completed);
    let contract = activate_default_processing_contract(store, task.queued_at);
    prepare_migration_rebuild_artifacts(store, task.queued_at).unwrap();
    finalize_migration_rebuild(
        store,
        task.queued_at,
        &contract,
        &SearchPublicationVectorization::default(),
    )
    .unwrap();
    assert_eq!(
        store.search_projection_state().unwrap().service_state,
        SearchProjectionServiceState::Ready
    );
    let queued = ImportTask {
        id: task.id.clone(),
        root_path: task.root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: task.queued_at,
        started_at: None,
        finished_at: None,
        updated_at: task.queued_at,
    };
    let mut initial_scope = scope.clone();
    initial_scope.updated_at = task.queued_at;
    store
        .insert_import_task_with_scan_scope(&queued, &initial_scope, &contract)
        .unwrap();
    if task.status != ImportTaskStatus::Queued {
        let running_at = task.started_at.unwrap_or(task.updated_at);
        let claimed = store
            .claim_observed_import_task_for_worker(&queued, running_at)
            .unwrap()
            .unwrap();
        if task.status == ImportTaskStatus::Running && task.updated_at != claimed.updated_at {
            assert!(store
                .heartbeat_running_import_task(&task.id, task.updated_at)
                .unwrap());
        }
    }
    if matches!(
        task.status,
        ImportTaskStatus::FailedRetryable | ImportTaskStatus::FailedPermanent
    ) {
        store
            .update_import_task_status(&task.id, task.status, task.updated_at)
            .unwrap();
    }
    let mut persisted_scope = scope.clone();
    persisted_scope.updated_at = task.updated_at;
    store.upsert_import_scan_scope(&persisted_scope).unwrap();
    contract
}
