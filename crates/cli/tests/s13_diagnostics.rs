use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn doctor_reports_no_index_without_path_or_fake_benchmark() {
    let data_dir = temp_path("doctor-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-ir doctor"));
    assert!(stdout.contains("metadata: ok"));
    assert!(stdout.contains("search index: unavailable"));
    assert!(stdout.contains("query smoke: skipped (no full-text index)"));
    assert!(stdout.contains("fault simulations: available"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains("p95"));

    remove_dir(&data_dir);
}

#[test]
fn doctor_handles_corrupt_index_snapshot_without_path_leak() {
    let data_dir = temp_dir("doctor-corrupt-private-data");
    let index_dir = data_dir.join("search-index");
    fs::create_dir_all(&index_dir).unwrap();
    fs::write(index_dir.join("meta.json"), b"not a tantivy index").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with corrupt index");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("search index: corrupt"));
    assert!(stdout.contains("query smoke: skipped (index unavailable)"));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[test]
fn export_diagnostics_redact_outputs_skeleton_without_paths() {
    let data_dir = temp_path("diagnostics-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\": \"diagnostics.v1\""));
    assert!(stdout.contains("\"redacted\": true"));
    assert!(stdout.contains("\"raw_paths\": \"<redacted>\""));
    assert!(stdout.contains("\"search_index_state\": \"unavailable\""));
    assert!(stdout.contains("\"daemon_restart\""));
    assert!(stdout.contains("\"disk_space_low\""));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s13-cli-{label}-{unique}"))
}

fn temp_dir(label: &str) -> PathBuf {
    let path = temp_path(label);
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
