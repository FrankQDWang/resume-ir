use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, IndexDocument, IndexSection, SearchQuery};
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

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
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
fn foreground_once_index_worker_rebuilds_missing_full_text_snapshot_without_path_leak() {
    let data_dir = temp_dir("daemon-index-worker-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    seed_queued_import_task(
        &data_dir,
        "daemon-index-worker-import",
        &canonical_fixture_root,
        1_700_000_000,
    );
    run_import_worker_once(&data_dir);
    fs::remove_dir_all(data_dir.join("search-index")).unwrap();
    assert!(FullTextIndex::open_active(&data_dir.join("search-index"))
        .unwrap()
        .is_none());

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-index-once",
        ])
        .output()
        .expect("run resume-daemon index worker once");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("index worker rebuilt: yes"));
    assert!(stdout.contains("index worker indexed documents: 2"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!search_fulltext(&data_dir, "java").is_empty());

    remove_dir(&data_dir);
}

#[test]
fn foreground_index_worker_loop_repairs_missing_snapshot_once_per_health_change() {
    let data_dir = temp_dir("daemon-index-loop-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    seed_queued_import_task(
        &data_dir,
        "daemon-index-loop-import",
        &canonical_fixture_root,
        1_700_000_000,
    );
    run_import_worker_once(&data_dir);
    fs::remove_dir_all(data_dir.join("search-index")).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-index",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "2",
        ])
        .output()
        .expect("run resume-daemon index worker loop");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("index worker rebuilt: yes").count(), 1);
    assert!(stdout.contains("index worker indexed documents: 2"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!search_fulltext(&data_dir, "java").is_empty());

    remove_dir(&data_dir);
}

#[test]
fn foreground_index_worker_loop_rebuilds_legacy_root_snapshot_layout() {
    let data_dir = temp_dir("daemon-index-legacy-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    seed_queued_import_task(
        &data_dir,
        "daemon-index-legacy-import",
        &canonical_fixture_root,
        1_700_000_000,
    );
    run_import_worker_once(&data_dir);
    write_legacy_root_fulltext_index(&data_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-index",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "2",
        ])
        .output()
        .expect("run resume-daemon index worker loop over legacy root");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("index worker rebuilt: yes").count(), 1);
    assert!(stdout.contains("index worker indexed documents: 2"));
    assert!(data_dir
        .join("search-index")
        .join("active-snapshot")
        .exists());
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!search_fulltext(&data_dir, "java").is_empty());

    remove_dir(&data_dir);
}

