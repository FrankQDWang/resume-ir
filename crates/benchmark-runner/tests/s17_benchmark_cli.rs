use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn resume_benchmark_outputs_redacted_synthetic_json() {
    let index_dir = temp_dir("synthetic-query-cli");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "synthetic-query",
            "--index-dir",
            path_str(&index_dir),
            "--documents",
            "24",
            "--queries",
            "6",
            "--top-k",
            "5",
            "--json",
        ])
        .output()
        .expect("run resume-benchmark");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"benchmark.v1\""));
    assert!(stdout.contains("\"run_id\":\"bench_"));
    assert!(stdout.contains("\"platform\":"));
    assert!(stdout.contains("\"dataset_kind\":\"synthetic\""));
    assert!(stdout.contains("\"document_count\":24"));
    assert!(stdout.contains("\"query_count\":6"));
    assert!(stdout.contains("\"top_k\":5"));
    assert!(stdout.contains("\"index_size_bytes\":"));
    assert!(stdout.contains("\"qps\":"));
    assert!(stdout.contains("\"percentile_confidence\":\"smoke\""));
    assert!(stdout.contains("\"million_scale_verified\":false"));
    assert!(stdout.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!stdout.contains(path_str(&index_dir)));
    assert!(!stdout.contains("Synthetic Candidate"));
    assert!(!stdout.contains("payment gateway"));

    remove_dir(&index_dir);
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s17-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
