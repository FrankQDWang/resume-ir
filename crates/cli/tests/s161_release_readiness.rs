use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn release_readiness_reports_blocked_evidence_without_local_path_leaks() {
    let data_dir = temp_path("release-readiness-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "release-readiness"])
        .output()
        .expect("run release readiness gate");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("resume-ir release readiness"));
    assert!(stdout.contains("stable release: blocked"));
    assert!(stdout.contains("signing certificates: blocked"));
    assert!(stdout.contains("macOS notarization: blocked"));
    assert!(stdout.contains("Windows installer lifecycle: blocked"));
    assert!(stdout.contains("Windows service lifecycle: blocked"));
    assert!(stdout.contains("macOS installer lifecycle: blocked"));
    assert!(stdout.contains("100k/1M real-corpus benchmarks: blocked"));
    assert!(stdout.contains("hot-index hybrid"));
    assert!(stdout.contains("500 query samples"));
    assert!(stdout.contains("percentile_confidence: release"));
    assert!(stdout.contains("--require-million-scale"));
    assert!(stdout.contains("field extraction quality: blocked"));
    assert!(stdout.contains("dedupe quality: blocked"));
    assert!(stdout.contains("vector quality: blocked"));
    assert!(stdout.contains("OCR throughput: blocked"));
    assert!(stdout.contains("OCR engine license/distribution: blocked"));
    assert!(stdout.contains("embedding model license/distribution: blocked"));
    assert!(stdout.contains("cross-platform release validation: blocked"));
    assert!(stdout.contains("hardware fault drills: blocked"));
    assert!(stdout.contains("actual ENOSPC"));
    assert!(stdout.contains("service-level daemon kill"));
    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stdout.contains("PRIVATE"));
}

#[test]
fn release_readiness_json_reports_blockers_without_local_path_leaks() {
    let data_dir = temp_path("release-readiness-json-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "release-readiness",
            "--json",
        ])
        .output()
        .expect("run release readiness json gate");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("release readiness json report");

    assert_eq!(report["schema_version"], "release-readiness.v1");
    assert_eq!(report["stable_release"], "blocked");
    assert_eq!(report["local_dry_run_artifacts"], "evidence_only");
    assert_eq!(
        report["next_gate"],
        "keep release blocked until every item has current local evidence"
    );

    let blockers = report["blockers"].as_array().expect("blockers array");
    assert_eq!(blockers.len(), 14);
    let labels = blockers
        .iter()
        .map(|blocker| blocker["label"].as_str().expect("blocker label"))
        .collect::<Vec<_>>();
    assert!(labels.contains(&"signing certificates"));
    assert!(labels.contains(&"macOS notarization"));
    assert!(labels.contains(&"Windows installer lifecycle"));
    assert!(labels.contains(&"Windows service lifecycle"));
    assert!(labels.contains(&"macOS installer lifecycle"));
    assert!(labels.contains(&"100k/1M real-corpus benchmarks"));
    assert!(labels.contains(&"field extraction quality"));
    assert!(labels.contains(&"dedupe quality"));
    assert!(labels.contains(&"vector quality"));
    assert!(labels.contains(&"OCR throughput"));
    assert!(labels.contains(&"OCR engine license/distribution"));
    assert!(labels.contains(&"embedding model license/distribution"));
    assert!(labels.contains(&"cross-platform release validation"));
    assert!(labels.contains(&"hardware fault drills"));
    for blocker in blockers {
        assert_eq!(blocker["status"], "blocked");
        assert!(blocker["detail"].as_str().expect("blocker detail").len() > 12);
    }
    let fault_drill_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "hardware fault drills")
        .expect("hardware fault drills blocker");
    let fault_drill_detail = fault_drill_blocker["detail"].as_str().unwrap();
    assert!(fault_drill_detail.contains("actual ENOSPC"));
    assert!(fault_drill_detail.contains("service-level daemon kill"));

    let benchmark_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "100k/1M real-corpus benchmarks")
        .expect("benchmark blocker");
    let benchmark_detail = benchmark_blocker["detail"].as_str().unwrap();
    assert!(benchmark_detail.contains("hot-index hybrid"));
    assert!(benchmark_detail.contains("500 query samples"));
    assert!(benchmark_detail.contains("percentile_confidence: release"));
    assert!(benchmark_detail.contains("--require-million-scale"));

    assert!(stderr.contains("release readiness blocked"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&data_dir)));
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-{label}-{unique}"))
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("test paths are utf-8")
}
