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
    assert!(stdout.contains("min-samples 1000"));
    assert!(stdout.contains("precision/recall/F1 >= 0.93"));
    assert!(stdout.contains("dedupe quality: blocked"));
    assert!(stdout.contains("min-pairs 1000"));
    assert!(stdout.contains("min-positive-pairs 100"));
    assert!(stdout.contains("precision/recall/F1 >= 0.90"));
    assert!(stdout.contains("vector quality: blocked"));
    assert!(stdout.contains("recall@k >= 0.90"));
    assert!(stdout.contains("MRR >= 0.85"));
    assert!(stdout.contains("NDCG@k >= 0.90"));
    assert!(stdout.contains("OCR throughput: blocked"));
    assert!(stdout.contains("min-pages 500"));
    assert!(stdout.contains("OCR p95 <= 1000ms"));
    assert!(stdout.contains("pages_per_second >= 1"));
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

    let field_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "field extraction quality")
        .expect("field quality blocker");
    let field_detail = field_blocker["detail"].as_str().unwrap();
    assert!(field_detail.contains("min-samples 1000"));
    assert!(field_detail.contains("precision/recall/F1 >= 0.93"));

    let dedupe_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "dedupe quality")
        .expect("dedupe quality blocker");
    let dedupe_detail = dedupe_blocker["detail"].as_str().unwrap();
    assert!(dedupe_detail.contains("min-pairs 1000"));
    assert!(dedupe_detail.contains("min-positive-pairs 100"));
    assert!(dedupe_detail.contains("precision/recall/F1 >= 0.90"));

    let vector_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "vector quality")
        .expect("vector quality blocker");
    let vector_detail = vector_blocker["detail"].as_str().unwrap();
    assert!(vector_detail.contains("min-samples 1000"));
    assert!(vector_detail.contains("recall@k >= 0.90"));
    assert!(vector_detail.contains("MRR >= 0.85"));
    assert!(vector_detail.contains("NDCG@k >= 0.90"));

    let ocr_blocker = blockers
        .iter()
        .find(|blocker| blocker["label"] == "OCR throughput")
        .expect("OCR throughput blocker");
    let ocr_detail = ocr_blocker["detail"].as_str().unwrap();
    assert!(ocr_detail.contains("min-pages 500"));
    assert!(ocr_detail.contains("OCR p95 <= 1000ms"));
    assert!(ocr_detail.contains("pages_per_second >= 1"));

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
