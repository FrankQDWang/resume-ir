use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
#[cfg(windows)]
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use import_pipeline::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportTaskOwnerLock,
};
use meta_store::{
    ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus,
    OwnedMetaStore, ReadMetaStore, SearchProjectionServiceState, SearchRepairReason, UnixTimestamp,
};
use process_containment::ContainedChild;
use search_runtime::{HitLimit, QueryCoordinator};

mod support;

const ACTIVE_KILL_FIXTURE_FILE_COUNT: usize = 128;

// The integration suite intentionally runs several resident daemon processes. On Windows,
// admitting more than one such test at a time can starve process startup and index writers;
// production still supports its normal one-daemon-per-data-directory ownership model.
macro_rules! serialize_windows_s4_daemon_test {
    () => {
        #[cfg(windows)]
        let _guard = windows_s4_daemon_test_lock();
    };
}

#[cfg(windows)]
fn windows_s4_daemon_test_lock() -> MutexGuard<'static, ()> {
    static WINDOWS_S4_DAEMON_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    WINDOWS_S4_DAEMON_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn retired_embedding_writer_flags_are_rejected_before_daemon_startup() {
    serialize_windows_s4_daemon_test!();
    for retired_args in [
        vec!["--once", "--work-embeddings-once"],
        vec!["--work-embeddings"],
        vec!["--embedding-max-docs", "1"],
        vec!["--embedding-max-text-bytes", "1024"],
    ] {
        let data_dir = temp_dir("retired-embedding-writer-flag");
        remove_dir(&data_dir);
        let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
            .args(["--data-dir", path_str(&data_dir), "run", "--foreground"])
            .args(retired_args)
            .output()
            .expect("run resume-daemon with a retired embedding writer flag");

        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!data_dir.exists());
    }
}

#[test]
fn foreground_once_opens_store_reports_unpublished_repair_state_and_exits() {
    serialize_windows_s4_daemon_test!();
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
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let state = store.search_projection_state().unwrap();
    assert_eq!(state.service_state, SearchProjectionServiceState::Repairing);
    assert_eq!(
        state.repair_reason,
        Some(SearchRepairReason::MigrationRebuild)
    );
    assert!(state.generation.is_none());
    drop(store);

    remove_dir(&data_dir);
}

#[test]
fn foreground_once_rejects_a_competing_import_processing_owner() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-processing-owner-data");
    let owner = match DataDirectoryOwnerLease::try_acquire(&data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(lease) => lease,
        DataDirectoryOwnerAcquisition::Contended => panic!("test data dir is already owned"),
    };

    let blocked = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
        ])
        .output()
        .expect("run competing resume-daemon foreground once");

    assert_eq!(blocked.status.code(), Some(1));
    assert!(blocked.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(!stderr.contains(path_str(&data_dir)));
    let event: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("parse daemon ownership fatal event");
    assert_eq!(event["schema_version"], "resume-ir.daemon-fatal.v1");
    assert_eq!(event["event"], "fatal");
    assert_eq!(event["class"], "ownership_conflict");
    assert_eq!(event["disposition"], "blocked");

    drop(owner);
    let recovered = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
        ])
        .output()
        .expect("run resume-daemon after processing ownership release");
    assert!(
        recovered.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&recovered.stdout),
        String::from_utf8_lossy(&recovered.stderr)
    );

    remove_dir(&data_dir);
}

#[test]
fn foreground_once_worker_processes_queued_import_task_from_persistent_scope() {
    serialize_windows_s4_daemon_test!();
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    let summary = store.status_summary().unwrap();
    assert_eq!(summary.import_tasks_queued, 0);
    assert_eq!(summary.import_tasks_recoverable, 0);
    assert_eq!(summary.searchable_documents, 2);

    remove_dir(&data_dir);
}

#[test]
fn migration_rebuild_retires_legacy_search_artifacts_before_first_publication() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-migration-legacy-artifacts-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_queued_import_task(
        &data_dir,
        "daemon-migration-legacy-artifacts",
        &canonical_fixture_root,
        1_700_000_000,
    );
    seed_legacy_search_artifact_layout(&data_dir);

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
        .expect("run migration rebuild worker once");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import worker processed: 1"), "{stdout}");
    assert!(!stdout.contains("import worker failed: 1"), "{stdout}");
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        store.import_task_by_id(&task_id).unwrap().unwrap().status,
        ImportTaskStatus::Completed
    );
    let projection = store.search_projection_state().unwrap();
    assert_eq!(
        projection.service_state,
        SearchProjectionServiceState::Ready
    );
    assert!(projection.visible_epoch > 0);
    assert!(projection.generation.is_some());
    assert!(!data_dir
        .join("search-index/fulltext.snapshot.key-v1")
        .exists());
    assert!(!data_dir
        .join("vector-index/vector.snapshot.key-v1")
        .exists());
    assert!(!search_fulltext(&data_dir, "java").is_empty());

    remove_dir(&data_dir);
}

