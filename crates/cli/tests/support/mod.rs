#![allow(dead_code)]

use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Command, Output};

use import_pipeline::{
    current_import_processing_contract, finalize_migration_rebuild,
    prepare_migration_rebuild_artifacts, ImportOptions, SearchPublicationVectorization,
};
use meta_store::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportProcessingContract,
    ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskStatus,
    OwnedMetaStore, SearchProjectionServiceState, UnixTimestamp,
};

pub const TEST_DAEMON_INSTANCE_ID: &str =
    "abababababababababababababababababababababababababababababababab";
pub const TEST_DAEMON_LAUNCH_ID: &str =
    "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";

pub fn create_store(data_dir: &Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test store owner contended"),
    };
    owner.open_store().unwrap()
}

pub fn activate_default_processing_contract(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
) -> ImportProcessingContract {
    let contract = current_import_processing_contract(&ImportOptions::default()).unwrap();
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    contract
}

pub fn insert_import_task(store: &OwnedMetaStore, task: &ImportTask) -> ImportProcessingContract {
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
        updated_at: task.updated_at,
    };
    insert_import_task_with_scope(store, task, &scope)
}

pub fn insert_import_task_with_scope(
    store: &OwnedMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
) -> ImportProcessingContract {
    assert_ne!(task.status, ImportTaskStatus::Completed);
    let contract = activate_default_processing_contract(store, task.queued_at);
    prepare_migration_rebuild_artifacts(
        store,
        task.queued_at,
        &import_pipeline::PipelineRunControl::default(),
    )
    .unwrap();
    finalize_migration_rebuild(
        store,
        task.queued_at,
        &contract,
        &SearchPublicationVectorization::default(),
        &import_pipeline::PipelineRunControl::default(),
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

pub fn write_daemon_auth(path: &Path, token: &str) {
    fs::write(
        path,
        serde_json::json!({
            "schema_version": "resume-ir.daemon-auth.v3",
            "launch_id": TEST_DAEMON_LAUNCH_ID,
            "instance_id": TEST_DAEMON_INSTANCE_ID,
            "token": token.trim(),
        })
        .to_string(),
    )
    .expect("write daemon auth fixture");
}

pub fn write_daemon_discovery(data_dir: &Path, addr: SocketAddr, token: &str) {
    fs::create_dir_all(data_dir).expect("create daemon discovery fixture directory");
    let manifest = serde_json::json!({
        "schema_version": "resume-ir.daemon-ipc.v3",
        "launch_id": TEST_DAEMON_LAUNCH_ID,
        "instance_id": TEST_DAEMON_INSTANCE_ID,
        "owner_mode": "standalone",
        "status": format!("http://{addr}/status"),
        "diagnostics": format!("http://{addr}/diagnostics"),
        "imports": format!("http://{addr}/imports"),
        "import_cancel": format!("http://{addr}/imports/cancel"),
        "import_control": format!("http://{addr}/imports/control"),
        "import_progress": format!("http://{addr}/imports/progress"),
        "search": format!("http://{addr}/search"),
        "search_batch": format!("http://{addr}/search/batch"),
        "details": format!("http://{addr}/details"),
        "delete": format!("http://{addr}/delete"),
    });
    fs::write(data_dir.join("ipc.endpoints.json"), manifest.to_string())
        .expect("write daemon discovery manifest fixture");
    write_daemon_auth(&data_dir.join("ipc.auth"), token);
}

pub fn ready_daemon_status_body() -> &'static str {
    r#"{
        "schema_version":"daemon.status.v3",
        "status":"ok",
        "process_state":"ready",
        "core":{"state":"ready","reason":null},
        "optional_runtimes":{
            "embedding":{"state":"available","reason":null},
            "ocr":{"state":"available","reason":null},
            "classifier":{"state":"available","reason":null}
        },
        "capabilities":{
            "keyword_search":{"state":"available","reason":null},
            "detail":{"state":"available","reason":null},
            "semantic_search":{"state":"available","reason":null},
            "hybrid_search":{"state":"available","reason":null},
            "text_import":{"state":"available","reason":null},
            "ocr_import":{"state":"available","reason":null},
            "index_publication":{"state":"available","reason":null}
        },
        "error":null,
        "repair_progress":null,
        "indexed_documents":4,
        "searchable_documents":3,
        "partial_documents":1,
        "visible_epoch":7,
        "failed_retryable":0,
        "failed_permanent":0,
        "recovery_queue_depth":0,
        "ocr_queue_depth":0,
        "ocr_jobs_queued":0,
        "ocr_page_budget_blocked":0,
        "ocr_remediation":"none",
        "ocr_language_unavailable":0,
        "ocr_language_remediation":"none",
        "embedding_queue_depth":0,
        "entity_mentions":8,
        "import_tasks_queued":0,
        "import_tasks_recoverable":0,
        "import_tasks_cancelled":0,
        "import_scan_scopes":1,
        "import_scan_errors":0,
        "query_latency":{
            "sample_count":1,
            "p50_ms":2.0,
            "p95_ms":3.0,
            "p99_ms":4.0,
            "last_result_count":1,
            "raw_queries":"<redacted>"
        },
        "latest_import_scan":null,
        "active_profile":"balanced",
        "index_health":"ready",
        "snapshot_present":true,
        "ipc":{"accepted":2,"completed":2,"client_disconnect":0,"request_failure":0,"response_failure":0}
    }"#
}

pub fn import_text_resumes<N: AsRef<str>, T: AsRef<str>>(
    data_dir: &Path,
    source_root: &Path,
    files: &[(N, T)],
) -> Output {
    fs::create_dir_all(source_root).expect("create synthetic source root");
    for (file_name, text) in files {
        fs::write(source_root.join(file_name.as_ref()), text.as_ref())
            .expect("write synthetic resume fixture");
    }

    import_existing_root(data_dir, source_root)
}

pub fn import_existing_root(data_dir: &Path, source_root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(source_root),
            "--parse-workers",
            "1",
        ])
        .output()
        .expect("run resume-cli import")
}

pub fn assert_import_succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("synthetic fixture path is utf-8")
}
