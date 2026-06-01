use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus,
    MetaStore, UnixTimestamp,
};

#[test]
fn foreground_once_opens_store_reports_ready_and_exits() {
    let data_dir = temp_dir("daemon-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
        ])
        .output()
        .expect("run resume-daemon foreground once");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-daemon foreground ready"));
    assert!(stdout.contains("mode: once"));
    assert!(stdout.contains("index health: empty"));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[test]
fn foreground_once_worker_processes_queued_import_task_from_persistent_scope() {
    let data_dir = temp_dir("daemon-import-worker-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_queued_import_task(
        &data_dir,
        "daemon-import-worker",
        &canonical_fixture_root,
        1_700_000_000,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-imports-once",
        ])
        .output()
        .expect("run resume-daemon import worker once");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import worker processed: 1"));
    assert!(stdout.contains("import worker searchable documents: 2"));
    assert!(stdout.contains("import worker ocr jobs queued: 1"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    let summary = store.status_summary().unwrap();
    assert_eq!(summary.import_tasks_queued, 0);
    assert_eq!(summary.import_tasks_recoverable, 0);
    assert_eq!(summary.searchable_documents, 2);

    remove_dir(&data_dir);
}

#[test]
fn foreground_once_worker_continues_after_retryable_import_failure() {
    let data_dir = temp_dir("daemon-import-worker-failure-data");
    let missing_root = temp_dir("daemon-import-worker-missing-root");
    remove_dir(&missing_root);
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let failed_task_id = seed_queued_import_task(
        &data_dir,
        "daemon-import-worker-missing",
        &missing_root,
        1_700_000_000,
    );
    let completed_task_id = seed_queued_import_task(
        &data_dir,
        "daemon-import-worker-valid",
        &canonical_fixture_root,
        1_700_000_010,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-imports-once",
        ])
        .output()
        .expect("run resume-daemon import worker once");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import worker processed: 1"));
    assert!(stdout.contains("import worker failed: 1"));
    assert!(stdout.contains("import worker searchable documents: 2"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&missing_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let failed_task = store.import_task_by_id(&failed_task_id).unwrap().unwrap();
    let completed_task = store
        .import_task_by_id(&completed_task_id)
        .unwrap()
        .unwrap();
    assert_eq!(failed_task.status, ImportTaskStatus::FailedRetryable);
    assert_eq!(completed_task.status, ImportTaskStatus::Completed);

    remove_dir(&data_dir);
}

fn seed_queued_import_task(
    data_dir: &Path,
    label: &str,
    canonical_root: &Path,
    queued_at_seconds: i64,
) -> ImportTaskId {
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(queued_at_seconds);
    let task_id = ImportTaskId::from_non_secret_parts(&["s43", label]);
    store
        .insert_import_task(&ImportTask {
            id: task_id.clone(),
            root_path: path_str(canonical_root).to_string(),
            status: ImportTaskStatus::Queued,
            queued_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
        })
        .unwrap();
    store
        .upsert_import_scan_scope(&ImportScanScope {
            import_task_id: task_id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: path_str(canonical_root).to_string(),
            canonical_root_path: path_str(canonical_root).to_string(),
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
        })
        .unwrap();
    task_id
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes")
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s4-daemon-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}