#[test]
fn migration_rebuild_invalid_publication_lock_blocks_repair_without_exiting_daemon() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-migration-invalid-publication-lock-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_queued_import_task(
        &data_dir,
        "daemon-migration-invalid-lock-pending",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let publication_lock = data_dir.join("search-publication.lock");
    fs::remove_file(&publication_lock).unwrap();
    fs::create_dir(&publication_lock).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-imports-once",
            "--work-index-once",
        ])
        .output()
        .expect("run daemon with invalid publication lock layout");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).contains("resume-daemon foreground ready"));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let state = store.search_projection_state().unwrap();
    assert_eq!(
        state.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        state.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(
        store.import_task_by_id(&task_id).unwrap().unwrap().status,
        ImportTaskStatus::Queued
    );
    drop(store);

    let restart = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
        ])
        .output()
        .expect("restart daemon after deterministic migration block");
    assert!(restart.status.success());
    assert!(restart.stderr.is_empty());
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let restarted_state = store.search_projection_state().unwrap();
    assert_eq!(
        restarted_state.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        restarted_state.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );

    remove_dir(&data_dir);
}

#[test]
fn foreground_startup_rebuilds_missing_ready_snapshot_without_manual_worker() {
    serialize_windows_s4_daemon_test!();
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
    let missing_generation = ready_generation(&data_dir);
    fs::remove_dir_all(data_dir.join("search-index")).unwrap();
    assert!(!data_dir
        .join("search-index/snapshots")
        .join(&missing_generation)
        .exists());

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
        .expect("run resume-daemon startup recovery once");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-daemon foreground ready"));
    assert!(stdout.contains("index health: ready"));
    assert!(stdout.contains("search artifact worker active generation rebuilt: yes"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!search_fulltext(&data_dir, "java").is_empty());

    remove_dir(&data_dir);
}

#[test]
fn foreground_startup_rebuilds_corrupt_ready_snapshot_without_manual_worker() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-index-worker-schema-mismatch-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    seed_queued_import_task(
        &data_dir,
        "daemon-index-worker-schema-mismatch-import",
        &canonical_fixture_root,
        1_700_000_000,
    );
    run_import_worker_once(&data_dir);

    let index_root = data_dir.join("search-index");
    let ready_generation = ready_generation(&data_dir);
    let manifest_path = index_root
        .join("snapshots")
        .join(&ready_generation)
        .join("snapshot-manifest.json");
    let manifest = fs::read_to_string(&manifest_path).unwrap();
    assert!(manifest.contains("\"schema_version\":\"fulltext.snapshot.v3\""));
    fs::write(
        &manifest_path,
        "{\"schema_version\":\"fulltext.snapshot.v999\",\"index_schema\":\"future-fulltext-schema\",\"payload\":\"PRIVATE schema mismatch path\"}\n",
    )
    .unwrap();

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
        .expect("run resume-daemon startup recovery after schema mismatch");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("index health: ready"));
    assert!(stdout.contains("search artifact worker active generation rebuilt: yes"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!stdout.contains("PRIVATE schema mismatch"));
    assert!(!search_fulltext(&data_dir, "java").is_empty());

    remove_dir(&data_dir);
}

#[test]
fn foreground_index_worker_loop_observes_startup_repaired_snapshot() {
    serialize_windows_s4_daemon_test!();
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
    assert!(stdout.contains("index health: ready"));
    assert_eq!(
        stdout
            .matches("search artifact worker active generation rebuilt: yes")
            .count(),
        1
    );
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!search_fulltext(&data_dir, "java").is_empty());

    remove_dir(&data_dir);
}

