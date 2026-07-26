use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use import_pipeline::{
    finalize_migration_rebuild, prepare_migration_rebuild_artifacts, ImportPipelineErrorClass,
    PipelineRunControl,
};
use meta_store::{
    ClassificationStatus, ContentDigest, CurrentClassifierEpoch, Document, DocumentId,
    DocumentStatus, FileExtension, ImportProcessingContract, ImportRootKind, ImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus, IngestJobId, IngestJobStatus,
    MetaStoreErrorClass, OwnedMetaStore, ReasonCode, SearchProjectionServiceState,
    SearchRepairReason, SourceRevision, SourceRevisionTriage, UnixTimestamp, CLASSIFIER_EPOCH,
};

use crate::daemon_policy::IPC_METADATA_READ_ATTEMPTS;
use crate::import_worker::{
    run_import_worker_once_with_retry_due, should_requeue_interrupted_import,
};
use crate::ipc::routes::status::{
    projection_query_error, status_json_with, unavailable_status_json,
};
use crate::ocr_worker::run_ocr_worker_once;
use crate::run_options::RunOptions;
use crate::store_access::open_owned_store;
use crate::worker_runtime::run_fault_priority_gate;
use crate::{import_processing, ipc};

fn open_test_store(data_dir: &std::path::Path) -> OwnedMetaStore {
    let owner = import_processing::acquire_owner(data_dir).unwrap();
    open_owned_store(&owner).unwrap()
}

#[test]
fn reported_artifact_fault_completes_before_lower_priority_work_enters() {
    let phase = AtomicUsize::new(0);
    let (lower_entered_sender, lower_entered_receiver) = mpsc::sync_channel(1);
    let (lower_release_sender, lower_release_receiver) = mpsc::sync_channel(1);

    std::thread::scope(|scope| {
        let worker_phase = &phase;
        let worker = scope.spawn(move || {
            run_fault_priority_gate(
                || {
                    assert_eq!(worker_phase.swap(1, Ordering::SeqCst), 0);
                    Ok(true)
                },
                |repaired_reported_fault| {
                    assert!(repaired_reported_fault);
                    assert_eq!(worker_phase.load(Ordering::SeqCst), 1);
                    lower_entered_sender.send(()).unwrap();
                    lower_release_receiver.recv().unwrap();
                    worker_phase.store(2, Ordering::SeqCst);
                    Ok(())
                },
            )
        });

        lower_entered_receiver.recv().unwrap();
        assert_eq!(phase.load(Ordering::SeqCst), 1);
        lower_release_sender.send(()).unwrap();
        worker.join().unwrap().unwrap();
    });
    assert_eq!(phase.load(Ordering::SeqCst), 2);
}

#[test]
fn durable_user_cancellation_is_never_requeued_as_lifecycle_interruption() {
    assert!(!should_requeue_interrupted_import(
        ImportPipelineErrorClass::Cancelled,
        true,
        true,
    ));
    assert!(should_requeue_interrupted_import(
        ImportPipelineErrorClass::Cancelled,
        true,
        false,
    ));
    assert!(should_requeue_interrupted_import(
        ImportPipelineErrorClass::Interrupted,
        true,
        false,
    ));
    assert!(!should_requeue_interrupted_import(
        ImportPipelineErrorClass::Cancelled,
        false,
        false,
    ));
}

#[test]
fn metadata_unavailable_status_keeps_process_and_service_health() {
    let body = unavailable_status_json();
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["schema_version"], "daemon.status.v3");
    assert_eq!(value["process_state"], "ready");
    assert_eq!(value["core"]["state"], "degraded");
    assert_eq!(value["core"]["reason"], "metadata_unavailable");
    assert_eq!(value["capabilities"]["detail"]["state"], "blocked");
    assert_eq!(value["error"]["code"], "SERVICE_BLOCKED");
    assert!(value["ipc"].is_object());
    assert!(value["indexed_documents"].is_null());
}

