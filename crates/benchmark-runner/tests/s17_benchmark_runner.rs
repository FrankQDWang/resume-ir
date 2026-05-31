use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use benchmark_runner::{run_synthetic_query_benchmark, SyntheticBenchmarkConfig};

#[test]
fn synthetic_query_benchmark_reports_real_percentiles_without_raw_text() {
    let index_dir = temp_dir("synthetic-query-index");
    let config = SyntheticBenchmarkConfig::new(32, 9).unwrap().with_top_k(7);

    let report = run_synthetic_query_benchmark(&index_dir, config).unwrap();

    assert_eq!(report.dataset_kind(), "synthetic");
    assert_eq!(report.document_count(), 32);
    assert_eq!(report.query_count(), 9);
    assert_eq!(report.top_k(), 7);
    assert_eq!(report.latency().samples(), 9);
    assert!(report.latency().p95_ms() >= report.latency().p50_ms());
    assert!(report.qps() > 0.0);
    assert!(report.index_size_bytes() > 0);
    assert_eq!(report.percentile_confidence(), "smoke");
    assert!(!report.million_scale_verified());

    let json = report.to_redacted_json();
    assert!(json.contains("\"run_id\":\"bench_"));
    assert!(json.contains("\"platform\":"));
    assert!(json.contains("\"dataset_kind\":\"synthetic\""));
    assert!(json.contains("\"index_size_bytes\":"));
    assert!(json.contains("\"qps\":"));
    assert!(json.contains("\"zero_result_queries\":"));
    assert!(json.contains("\"percentile_confidence\":\"smoke\""));
    assert!(json.contains("\"million_scale_verified\":false"));
    assert!(json.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!json.contains("Synthetic Candidate"));
    assert!(!json.contains("payment gateway"));
    assert!(!json.contains(path_str(&index_dir)));

    remove_dir(&index_dir);
}

#[test]
fn synthetic_query_benchmark_rejects_empty_workloads() {
    assert!(SyntheticBenchmarkConfig::new(0, 1).is_err());
    assert!(SyntheticBenchmarkConfig::new(1, 0).is_err());
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