#[test]
fn foreground_once_worker_skips_cancelled_import_task() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-cancelled-data");
    initialize_empty_ready_store(&data_dir);
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_queued_import_task_in_ready_store(
        &data_dir,
        "daemon-import-cancelled",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let store = open_owned_store(&data_dir);
    store
        .cancel_import_task(&task_id, UnixTimestamp::from_unix_seconds(1_700_000_010))
        .unwrap();
    drop(store);

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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-worker-failure-data");
    initialize_empty_ready_store(&data_dir);
    let missing_root = temp_dir("daemon-import-worker-missing-root");
    remove_dir(&missing_root);
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let failed_task_id = seed_queued_import_task_in_ready_store(
        &data_dir,
        "daemon-import-worker-missing",
        &missing_root,
        1_700_000_000,
    );
    let completed_task_id = seed_queued_import_task_in_ready_store(
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
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
fn migration_rebuild_never_publishes_valid_root_before_later_unavailable_root() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-migration-publication-barrier-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let missing_root = temp_dir("daemon-migration-publication-barrier-missing-root");
    remove_dir(&missing_root);
    let completed_task_id = seed_queued_import_task(
        &data_dir,
        "daemon-migration-valid-first",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let failed_task_id = seed_queued_import_task(
        &data_dir,
        "daemon-migration-missing-later",
        &missing_root,
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
        .expect("run migration rebuild behind the all-root publication barrier");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        store
            .import_task_by_id(&completed_task_id)
            .unwrap()
            .unwrap()
            .status,
        ImportTaskStatus::Completed
    );
    assert_eq!(
        store
            .import_task_by_id(&failed_task_id)
            .unwrap()
            .unwrap()
            .status,
        ImportTaskStatus::FailedRetryable
    );
    let projection = store.search_projection_state().unwrap();
    assert_eq!(
        projection.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        projection.repair_reason,
        Some(SearchRepairReason::SourceUnavailable)
    );
    assert!(projection.generation.is_none());
    assert_eq!(projection.visible_epoch, 0);
    assert!(store.searchable_document_ids().unwrap().is_empty());

    remove_dir(&data_dir);
}

#[test]
fn foreground_import_scheduler_processes_task_enqueued_after_startup() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-scheduler-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_resume-daemon"));
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "25",
            "--ipc-listen",
            "127.0.0.1:0",
            "--parent-lifecycle-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child =
        ContainedChild::spawn(&mut command).expect("start resume-daemon import scheduler");
    let lifecycle_stdin = child.take_stdin().expect("daemon lifecycle stdin");
    let stdout = child.take_stdout().expect("daemon stdout");
    let mut stderr = child.take_stderr().expect("daemon stderr");
    let mut stdout = spawn_daemon_stdout_collector(stdout);
    wait_until_contained_foreground_ready(&mut child, &mut stdout, &mut stderr);

    let task_id = request_daemon_import(&data_dir, &canonical_fixture_root);
    wait_until_import_task_completed(&mut child, &data_dir, &task_id);
    wait_until_search_projection_ready(&mut child, &data_dir);
    request_daemon_status(&data_dir);
    drop(lifecycle_stdin);

    let output = wait_contained_daemon(child, stdout, stderr);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains("import worker processed:"));
    assert!(!output.stdout.contains(path_str(&data_dir)));
    assert!(!output.stdout.contains(path_str(&fixture_root)));
    assert!(!output.stdout.contains(path_str(&canonical_fixture_root)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().searchable_documents, 2);

    remove_dir(&data_dir);
}

#[test]
fn foreground_import_scheduler_rescans_completed_root_without_path_leak() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-rescan-data");
    let fixture_root = temp_dir("daemon-import-rescan-root");
    fs::write(
        fixture_root.join("first.txt"),
        b"SUMMARY\nSynthetic first resume.\nEXPERIENCE\nBuilt Rust search services.\nSKILLS\nRust",
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
        b"SUMMARY\nSynthetic second resume.\nEXPERIENCE\nBuilt Kubernetes search services.\nSKILLS\nKubernetes",
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

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(store.status_summary().unwrap().searchable_documents, 2);

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
fn foreground_import_scheduler_preserves_rescan_interval_across_restart() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-startup-catchup-data");
    let fixture_root = temp_dir("daemon-import-startup-catchup-root");
    fs::write(
        fixture_root.join("first.txt"),
        b"SUMMARY\nSynthetic first resume.\nEXPERIENCE\nBuilt Rust search services.\nSKILLS\nRust",
    )
    .unwrap();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    seed_queued_import_task(
        &data_dir,
        "daemon-import-startup-catchup-initial",
        &canonical_fixture_root,
        1_700_000_000,
    );
    run_import_worker_once(&data_dir);
    fs::write(
        fixture_root.join("second.txt"),
        b"SUMMARY\nStartupCatchupToken candidate.\nEXPERIENCE\nBuilt local search services.\nSKILLS\nSearch",
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
            "300",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "2",
        ])
        .output()
        .expect("run resume-daemon restart inside rescan interval");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("import worker requeued completed imports:"));
    assert!(!stdout.contains("import worker processed:"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(search_fulltext(&data_dir, "StartupCatchupToken").is_empty());

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
fn foreground_import_scheduler_backs_off_retryable_failures() {
    serialize_windows_s4_daemon_test!();
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
    let stdout = spawn_daemon_stdout_collector(stdout);
    let output = wait_daemon(child, stdout);
    assert!(output.success, "stderr:\n{}", output.stderr);
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout.matches("import worker failed: 1").count(), 1);
    assert!(!output.stdout.contains(path_str(&data_dir)));
    assert!(!output.stdout.contains(path_str(&missing_root)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::FailedRetryable);
    let projection = store.search_projection_state().unwrap();
    assert_eq!(
        projection.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        projection.repair_reason,
        Some(SearchRepairReason::SourceUnavailable)
    );
    assert!(projection.generation.is_none());

    remove_dir(&data_dir);
}

#[test]
fn foreground_import_scheduler_recovers_orphaned_running_import_task_immediately() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-scheduler-recovery-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    initialize_empty_ready_store(&data_dir);
    let task_id = seed_queued_import_task_in_ready_store(
        &data_dir,
        "daemon-import-scheduler-stale-running",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let store = open_owned_store(&data_dir);
    let queued = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert!(store
        .claim_observed_import_task_for_worker(
            &queued,
            UnixTimestamp::from_unix_seconds(1_700_000_010),
        )
        .unwrap()
        .is_some());
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
    assert!(stdout.contains("import worker recovered orphaned running: 1"));
    assert!(stdout.contains("import worker recovered stale running: 0"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert!(task.finished_at.is_some());
    assert!(store.status_summary().unwrap().searchable_documents > 0);

    remove_dir(&data_dir);
}

#[test]
fn foreground_import_scheduler_fails_closed_against_a_legacy_live_task_owner() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-live-owner-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    initialize_empty_ready_store(&data_dir);
    let task_id = seed_queued_import_task_in_ready_store(
        &data_dir,
        "daemon-import-live-owner",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let store = open_owned_store(&data_dir);
    let queued = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert!(store
        .claim_observed_import_task_for_worker(
            &queued,
            UnixTimestamp::from_unix_seconds(1_700_000_010),
        )
        .unwrap()
        .is_some());
    drop(store);
    let live_owner = ImportTaskOwnerLock::acquire(&data_dir, &task_id).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--stale-import-task-seconds",
            "0",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "2",
        ])
        .output()
        .expect("run resume-daemon beside live import owner");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let event: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(event["class"], "ownership_conflict");
    assert_eq!(event["disposition"], "blocked");
    assert!(!stderr.contains(path_str(&data_dir)));
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        store.import_task_by_id(&task_id).unwrap().unwrap().status,
        ImportTaskStatus::Running
    );

    drop(live_owner);
    remove_dir(&data_dir);
}

#[test]
fn foreground_import_scheduler_claims_only_after_acquiring_the_owner_lock() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-owner-handshake-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let task_id = seed_queued_import_task(
        &data_dir,
        "daemon-import-owner-handshake",
        &canonical_fixture_root,
        1_700_000_000,
    );
    let competing_owner = ImportTaskOwnerLock::acquire(&data_dir, &task_id).unwrap();

    let blocked_claim = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "1",
        ])
        .output()
        .expect("run import worker while candidate owner lock is held");

    assert!(blocked_claim.status.success());
    assert!(blocked_claim.stderr.is_empty());
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        store.import_task_by_id(&task_id).unwrap().unwrap().status,
        ImportTaskStatus::Queued
    );
    drop(store);

    drop(competing_owner);
    run_import_worker_once(&data_dir);
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        store.import_task_by_id(&task_id).unwrap().unwrap().status,
        ImportTaskStatus::Completed
    );
    assert!(store.status_summary().unwrap().searchable_documents > 0);

    remove_dir(&data_dir);
}

