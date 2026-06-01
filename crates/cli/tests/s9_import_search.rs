use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{ImportTask, ImportTaskId, ImportTaskStatus, MetaStore, UnixTimestamp};

const LOCAL_DISCOVERY_ROOTS_ENV: &str = "RESUME_IR_LOCAL_DISCOVERY_ROOTS";

#[test]
fn import_fixtures_builds_searchable_index_and_reopens_snapshot() {
    let data_dir = temp_dir("import-search-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
        ])
        .output()
        .expect("run resume-cli import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("import task submitted"));
    assert!(import_stdout.contains("task id: imp_"));
    assert!(import_stdout.contains("status: completed"));
    assert!(import_stdout.contains("files discovered: 3"));
    assert!(import_stdout.contains("searchable documents: 2"));
    assert!(import_stdout.contains("ocr required documents: 1"));
    assert!(!import_stdout.contains(path_str(&fixture_root)));
    assert!(!import_stdout.contains(path_str(&canonical_fixture_root)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("searchable documents: 2"));
    assert!(status_stdout.contains("ocr queue: 1"));
    assert!(status_stdout.contains("import tasks queued: 0"));
    assert!(status_stdout.contains("index health: ready"));
    assert!(status_stdout.contains("search index: available (full-text snapshot)"));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 2"));
    assert!(search_stdout.contains("rank: 1"));
    assert!(search_stdout.contains("synthetic-java-platform.pdf"));
    assert!(search_stdout.contains("synthetic-java-engineer.docx"));
    assert!(!search_stdout.contains("query:"));

    let reopened_search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after reopen");
    assert!(reopened_search.status.success());
    let reopened_stdout = String::from_utf8_lossy(&reopened_search.stdout);
    assert!(reopened_stdout.contains("rank: 1"));

    let empty_root = temp_dir("empty-import-root");
    let empty_import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&empty_root),
        ])
        .output()
        .expect("run resume-cli import empty root");
    assert!(empty_import.status.success());
    let empty_import_stdout = String::from_utf8_lossy(&empty_import.stdout);
    assert!(empty_import_stdout.contains("files discovered: 0"));
    assert!(!empty_import_stdout.contains(path_str(&empty_root)));

    let search_after_empty_import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after empty import");
    assert!(search_after_empty_import.status.success());
    let search_after_empty_stdout = String::from_utf8_lossy(&search_after_empty_import.stdout);
    assert!(search_after_empty_stdout.contains("results: 2"));
    assert!(search_after_empty_stdout.contains("synthetic-java-platform.pdf"));
    assert!(search_after_empty_stdout.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
    remove_dir(&empty_root);
}

#[test]
fn import_multiple_roots_builds_searchable_index_without_path_leak() {
    let data_dir = temp_dir("multi-root-import-data");
    let first_root = temp_dir("multi-root-a-private");
    let second_root = temp_dir("multi-root-b-private");
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        first_root.join("synthetic-java-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        second_root.join("synthetic-java-engineer.docx"),
    )
    .unwrap();
    let canonical_first_root = fs::canonicalize(&first_root).unwrap();
    let canonical_second_root = fs::canonicalize(&second_root).unwrap();

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&first_root),
            "--root",
            path_str(&second_root),
        ])
        .output()
        .expect("run resume-cli multi-root import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("roots scanned: 2"));
    assert!(import_stdout.contains("files discovered: 2"));
    assert!(import_stdout.contains("searchable documents: 2"));
    assert!(!import_stdout.contains(path_str(&first_root)));
    assert!(!import_stdout.contains(path_str(&second_root)));
    assert!(!import_stdout.contains(path_str(&canonical_first_root)));
    assert!(!import_stdout.contains(path_str(&canonical_second_root)));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after multi-root import");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 2"));
    assert!(search_stdout.contains("synthetic-java-platform.pdf"));
    assert!(search_stdout.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
    remove_dir(&first_root);
    remove_dir(&second_root);
}

#[test]
fn local_discovery_root_preset_uses_discovery_profile_without_path_leak() {
    let data_dir = temp_dir("local-discovery-import-data");
    let local_root = temp_dir("local-discovery-private-root");
    let canonical_local_root = fs::canonicalize(&local_root).unwrap();
    fs::create_dir_all(local_root.join("Documents")).unwrap();
    fs::create_dir_all(local_root.join("node_modules")).unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-platform.pdf"),
        local_root
            .join("Documents")
            .join("synthetic-java-platform.pdf"),
    )
    .unwrap();
    fs::copy(
        fixture_root().join("synthetic-java-engineer.docx"),
        local_root
            .join("node_modules")
            .join("synthetic-java-engineer.docx"),
    )
    .unwrap();

    let root_override = std::env::join_paths([&local_root]).unwrap();
    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env(LOCAL_DISCOVERY_ROOTS_ENV, root_override)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root-preset",
            "local-discovery",
        ])
        .output()
        .expect("run local discovery preset import");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("scan profile: discovery"));
    assert!(import_stdout.contains("roots scanned: 1"));
    assert!(import_stdout.contains("files discovered: 1"));
    assert!(import_stdout.contains("searchable documents: 1"));
    assert!(!import_stdout.contains(path_str(&local_root)));
    assert!(!import_stdout.contains(path_str(&canonical_local_root)));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after local discovery import");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 1"));
    assert!(search_stdout.contains("synthetic-java-platform.pdf"));
    assert!(!search_stdout.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
    remove_dir(&local_root);
}