#[test]
fn foreground_once_worker_skips_cancelled_import_task() {
    let data_dir = temp_dir("daemon-import-cancelled-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_queued_import_task(
        &data_dir,
        "daemon-import-cancelled",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .cancel_import_task(&task_id, UnixTimestamp::from_unix_seconds(1_700_000_010))
        .unwrap();

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
        .expect("run resume-daemon import worker once with cancelled task");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import worker processed: 0"));
    assert!(stdout.contains("import worker searchable documents: 0"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));

    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Queued);
    assert!(store.is_import_task_cancelled(&task_id).unwrap());
    let summary = store.status_summary().unwrap();
    assert_eq!(summary.import_tasks_queued, 0);
    assert_eq!(summary.import_tasks_cancelled, 1);
    assert_eq!(summary.searchable_documents, 0);

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

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
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

#[test]
fn foreground_import_scheduler_processes_task_enqueued_after_startup() {
    let data_dir = temp_dir("daemon-import-scheduler-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "25",
            "--max-worker-ticks",
            "80",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon import scheduler");

    let stdout = child.stdout.take().expect("daemon stdout");
    wait_until_metadata_store_ready(&mut child, &data_dir);

    let task_id = seed_queued_import_task(
        &data_dir,
        "daemon-import-scheduler",
        &canonical_fixture_root,
        1_700_000_000,
    );

    let output = wait_daemon(child, BufReader::new(stdout));
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    assert!(
        output.stdout.contains("import worker processed: 1"),
        "stdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    assert!(output
        .stdout
        .contains("import worker searchable documents: 2"));
    assert!(!output.stdout.contains(path_str(&data_dir)));
    assert!(!output.stdout.contains(path_str(&fixture_root)));
    assert!(!output.stdout.contains(path_str(&canonical_fixture_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().searchable_documents, 2);

    remove_dir(&data_dir);
}

#[test]
fn foreground_import_scheduler_rescans_completed_root_without_path_leak() {
    let data_dir = temp_dir("daemon-import-rescan-data");
    let fixture_root = temp_dir("daemon-import-rescan-root");
    fs::write(
        fixture_root.join("first.txt"),
        b"Synthetic first resume\nSkills: Rust",
    )
    .unwrap();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    seed_queued_import_task(
        &data_dir,
        "daemon-import-rescan-initial",
        &canonical_fixture_root,
        1_700_000_000,
    );
    run_import_worker_once(&data_dir);
    fs::write(
        fixture_root.join("second.txt"),
        b"Synthetic second resume\nSkills: Kubernetes",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--rescan-completed-imports",
            "--import-rescan-min-age-seconds",
            "0",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "1",
        ])
        .output()
        .expect("run resume-daemon import scheduler with completed root rescan");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import worker requeued completed imports: 1"));
    assert!(stdout.contains("import worker processed: 1"));
    assert!(stdout.contains("import worker searchable documents: 2"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!search_fulltext(&data_dir, "kubernetes").is_empty());

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    assert_eq!(store.status_summary().unwrap().searchable_documents, 2);

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
fn foreground_import_scheduler_backs_off_retryable_failures() {
    let data_dir = temp_dir("daemon-import-scheduler-backoff-data");
    let missing_root = temp_dir("daemon-import-scheduler-backoff-missing-root");
    remove_dir(&missing_root);
    let task_id = seed_queued_import_task(
        &data_dir,
        "daemon-import-scheduler-backoff",
        &missing_root,
        1_700_000_000,
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "25",
            "--max-worker-ticks",
            "30",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon import scheduler");

    let stdout = child.stdout.take().expect("daemon stdout");
    let output = wait_daemon(child, BufReader::new(stdout));
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout.matches("import worker failed: 1").count(), 1);
    assert!(!output.stdout.contains(path_str(&data_dir)));
    assert!(!output.stdout.contains(path_str(&missing_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::FailedRetryable);

    remove_dir(&data_dir);
}

#[test]
fn foreground_import_scheduler_recovers_stale_running_import_task() {
    let data_dir = temp_dir("daemon-import-scheduler-recovery-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_queued_import_task(
        &data_dir,
        "daemon-import-scheduler-stale-running",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .update_import_task_status(
            &task_id,
            ImportTaskStatus::Running,
            UnixTimestamp::from_unix_seconds(1_700_000_010),
        )
        .unwrap();
    drop(store);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "25",
            "--max-worker-ticks",
            "2",
        ])
        .output()
        .expect("run resume-daemon import scheduler");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import worker recovered stale running: 1"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::FailedRetryable);
    assert!(task.finished_at.is_some());

    remove_dir(&data_dir);
}

#[test]
fn foreground_import_scheduler_recovers_active_import_after_kill_and_restart() {
    let data_dir = temp_dir("daemon-import-active-kill-data");
    let fixture_root = active_kill_fixture_root(1_024);
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_queued_import_task(
        &data_dir,
        "daemon-import-active-kill",
        &canonical_fixture_root,
        1_700_000_000,
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "25",
            "--max-worker-ticks",
            "240",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon import scheduler");
    let stdout = child.stdout.take().expect("daemon stdout");
    wait_until_import_task_running(&mut child, &data_dir, &task_id);

    child.kill().expect("kill daemon during active import");
    let killed_output = wait_killed_daemon(child, BufReader::new(stdout));
    assert!(!killed_output.success);
    assert!(!killed_output.stdout.contains(path_str(&data_dir)));
    assert!(!killed_output.stdout.contains(path_str(&fixture_root)));
    assert!(!killed_output
        .stdout
        .contains(path_str(&canonical_fixture_root)));
    assert!(!killed_output.stderr.contains(path_str(&data_dir)));
    assert!(!killed_output.stderr.contains(path_str(&fixture_root)));
    assert!(!killed_output
        .stderr
        .contains(path_str(&canonical_fixture_root)));

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--stale-import-task-seconds",
            "0",
            "--import-retry-backoff-seconds",
            "0",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "2",
        ])
        .output()
        .expect("restart resume-daemon import scheduler");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import worker recovered stale running: 1"));
    assert!(stdout.contains("import worker processed: 1"));
    assert!(stdout.contains("import worker searchable documents: 1024"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!search_fulltext(&data_dir, "ActiveKillToken").is_empty());

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().searchable_documents, 1_024);

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
fn foreground_import_watcher_requeues_completed_root_after_file_change_without_path_leak() {
    let data_dir = temp_dir("daemon-import-watcher-data");
    let watched_root = temp_dir("daemon-import-watcher-root");
    let watched_file = watched_root.join("candidate.txt");
    fs::write(
        &watched_file,
        "Initial watcher candidate with Rust backend experience.",
    )
    .unwrap();
    let canonical_watched_root = fs::canonicalize(&watched_root).unwrap();
    seed_queued_import_task(
        &data_dir,
        "daemon-import-watcher-initial",
        &canonical_watched_root,
        1_700_000_000,
    );
    run_import_worker_once(&data_dir);
    assert!(search_fulltext(&data_dir, "WatcherUpdatedToken").is_empty());

    let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--watch-import-roots",
            "--worker-interval-ms",
            "25",
            "--max-worker-ticks",
            "120",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start resume-daemon import watcher");
    wait_until_metadata_store_ready(&mut child, &data_dir);
    std::thread::sleep(Duration::from_millis(250));
    fs::write(
        &watched_file,
        "WatcherUpdatedToken refreshed candidate with Rust backend experience.",
    )
    .unwrap();

    let stdout = child.stdout.take().expect("daemon stdout");
    let output = wait_daemon(child, BufReader::new(stdout));
    assert!(
        output.success,
        "stdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    );
    assert!(output.stderr.is_empty());
    assert!(output.stdout.contains("import watcher active roots: 1"));
    assert!(output.stdout.contains("import watcher requeued imports: 1"));
    assert!(output.stdout.contains("import worker processed: 1"));
    assert!(!output.stdout.contains(path_str(&data_dir)));
    assert!(!output.stdout.contains(path_str(&watched_root)));
    assert!(!output.stdout.contains(path_str(&canonical_watched_root)));
    assert!(!output.stdout.contains(path_str(&watched_file)));
    assert!(!search_fulltext(&data_dir, "WatcherUpdatedToken").is_empty());

    remove_dir(&data_dir);
    remove_dir(&watched_root);
}

fn seed_queued_import_task(
    data_dir: &Path,
    label: &str,
    canonical_root: &Path,
    queued_at_seconds: i64,
) -> ImportTaskId {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(queued_at_seconds);
    let task_id = ImportTaskId::from_non_secret_parts(&["s43", label]);
    let task = ImportTask {
        id: task_id.clone(),
        root_path: path_str(canonical_root).to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
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
    };
    store
        .insert_import_task_with_scan_scope(&task, &scope)
        .unwrap();
    task_id
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes")
}

fn active_kill_fixture_root(file_count: usize) -> PathBuf {
    let root = temp_dir("daemon-import-active-kill-root");
    for index in 0..file_count {
        fs::write(
            root.join(format!("candidate-{index:04}.txt")),
            format!(
                "Synthetic resume {index}\nSkills: Rust Java Kubernetes ActiveKillToken\nExperience: {}\n",
                "local-first search ".repeat(48)
            ),
        )
        .unwrap();
    }
    root
}

fn run_import_worker_once(data_dir: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(data_dir),
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
}

fn search_fulltext(data_dir: &Path, query: &str) -> Vec<index_fulltext::SearchHit> {
    let index = FullTextIndex::open_active(&data_dir.join("search-index"))
        .unwrap()
        .expect("active full-text index");
    index
        .search(SearchQuery::new(query).with_limit(20))
        .expect("search full-text index")
}

fn write_legacy_root_fulltext_index(data_dir: &Path) {
    let index_root = data_dir.join("search-index");
    fs::remove_dir_all(&index_root).unwrap();
    let index = FullTextIndex::open_or_create(&index_root).unwrap();
    index
        .replace_documents([IndexDocument {
            doc_id: "legacy-doc".to_string(),
            version_id: "legacy-version".to_string(),
            file_name: "legacy.txt".to_string(),
            clean_text: "legacy root layout marker".to_string(),
            sections: vec![IndexSection {
                section_type: "summary".to_string(),
                text: "legacy root layout marker".to_string(),
            }],
            is_deleted: false,
        }])
        .unwrap();
    index.commit().unwrap();
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

fn wait_until_metadata_store_ready(child: &mut Child, data_dir: &Path) {
    let metadata_store = data_dir.join("metadata.sqlite3");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if metadata_store.exists()
            && MetaStore::open_data_dir(data_dir)
                .and_then(|store| store.status_summary().map(|_| ()))
                .is_ok()
        {
            return;
        }
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            panic!("daemon exited before metadata store was ready: {status}");
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    let _ = child.kill();
    let _ = child.wait();
    panic!("daemon did not prepare metadata store before timeout");
}

fn wait_until_import_task_running(child: &mut Child, data_dir: &Path, task_id: &ImportTaskId) {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            panic!("daemon exited before import task entered running: {status}");
        }
        let task = store.import_task_by_id(task_id).unwrap().unwrap();
        match task.status {
            ImportTaskStatus::Running => return,
            ImportTaskStatus::Completed => {
                panic!("import completed before active kill window")
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    let _ = child.kill();
    let _ = child.wait();
    panic!("import task did not enter running before timeout");
}

struct DaemonOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

fn wait_killed_daemon(mut child: Child, mut stdout: BufReader<ChildStdout>) -> DaemonOutput {
    let status = child.wait().expect("wait killed daemon");
    let mut stdout_text = String::new();
    stdout
        .read_to_string(&mut stdout_text)
        .expect("read killed daemon stdout");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("daemon stderr")
        .read_to_string(&mut stderr)
        .expect("read killed daemon stderr");
    DaemonOutput {
        success: status.success(),
        stdout: stdout_text,
        stderr,
    }
}

fn wait_daemon(mut child: Child, mut stdout: BufReader<ChildStdout>) -> DaemonOutput {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            let mut stdout_text = String::new();
            stdout
                .read_to_string(&mut stdout_text)
                .expect("read daemon stdout");
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .expect("daemon stderr")
                .read_to_string(&mut stderr)
                .expect("read daemon stderr");
            return DaemonOutput {
                success: status.success(),
                stdout: stdout_text,
                stderr,
            };
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon did not exit after max worker ticks");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