#[test]
fn migration_rebuild_reconciles_a_root_cancelled_after_the_first_worker_tick() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-migration-live-cancel-data");
    let fixture_root = active_kill_fixture_root(ACTIVE_KILL_FIXTURE_FILE_COUNT);
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let cancelled_task_id = seed_queued_import_task(
        &data_dir,
        "daemon-migration-live-cancel",
        &canonical_fixture_root,
        1_700_000_000,
    );

    let mut command = Command::new(env!("CARGO_BIN_EXE_resume-daemon"));
    command
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--work-imports",
            "--worker-interval-ms",
            "25",
            "--ipc-listen",
            "127.0.0.1:0",
            "--parent-lifecycle-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child =
        ContainedChild::spawn(&mut command).expect("start migration rebuild import scheduler");
    let lifecycle_stdin = child.take_stdin().expect("daemon lifecycle stdin");
    let stdout = child.take_stdout().expect("daemon stdout");
    let stderr = child.take_stderr().expect("daemon stderr");
    let stdout = spawn_daemon_stdout_collector(stdout);
    wait_until_import_task_running(&mut child, &data_dir, &cancelled_task_id);

    request_daemon_import_cancellation(&data_dir, &cancelled_task_id);
    let replacement_task_id = wait_until_replacement_import_completed(
        &mut child,
        &data_dir,
        &cancelled_task_id,
        &canonical_fixture_root,
    );
    drop(lifecycle_stdin);

    let output = wait_contained_daemon(child, stdout, stderr);
    assert!(
        output.success,
        "stdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    );
    assert!(output.stderr.is_empty());
    assert!(!output.stdout.contains("import worker cancelled:"));
    assert!(!output
        .stdout
        .contains("import worker queued migration repairs:"));
    assert!(!output.stdout.contains(path_str(&data_dir)));
    assert!(!output.stdout.contains(path_str(&fixture_root)));
    assert!(!output.stdout.contains(path_str(&canonical_fixture_root)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert!(store.is_import_task_cancelled(&cancelled_task_id).unwrap());
    let replacement = store
        .latest_import_task_by_root(path_str(&canonical_fixture_root))
        .unwrap()
        .unwrap();
    assert_eq!(replacement.id, replacement_task_id);
    assert_ne!(replacement.id, cancelled_task_id);
    assert_eq!(replacement.status, ImportTaskStatus::Completed);
    assert!(!store.is_import_task_cancelled(&replacement.id).unwrap());
    let projection = store.search_projection_state().unwrap();
    assert_eq!(
        projection.service_state,
        SearchProjectionServiceState::Ready
    );
    assert_eq!(projection.repair_reason, None);
    assert!(projection.generation.is_some());
    assert!(!search_fulltext(&data_dir, "ActiveKillToken").is_empty());

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
fn foreground_import_scheduler_recovers_active_import_after_kill_and_restart() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-active-kill-data");
    let fixture_root = active_kill_fixture_root(ACTIVE_KILL_FIXTURE_FILE_COUNT);
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
    assert!(stdout.contains("import worker recovered orphaned running: 1"));
    assert!(stdout.contains("import worker recovered stale running: 0"));
    assert!(stdout.contains("import worker processed: 1"));
    assert!(stdout.contains(&format!(
        "import worker searchable documents: {ACTIVE_KILL_FIXTURE_FILE_COUNT}"
    )));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));
    assert!(!stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!search_fulltext(&data_dir, "ActiveKillToken").is_empty());

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let task = store.import_task_by_id(&task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(
        store.status_summary().unwrap().searchable_documents,
        ACTIVE_KILL_FIXTURE_FILE_COUNT as u64
    );

    remove_dir(&data_dir);
    remove_dir(&fixture_root);
}

#[test]
fn foreground_import_watcher_requeues_completed_root_after_file_change_without_path_leak() {
    serialize_windows_s4_daemon_test!();
    let data_dir = temp_dir("daemon-import-watcher-data");
    let watched_root = temp_dir("daemon-import-watcher-root");
    let watched_file = watched_root.join("candidate.txt");
    fs::write(
        &watched_file,
        "SUMMARY\nInitial watcher candidate.\nEXPERIENCE\nBuilt Rust backend services.\nSKILLS\nRust",
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
    let stdout = child.stdout.take().expect("daemon stdout");
    let mut stdout = spawn_daemon_stdout_collector(stdout);
    wait_until_foreground_ready(&mut child, &mut stdout);
    std::thread::sleep(Duration::from_millis(250));
    for attempt in 0..5 {
        fs::write(
            &watched_file,
            format!(
                "SUMMARY\nWatcherUpdatedToken refreshed candidate attempt {attempt}.\nEXPERIENCE\nBuilt Rust backend services for the watcher.\nSKILLS\nRust"
            ),
        )
        .unwrap();
        fs::write(
            watched_root.join(format!("candidate-extra-{attempt}.txt")),
            format!(
                "SUMMARY\nWatcherUpdatedToken extra candidate attempt {attempt}.\nEXPERIENCE\nBuilt Rust backend services for the watcher.\nSKILLS\nRust"
            ),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(100));
    }

    let output = wait_daemon(child, stdout);
    assert!(
        output.success,
        "stdout:\n{}\nstderr:\n{}",
        output.stdout, output.stderr
    );
    assert!(output.stderr.is_empty());
    assert!(
        output.stdout.contains("import watcher active roots: 1"),
        "stdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    assert!(
        output.stdout.contains("import watcher requeued imports: 1"),
        "stdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
    assert!(
        output.stdout.contains("import worker processed: 1"),
        "stdout:\n{}\nstderr:\n{}",
        output.stdout,
        output.stderr
    );
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
    let store = open_owned_store(data_dir);
    insert_queued_import_task(&store, label, canonical_root, queued_at_seconds)
}

fn initialize_empty_ready_store(data_dir: &Path) {
    let empty_root = data_dir.join("synthetic-empty-corpus");
    fs::create_dir_all(&empty_root).unwrap();
    let canonical_empty_root = fs::canonicalize(&empty_root).unwrap();
    seed_queued_import_task(
        data_dir,
        "initialize-empty-ready-store",
        &canonical_empty_root,
        1_700_000_000,
    );
    run_import_worker_once(data_dir);
    let store = ReadMetaStore::open_data_dir(data_dir).unwrap();
    assert_eq!(
        store.search_projection_state().unwrap().service_state,
        SearchProjectionServiceState::Ready
    );
}

fn seed_queued_import_task_in_ready_store(
    data_dir: &Path,
    label: &str,
    canonical_root: &Path,
    queued_at_seconds: i64,
) -> ImportTaskId {
    let store = open_owned_store(data_dir);
    insert_queued_import_task(&store, label, canonical_root, queued_at_seconds)
}

fn acquire_data_directory_owner(data_dir: &Path) -> DataDirectoryOwnerLease {
    match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test data dir is already owned"),
    }
}

fn open_owned_store(data_dir: &Path) -> OwnedMetaStore {
    acquire_data_directory_owner(data_dir).open_store().unwrap()
}

fn insert_queued_import_task(
    store: &OwnedMetaStore,
    label: &str,
    canonical_root: &Path,
    queued_at_seconds: i64,
) -> ImportTaskId {
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
    let contract = support::activate_default_processing_contract(store, now);
    store
        .insert_import_task_with_scan_scope(&task, &scope, &contract)
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
                "SUMMARY\nSynthetic resume {index}.\nEXPERIENCE\nBuilt ActiveKillToken local-first search services. {}\nSKILLS\nRust Java Kubernetes\n",
                "local-first search ".repeat(48)
            ),
        )
        .unwrap();
    }
    root
}