#[test]
fn import_reuses_recoverable_task_after_restart() {
    let data_dir = temp_dir("import-restart-data");
    let fixture_root = fixture_root();
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let pending_task_id = seed_retryable_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
        ])
        .output()
        .expect("run resume-cli import after restart");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains("status: completed"));
    assert!(import_stdout.contains(&pending_task_id.to_string()));
    assert!(!import_stdout.contains(path_str(&fixture_root)));
    assert!(!import_stdout.contains(path_str(&canonical_fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().import_tasks_recoverable, 0);

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after recovered import");
    assert!(search.status.success());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 2"));

    remove_dir(&data_dir);
}

#[test]
fn import_does_not_take_over_live_running_task() {
    let data_dir = temp_dir("import-live-running-data");
    let fixture_root = fixture_root();
    let pending_task_id = seed_live_running_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
        ])
        .output()
        .expect("run resume-cli import while task is live");

    assert!(!import.status.success());
    assert!(import.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("import task is already running"));
    assert!(!stderr.contains(path_str(&fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Running);

    remove_dir(&data_dir);
}

#[test]
fn discovery_import_does_not_take_over_live_running_task_for_same_root() {
    let data_dir = temp_dir("discovery-import-live-running-data");
    let fixture_root = fixture_root();
    let pending_task_id = seed_live_running_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
            "--profile",
            "discovery",
        ])
        .output()
        .expect("run discovery import while task is live");

    assert!(!import.status.success());
    assert!(import.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("import task is already running"));
    assert!(!stderr.contains(path_str(&fixture_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Running);

    remove_dir(&data_dir);
}

#[test]
fn multi_root_import_does_not_take_over_live_running_task_for_any_root() {
    let data_dir = temp_dir("multi-root-live-running-data");
    let fixture_root = fixture_root();
    let second_root = temp_dir("multi-root-live-second");
    let pending_task_id = seed_live_running_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
            "--root",
            path_str(&second_root),
        ])
        .output()
        .expect("run multi-root import while one task is live");

    assert!(!import.status.success());
    assert!(import.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(stderr.contains("import task is already running"));
    assert!(!stderr.contains(path_str(&fixture_root)));
    assert!(!stderr.contains(path_str(&second_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Running);

    remove_dir(&data_dir);
    remove_dir(&second_root);
}

#[test]
fn multi_root_import_reuses_recoverable_task_for_each_root() {
    let data_dir = temp_dir("multi-root-recoverable-data");
    let fixture_root = fixture_root();
    let second_root = temp_dir("multi-root-recoverable-second");
    let canonical_fixture_root = fs::canonicalize(&fixture_root).unwrap();
    let canonical_second_root = fs::canonicalize(&second_root).unwrap();
    let pending_task_id = seed_retryable_import_task(&data_dir, &fixture_root);

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
            "--root",
            path_str(&second_root),
        ])
        .output()
        .expect("run multi-root import after restart");

    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());
    let import_stdout = String::from_utf8_lossy(&import.stdout);
    assert!(import_stdout.contains(&pending_task_id.to_string()));
    assert!(import_stdout.contains("roots scanned: 2"));
    assert!(!import_stdout.contains(path_str(&fixture_root)));
    assert!(!import_stdout.contains(path_str(&second_root)));
    assert!(!import_stdout.contains(path_str(&canonical_fixture_root)));
    assert!(!import_stdout.contains(path_str(&canonical_second_root)));

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let task = store.import_task_by_id(&pending_task_id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Completed);
    assert_eq!(store.status_summary().unwrap().import_tasks_recoverable, 0);

    remove_dir(&data_dir);
    remove_dir(&second_root);
}

fn seed_retryable_import_task(data_dir: &Path, fixture_root: &Path) -> ImportTaskId {
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let queued_at = UnixTimestamp::from_unix_seconds(1_700_000_000);
    let started_at = UnixTimestamp::from_unix_seconds(1_700_000_010);
    let finished_at = UnixTimestamp::from_unix_seconds(1_700_000_020);
    let id = ImportTaskId::from_non_secret_parts(&["s9", "recoverable-import-task"]);
    let canonical_root = fs::canonicalize(fixture_root).unwrap();
    store
        .insert_import_task(&ImportTask {
            id: id.clone(),
            root_path: path_str(&canonical_root).to_string(),
            status: ImportTaskStatus::FailedRetryable,
            queued_at,
            started_at: Some(started_at),
            finished_at: Some(finished_at),
            updated_at: finished_at,
        })
        .unwrap();
    id
}

fn seed_live_running_import_task(data_dir: &Path, fixture_root: &Path) -> ImportTaskId {
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let canonical_root = fs::canonicalize(fixture_root).unwrap();
    store
        .insert_import_task(&ImportTask {
            id: ImportTaskId::from_non_secret_parts(&["s9", "older-queued-import-task"]),
            root_path: path_str(&canonical_root).to_string(),
            status: ImportTaskStatus::Queued,
            queued_at: UnixTimestamp::from_unix_seconds(1_699_999_000),
            started_at: None,
            finished_at: None,
            updated_at: UnixTimestamp::from_unix_seconds(1_699_999_000),
        })
        .unwrap();

    let queued_at = UnixTimestamp::from_unix_seconds(1_700_000_000);
    let started_at = UnixTimestamp::from_unix_seconds(1_700_000_010);
    let id = ImportTaskId::from_non_secret_parts(&["s9", "live-running-import-task"]);
    store
        .insert_import_task(&ImportTask {
            id: id.clone(),
            root_path: path_str(&canonical_root).to_string(),
            status: ImportTaskStatus::Running,
            queued_at,
            started_at: Some(started_at),
            finished_at: None,
            updated_at: started_at,
        })
        .unwrap();
    id
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
    let path = std::env::temp_dir().join(format!("resume-ir-s9-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