#[test]
fn worker_cancels_ready_task_bound_to_a_different_processing_contract() {
    let data_dir = std::env::temp_dir().join(format!(
        "resume-ir-daemon-contract-mismatch-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&data_dir).unwrap();
    let store = open_test_store(&data_dir);
    let options = RunOptions::default();
    let contract = import_processing::current_contract(&options).unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_280_000);
    import_processing::activate_contract(&store, &contract, now).unwrap();
    prepare_migration_rebuild_artifacts(&store, now, &PipelineRunControl::default()).unwrap();
    finalize_migration_rebuild(
        &store,
        now,
        &contract,
        &options.search_vectorization,
        &PipelineRunControl::default(),
    )
    .unwrap();

    let wrong_contract = ImportProcessingContract::new(
        "synthetic-wrong-primary-v28",
        "synthetic-wrong-ocr-v28",
        contract.derived_schema_version(),
        contract.classifier_epoch(),
    )
    .unwrap();
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["daemon-wrong-contract"]),
        root_path: "/synthetic/wrong-contract".to_string(),
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
        .insert_import_task_with_scan_scope(&task, &scope, &wrong_contract)
        .unwrap();

    let summary = run_import_worker_once_with_retry_due(
        &data_dir,
        &store,
        &options,
        &contract,
        now,
        PipelineRunControl::default(),
        || true,
    )
    .unwrap();
    assert_eq!(summary.failed, 1);
    assert!(store.is_import_task_cancelled(&task.id).unwrap());
    assert_eq!(
        store
            .import_task_processing_contract_id(&task.id)
            .unwrap()
            .as_ref(),
        Some(wrong_contract.id())
    );

    drop(store);
    let _ = fs::remove_dir_all(data_dir);
}

#[test]
fn import_capability_loss_between_candidate_and_claim_preserves_task_and_projection_head() {
    let data_dir = std::env::temp_dir().join(format!(
        "resume-ir-daemon-import-claim-gate-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&data_dir).unwrap();
    let store = open_test_store(&data_dir);
    let options = RunOptions::default();
    let contract = import_processing::current_contract(&options).unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_280_100);
    import_processing::activate_contract(&store, &contract, now).unwrap();
    prepare_migration_rebuild_artifacts(&store, now, &PipelineRunControl::default()).unwrap();
    finalize_migration_rebuild(
        &store,
        now,
        &contract,
        &options.search_vectorization,
        &PipelineRunControl::default(),
    )
    .unwrap();
    let projection_before = store.search_projection_state().unwrap();
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["daemon-capability-loss-before-claim"]),
        root_path: data_dir.to_string_lossy().into_owned(),
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
        .insert_import_task_with_scan_scope(&task, &scope, &contract)
        .unwrap();
    let probes = AtomicUsize::new(0);

    let summary = run_import_worker_once_with_retry_due(
        &data_dir,
        &store,
        &options,
        &contract,
        now,
        PipelineRunControl::default(),
        || probes.fetch_add(1, Ordering::SeqCst) == 0,
    )
    .unwrap();

    assert!(!summary.has_activity());
    assert!(probes.load(Ordering::SeqCst) >= 2);
    assert_eq!(store.import_task_by_id(&task.id).unwrap(), Some(task));
    assert_eq!(store.search_projection_state().unwrap(), projection_before);

    drop(store);
    let _ = fs::remove_dir_all(data_dir);
}

#[test]
fn runtime_metadata_read_failure_returns_status_v3_with_blocked_capabilities() {
    let mut attempts = 0;
    let body = status_json_with(|| {
        attempts += 1;
        Err(MetaStoreErrorClass::Storage)
    });
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(attempts, IPC_METADATA_READ_ATTEMPTS);
    assert_eq!(value["schema_version"], "daemon.status.v3");
    assert_eq!(value["process_state"], "ready");
    assert_eq!(value["core"]["state"], "degraded");
    assert_eq!(value["core"]["reason"], "metadata_unavailable");
    assert_eq!(value["capabilities"]["keyword_search"]["state"], "blocked");
    assert_eq!(value["error"]["code"], "SERVICE_BLOCKED");
}

#[test]
fn projection_state_gates_query_routes_with_fixed_codes() {
    assert_eq!(
        projection_query_error(Some(SearchProjectionServiceState::Ready)),
        None
    );
    assert_eq!(
        projection_query_error(Some(SearchProjectionServiceState::Repairing)),
        Some(ipc::ServiceErrorCode::Repairing)
    );
    assert_eq!(
        projection_query_error(Some(SearchProjectionServiceState::RepairBlocked)),
        Some(ipc::ServiceErrorCode::QueryServiceUnavailable)
    );
    assert_eq!(
        projection_query_error(None),
        Some(ipc::ServiceErrorCode::MetadataUnavailable)
    );
}