fn seed_legacy_search_artifact_layout(data_dir: &Path) {
    let search_root = data_dir.join("search-index");
    let staging = search_root.join("staging");
    let snapshots = search_root.join("snapshots");
    let vector_root = data_dir.join("vector-index");
    fs::create_dir_all(&staging).unwrap();
    fs::create_dir_all(&snapshots).unwrap();
    fs::create_dir_all(&vector_root).unwrap();
    fs::write(search_root.join("fulltext.snapshot.key-v1"), b"legacy").unwrap();
    fs::write(search_root.join("snapshot-readers.lock"), []).unwrap();
    fs::write(vector_root.join("vector.snapshot.key-v1"), b"legacy").unwrap();
    fs::write(vector_root.join("vector.lock"), []).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        for path in [&search_root, &staging, &snapshots, &vector_root] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        fs::set_permissions(
            search_root.join("snapshot-readers.lock"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        fs::set_permissions(
            vector_root.join("vector.lock"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
    }
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

fn search_fulltext(data_dir: &Path, query: &str) -> Vec<search_runtime::FullTextCandidate> {
    let mut coordinator = QueryCoordinator::open(data_dir).unwrap();
    coordinator
        .with_query(|scope| scope.fulltext_candidates(query, HitLimit::new(20)?, None))
        .expect("generation-pinned full-text query")
}

fn ready_generation(data_dir: &Path) -> String {
    let store = ReadMetaStore::open_data_dir(data_dir).unwrap();
    store.search_projection_state().unwrap().generation.unwrap()
}

fn request_daemon_import_cancellation(data_dir: &Path, task_id: &ImportTaskId) {
    let (endpoint, token) = read_daemon_ipc_endpoint(data_dir, "import_cancel");
    let rest = endpoint.strip_prefix("http://").expect("loopback endpoint");
    let (addr, _) = rest.split_once('/').expect("endpoint path");
    let body = serde_json::json!({ "task_id": task_id.to_string() }).to_string();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut stream = TcpStream::connect(addr).expect("connect daemon import cancellation");
        write!(
            stream,
            "POST /imports/cancel HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("request daemon import cancellation");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("read daemon import cancellation response");
        assert!(!response.contains(&token));
        assert!(!response.contains(path_str(data_dir)));
        if response.contains("HTTP/1.1 202 Accepted") {
            return;
        }
        assert!(
            response.contains("HTTP/1.1 503 Service Unavailable")
                && response.contains("METADATA_UNAVAILABLE"),
            "{response}"
        );
        assert!(
            Instant::now() < deadline,
            "daemon did not accept import cancellation before timeout"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn request_daemon_import(data_dir: &Path, root: &Path) -> ImportTaskId {
    let (endpoint, token) = read_daemon_ipc_endpoint(data_dir, "imports");
    let rest = endpoint.strip_prefix("http://").expect("loopback endpoint");
    let (addr, _) = rest.split_once('/').expect("endpoint path");
    let body = serde_json::json!({
        "roots": [path_str(root)],
        "profile": "explicit",
        "max_files": serde_json::Value::Null,
    })
    .to_string();
    let mut stream = TcpStream::connect(addr).expect("connect daemon import endpoint");
    write!(
        stream,
        "POST /imports HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("request daemon import");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read daemon import response");
    assert!(response.contains("HTTP/1.1 202 Accepted"), "{response}");
    assert!(!response.contains(&token));
    assert!(!response.contains(path_str(data_dir)));
    let payload: serde_json::Value = serde_json::from_str(
        response
            .split("\r\n\r\n")
            .nth(1)
            .expect("daemon import response body"),
    )
    .expect("parse daemon import response");
    payload["task_ids"][0]
        .as_str()
        .expect("daemon import task id")
        .parse()
        .expect("parse daemon import task id")
}

fn request_daemon_status(data_dir: &Path) {
    let (endpoint, token) = read_daemon_ipc_endpoint(data_dir, "status");
    let rest = endpoint.strip_prefix("http://").expect("loopback endpoint");
    let (addr, _) = rest.split_once('/').expect("endpoint path");
    let mut stream = TcpStream::connect(addr).expect("connect daemon status endpoint");
    write!(
        stream,
        "GET /status HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    )
    .expect("request daemon status");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read daemon status response");
    assert!(response.contains("HTTP/1.1 200 OK"), "{response}");
    assert!(!response.contains(&token));
    assert!(!response.contains(path_str(data_dir)));
}

fn read_daemon_ipc_endpoint(data_dir: &Path, endpoint_name: &str) -> (String, String) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let endpoint = fs::read(data_dir.join("ipc.endpoints.json"))
            .ok()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(&body).ok())
            .and_then(|manifest| manifest[endpoint_name].as_str().map(str::to_string));
        let token = fs::read(data_dir.join("ipc.auth"))
            .ok()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(&body).ok())
            .and_then(|manifest| manifest["token"].as_str().map(str::to_string));
        if let (Some(endpoint), Some(token)) = (endpoint, token) {
            return (endpoint, token);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("daemon IPC owner files were not published before timeout");
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

struct DaemonStdoutCollector {
    ready: Receiver<()>,
    join: Option<JoinHandle<String>>,
}

fn spawn_daemon_stdout_collector(stdout: ChildStdout) -> DaemonStdoutCollector {
    let (ready_sender, ready_receiver) = mpsc::channel();
    let join = thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut output = String::new();
        let mut line = String::new();
        let mut ready_sent = false;
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => return output,
                Ok(_) => {
                    output.push_str(&line);
                    if !ready_sent && output.contains("resume-daemon foreground ready") {
                        let _ = ready_sender.send(());
                        ready_sent = true;
                    }
                }
                Err(error) => panic!("read daemon stdout: {error}"),
            }
        }
    });
    DaemonStdoutCollector {
        ready: ready_receiver,
        join: Some(join),
    }
}

impl DaemonStdoutCollector {
    fn finish(&mut self) -> String {
        self.join
            .take()
            .expect("daemon stdout join handle")
            .join()
            .expect("join daemon stdout collector")
    }
}

trait TestDaemonProcess {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    fn terminate(&mut self);
}

impl TestDaemonProcess for Child {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        Child::try_wait(self)
    }

    fn terminate(&mut self) {
        let _ = self.kill();
        let _ = self.wait();
    }
}

impl TestDaemonProcess for ContainedChild {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        ContainedChild::try_wait(self)
    }

    fn terminate(&mut self) {
        ContainedChild::terminate(self);
    }
}

fn wait_until_foreground_ready(child: &mut Child, stdout: &mut DaemonStdoutCollector) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match stdout.ready.recv_timeout(Duration::from_millis(25)) {
            Ok(()) => return,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {}
        }
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            let stdout_text = stdout.finish();
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .expect("daemon stderr")
                .read_to_string(&mut stderr)
                .expect("read daemon stderr");
            panic!(
                "daemon exited before foreground ready: {status}\nstdout:\n{stdout_text}\nstderr:\n{stderr}"
            );
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let stdout_text = stdout.finish();
    panic!("daemon did not report foreground ready before timeout\nstdout:\n{stdout_text}");
}

