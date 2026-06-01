use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn embed_worker_without_command_reports_blocked_without_path_leak() {
    let data_dir = temp_dir("embed-worker-no-command-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "embed-worker", "--once"])
        .output()
        .expect("run embed worker without command");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("embedding worker blocked: local embedding command not configured"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&fixture_root)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn embed_worker_runs_local_command_and_persists_vector_snapshot_without_hiding_search_results() {
    let data_dir = temp_dir("embed-worker-command-data");
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-embedding-worker",
        r#"#!/bin/sh
if [ ! -s "$RESUME_IR_EMBEDDING_INPUT_PATH" ]; then
  exit 7
fi
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { sub(/^input=/, "", $1); printf "vector=%s\t0.5,0.5,0.5,0.5\n", $1 }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
printf 'metadata=synthetic-fixture\n'
"#,
    );
    import_fixtures(&data_dir, &fixture_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "embed-worker",
            "--once",
            "--command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "4",
            "--max-docs",
            "8",
            "--max-text-bytes",
            "100000",
        ])
        .output()
        .expect("run embed worker with local command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("embedding worker: completed"));
    assert!(stdout.contains("model id: fixture-local-model"));
    assert!(stdout.contains("dimension: 4"));
    assert!(stdout.contains("documents considered: 2"));
    assert!(stdout.contains("documents embedded: 2"));
    assert!(stdout.contains("vector index: available (vector snapshot)"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("searchable documents: 2"));
    assert!(status_stdout.contains("vector index: available (vector snapshot)"));
    assert!(status_stdout.contains("vector index vectors: 2"));
    assert!(!status_stdout.contains(path_str(&data_dir)));
    assert!(!status_stdout.contains(path_str(&fixture_root)));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after embedding");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 2"));
    assert!(search_stdout.contains("synthetic-java-platform.pdf"));
    assert!(search_stdout.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
}

fn import_fixtures(data_dir: &Path, fixture_root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(fixture_root),
        ])
        .output()
        .expect("import fixtures");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
    let path = std::env::temp_dir().join(format!("resume-ir-s39-cli-{label}-{unique}"));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

#[cfg(unix)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = temp_dir("embed-worker-command-bin");
    let path = directory.join(name);
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}
