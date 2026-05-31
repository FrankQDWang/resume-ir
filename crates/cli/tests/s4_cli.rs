use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn status_creates_store_and_reports_empty_aggregates() {
    let data_dir = temp_dir("status-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("indexed documents: 0"));
    assert!(stdout.contains("search index: unavailable"));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[test]
fn import_root_submits_persistent_task_without_path_leak() {
    let data_dir = temp_dir("import-data");
    let root_dir = temp_dir("import-root-private-name");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli import");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import task submitted"));
    assert!(stdout.contains("task id: imp_"));
    assert!(stdout.contains("status: completed"));
    assert!(stdout.contains("files discovered: 0"));
    assert!(!stdout.contains(path_str(&root_dir)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status after import");
    assert!(status.status.success());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("import tasks queued: 0"));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn search_without_index_returns_unavailable_message_without_echoing_query() {
    let data_dir = temp_path("search-data");
    let sensitive_query = "Java PRIVATE_TOKEN";

    assert!(!data_dir.exists());

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", sensitive_query])
        .output()
        .expect("run resume-cli search");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("search index not available yet"));
    assert!(stdout.contains("results: 0"));
    assert!(!stdout.contains(sensitive_query));
    assert!(!data_dir.exists());

    remove_dir(&data_dir);
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s4-cli-{label}-{unique}"))
}

fn temp_dir(label: &str) -> PathBuf {
    let path = temp_path(label);
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}