fn wait_until_contained_foreground_ready(
    child: &mut ContainedChild,
    stdout: &mut DaemonStdoutCollector,
    stderr: &mut std::process::ChildStderr,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match stdout.ready.recv_timeout(Duration::from_millis(25)) {
            Ok(()) => return,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {}
        }
        if let Some(status) = child.try_wait().expect("poll contained daemon child") {
            let stdout_text = stdout.finish();
            let mut stderr_text = String::new();
            stderr
                .read_to_string(&mut stderr_text)
                .expect("read contained daemon stderr");
            panic!(
                "contained daemon exited before foreground ready: {status}\nstdout:\n{stdout_text}\nstderr:\n{stderr_text}"
            );
        }
    }

    child.terminate();
    let stdout_text = stdout.finish();
    panic!(
        "contained daemon did not report foreground ready before timeout\nstdout:\n{stdout_text}"
    );
}

fn wait_until_import_task_running(
    child: &mut impl TestDaemonProcess,
    data_dir: &Path,
    task_id: &ImportTaskId,
) {
    let store = ReadMetaStore::open_data_dir(data_dir).unwrap();
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

    child.terminate();
    panic!("import task did not enter running before timeout");
}

fn wait_until_import_task_completed(
    child: &mut impl TestDaemonProcess,
    data_dir: &Path,
    task_id: &ImportTaskId,
) {
    let store = ReadMetaStore::open_data_dir(data_dir).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            panic!("daemon exited before import task completed: {status}");
        }
        let task = store.import_task_by_id(task_id).unwrap().unwrap();
        match task.status {
            ImportTaskStatus::Completed => return,
            ImportTaskStatus::FailedRetryable | ImportTaskStatus::FailedPermanent => {
                panic!("import task failed before completion: {:?}", task.status)
            }
            ImportTaskStatus::Queued | ImportTaskStatus::Running => {}
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    child.terminate();
    panic!("import task did not complete before timeout");
}

