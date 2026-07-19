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

pub fn write_daemon_auth(path: &Path, token: &str) {
    fs::write(
        path,
        serde_json::json!({
            "schema_version": "resume-ir.daemon-auth.v2",
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
        "schema_version": "resume-ir.daemon-ipc.v2",
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
    "{\"schema_version\":\"daemon.status.v2\",\"status\":\"ok\",\"process_state\":\"ready\",\"index_health\":\"ready\"}"
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