#[test]
fn ocr_claim_requires_both_ready_projection_and_an_attested_runtime() {
    let data_dir = std::env::temp_dir().join(format!(
        "resume-ir-daemon-ocr-migration-gate-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&data_dir).unwrap();
    let store = open_test_store(&data_dir);
    let now = UnixTimestamp::from_unix_seconds(1_800_281_000);
    let job_id = enqueue_ocr_job_for_worker_gate(&data_dir, &store, now, "repairing");
    let options = RunOptions {
        ocr_command: Some(data_dir.join("unused-ocr-command")),
        ..RunOptions::default()
    };

    let repairing_summary = run_ocr_worker_once(&data_dir, &store, &options, || true).unwrap();
    assert!(!repairing_summary.paused);
    assert!(!repairing_summary.has_activity());
    let still_queued = store.ingest_job_by_id(&job_id).unwrap().unwrap();
    assert_eq!(still_queued.status, IngestJobStatus::Queued);
    assert_eq!(still_queued.attempt_count, 0);

    let contract = import_processing::current_contract(&options).unwrap();
    import_processing::activate_contract(&store, &contract, now).unwrap();
    prepare_migration_rebuild_artifacts(&store, now, &PipelineRunControl::default()).unwrap();
    finalize_migration_rebuild(
        &store,
        now,
        &contract,
        &options.search_vectorization,
        &PipelineRunControl::default(),
    )
    .unwrap();
    assert_eq!(
        store.search_projection_state().unwrap().service_state,
        SearchProjectionServiceState::Ready
    );

    let ready_summary = run_ocr_worker_once(&data_dir, &store, &options, || true).unwrap();
    assert_eq!(
        ready_summary.runtime_unavailable,
        Some(ipc::OptionalRuntimeReason::Missing)
    );
    let still_queued = store.ingest_job_by_id(&job_id).unwrap().unwrap();
    assert_eq!(still_queued.status, IngestJobStatus::Queued);
    assert_eq!(still_queued.attempt_count, 0);

    drop(store);
    let _ = fs::remove_dir_all(data_dir);
}

#[test]
fn repair_blocked_projection_keeps_ocr_queued_across_worker_ticks() {
    let data_dir = std::env::temp_dir().join(format!(
        "resume-ir-daemon-ocr-repair-blocked-gate-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&data_dir).unwrap();
    let store = open_test_store(&data_dir);
    let now = UnixTimestamp::from_unix_seconds(1_800_281_100);
    let job_id = enqueue_ocr_job_for_worker_gate(&data_dir, &store, now, "blocked");
    store
        .block_migration_rebuild(SearchRepairReason::RuntimeInvariant, now)
        .unwrap();
    let options = RunOptions {
        ocr_command: Some(data_dir.join("unused-ocr-command")),
        ..RunOptions::default()
    };

    for _ in 0..2 {
        let summary = run_ocr_worker_once(&data_dir, &store, &options, || true).unwrap();
        assert!(!summary.has_activity());
    }

    let still_queued = store.ingest_job_by_id(&job_id).unwrap().unwrap();
    assert_eq!(still_queued.status, IngestJobStatus::Queued);
    assert_eq!(still_queued.attempt_count, 0);
    assert_eq!(
        store.search_projection_state().unwrap().service_state,
        SearchProjectionServiceState::RepairBlocked
    );

    drop(store);
    let _ = fs::remove_dir_all(data_dir);
}

fn enqueue_ocr_job_for_worker_gate(
    data_dir: &std::path::Path,
    store: &OwnedMetaStore,
    now: UnixTimestamp,
    fixture_id: &str,
) -> IngestJobId {
    let digest = ContentDigest::from_bytes(fixture_id.as_bytes());
    let document_id = DocumentId::from_non_secret_parts(&["daemon-ocr-gate", fixture_id]);
    let missing_document_path = data_dir.join(format!("synthetic-{fixture_id}-scanned.pdf"));
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: format!("synthetic://ocr-gate/{fixture_id}"),
            normalized_path: missing_document_path.to_string_lossy().into_owned(),
            file_name: format!("synthetic-{fixture_id}-scanned.pdf"),
            extension: FileExtension::Pdf,
            byte_size: 32,
            mtime: now,
            content_hash: Some(digest.as_str().to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::OcrRequired,
        })
        .unwrap();
    let source_revision = SourceRevision::for_content(document_id, digest, 32);
    store.insert_source_revision(&source_revision).unwrap();
    store
        .insert_source_revision_triage(&SourceRevisionTriage {
            source_revision_id: source_revision.id.clone(),
            status: ClassificationStatus::OcrBacklog,
            triage_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::OcrRequired],
            triaged_at: now,
        })
        .unwrap();
    store
        .enqueue_ocr_job_for_source_triage(
            &source_revision.id,
            CurrentClassifierEpoch::parse(CLASSIFIER_EPOCH).unwrap(),
            now,
        )
        .unwrap()
        .job
        .id
}