fn wait_until_replacement_import_completed(
    child: &mut impl TestDaemonProcess,
    data_dir: &Path,
    cancelled_task_id: &ImportTaskId,
    canonical_root: &Path,
) -> ImportTaskId {
    let store = ReadMetaStore::open_data_dir(data_dir).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            panic!("daemon exited before replacement import completed: {status}");
        }
        if let Some(task) = store
            .latest_import_task_by_root(path_str(canonical_root))
            .unwrap()
            .filter(|task| task.id != *cancelled_task_id)
        {
            match task.status {
                ImportTaskStatus::Completed
                    if store.search_projection_state().unwrap().service_state
                        == SearchProjectionServiceState::Ready =>
                {
                    return task.id;
                }
                ImportTaskStatus::Completed => {}
                ImportTaskStatus::FailedRetryable | ImportTaskStatus::FailedPermanent => {
                    panic!("replacement import failed: {:?}", task.status)
                }
                ImportTaskStatus::Queued | ImportTaskStatus::Running => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    child.terminate();
    panic!("replacement import did not complete before timeout");
}

fn wait_until_search_projection_ready(child: &mut impl TestDaemonProcess, data_dir: &Path) {
    let store = ReadMetaStore::open_data_dir(data_dir).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            panic!("daemon exited before search projection became ready: {status}");
        }
        let state = store.search_projection_state().unwrap();
        match state.service_state {
            SearchProjectionServiceState::Ready => return,
            SearchProjectionServiceState::RepairBlocked => {
                panic!("search projection became repair-blocked")
            }
            SearchProjectionServiceState::Repairing => {}
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    child.terminate();
    panic!("search projection did not become ready before timeout");
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

fn wait_daemon(mut child: Child, mut stdout: DaemonStdoutCollector) -> DaemonOutput {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = child.try_wait().expect("poll daemon child") {
            let stdout_text = stdout.finish();
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

fn wait_contained_daemon(
    mut child: ContainedChild,
    mut stdout: DaemonStdoutCollector,
    mut stderr: std::process::ChildStderr,
) -> DaemonOutput {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = child.try_wait().expect("poll contained daemon child") {
            let stdout_text = stdout.finish();
            let mut stderr_text = String::new();
            stderr
                .read_to_string(&mut stderr_text)
                .expect("read contained daemon stderr");
            return DaemonOutput {
                success: status.success(),
                stdout: stdout_text,
                stderr: stderr_text,
            };
        }
        if Instant::now() >= deadline {
            child.terminate();
            panic!("contained daemon did not exit after lifecycle EOF");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}
