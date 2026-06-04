use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use benchmark_runner::{
    evaluate_benchmark_gate_json, evaluate_field_quality_gate_json,
    evaluate_ocr_throughput_gate_json, run_field_quality_jsonl,
    run_synthetic_ocr_throughput_benchmark, run_synthetic_query_benchmark, BenchmarkGateConfig,
    FieldQualityGateConfig, OcrThroughputGateConfig, SyntheticBenchmarkConfig,
    SyntheticOcrBenchmarkConfig, SyntheticOcrBenchmarkEngine,
};

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

#[test]
fn benchmark_gate_rejects_synthetic_report_without_explicit_scope() {
    let report = minimal_benchmark_json("synthetic", 1_000, 100, 25.0, 0, false);
    let config = BenchmarkGateConfig::new(1_000, 100, 50.0);

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("synthetic benchmark requires explicit allowance"));
}

#[test]
fn benchmark_gate_rejects_latency_regression() {
    let report = minimal_benchmark_json("synthetic", 1_000, 100, 251.0, 0, false);
    let config = BenchmarkGateConfig::new(1_000, 100, 250.0).allow_synthetic();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error.to_string().contains("query p95 exceeded threshold"));
}

#[test]
fn benchmark_gate_accepts_explicit_synthetic_smoke_without_scale_claim() {
    let report = minimal_benchmark_json("synthetic", 1_000, 100, 25.0, 0, false);
    let config = BenchmarkGateConfig::new(1_000, 100, 50.0).allow_synthetic();

    let evaluation = evaluate_benchmark_gate_json(&report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "synthetic");
    assert_eq!(evaluation.document_count(), 1_000);
    assert_eq!(evaluation.query_count(), 100);
    assert_eq!(evaluation.p95_ms(), 25.0);
}

#[test]
fn benchmark_gate_rejects_unproven_million_scale_claims() {
    let report = minimal_benchmark_json("synthetic", 1_000, 100, 25.0, 0, true);
    let config = BenchmarkGateConfig::new(1_000, 100, 50.0).allow_synthetic();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("million-scale claim is not proven"));
}

#[test]
fn field_quality_report_scores_labeled_samples_without_raw_value_leakage() {
    let dataset = concat!(
        "{\"sample_id\":\"case-a\",\"text\":\"Name: Synthetic Candidate\\nEmail: candidate@example.test\\nPhone: +1 (415) 555-0132\\nSkills: Rust, Java\\nBachelor of Science\",",
        "\"expected\":[",
        "{\"type\":\"name\",\"normalized\":\"synthetic candidate\"},",
        "{\"type\":\"email\",\"normalized\":\"candidate@example.test\"},",
        "{\"type\":\"phone\",\"normalized\":\"+14155550132\"},",
        "{\"type\":\"skill\",\"normalized\":\"Rust\"},",
        "{\"type\":\"skill\",\"normalized\":\"Java\"},",
        "{\"type\":\"degree\",\"normalized\":\"bachelor\"}",
        "]}\n",
        "{\"sample_id\":\"case-b\",\"text\":\"Education\\nSynthetic University\\nSkills: SQLite\",",
        "\"expected\":[",
        "{\"type\":\"school\",\"normalized\":\"synthetic university\"},",
        "{\"type\":\"skill\",\"normalized\":\"SQLite\"}",
        "]}\n",
    );

    let report = run_field_quality_jsonl(dataset).unwrap();

    assert_eq!(report.dataset_kind(), "labeled");
    assert_eq!(report.sample_count(), 2);
    assert_eq!(report.expected_mentions(), 8);
    assert!(report.overall().f1() >= 0.95);
    assert!(report.field_metric("email").unwrap().f1() >= 0.99);
    assert!(report.field_metric("phone").unwrap().f1() >= 0.99);
    assert!(report.field_metric("skill").unwrap().f1() >= 0.99);
    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"field-quality.v1\""));
    assert!(json.contains("\"dataset_kind\":\"labeled\""));
    assert!(json.contains("\"sample_count\":2"));
    assert!(json.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!json.contains("Synthetic Candidate"));
    assert!(!json.contains("candidate@example.test"));
    assert!(!json.contains("+1 (415) 555-0132"));
    assert!(!json.contains("+14155550132"));
    assert!(!json.contains("case-a"));
}

#[test]
fn field_quality_gate_rejects_low_recall_reports() {
    let dataset = concat!(
        "{\"text\":\"Skills: Rust\",",
        "\"expected\":[",
        "{\"type\":\"skill\",\"normalized\":\"Rust\"},",
        "{\"type\":\"skill\",\"normalized\":\"Kubernetes\"}",
        "]}\n",
    );
    let report = run_field_quality_jsonl(dataset).unwrap();
    let config = FieldQualityGateConfig::new(0.95, 0.95, 0.95).with_min_samples(1);

    let error = evaluate_field_quality_gate_json(&report.to_redacted_json(), config).unwrap_err();

    assert!(error.to_string().contains("field recall below threshold"));
}

#[test]
fn field_quality_gate_accepts_labeled_report() {
    let dataset = concat!(
        "{\"text\":\"Email: candidate@example.test\\nPhone: (415) 555-0132\",",
        "\"expected\":[",
        "{\"type\":\"email\",\"normalized\":\"candidate@example.test\"},",
        "{\"type\":\"phone\",\"normalized\":\"+14155550132\"}",
        "]}\n",
    );
    let report = run_field_quality_jsonl(dataset).unwrap();
    let config = FieldQualityGateConfig::new(0.99, 0.99, 0.99).with_min_samples(1);

    let evaluation = evaluate_field_quality_gate_json(&report.to_redacted_json(), config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "labeled");
    assert_eq!(evaluation.sample_count(), 1);
    assert!(evaluation.f1() >= 0.99);
}

#[test]
fn synthetic_ocr_throughput_reports_page_latency_without_payload_or_path_leakage() {
    let command = ocr_fixture_script("ocr-throughput-private-command");
    let config = SyntheticOcrBenchmarkConfig::new(3, 5_000).unwrap();
    let engine = SyntheticOcrBenchmarkEngine::local_command(&command).unwrap();

    let report = run_synthetic_ocr_throughput_benchmark(engine, config).unwrap();

    assert_eq!(report.dataset_kind(), "synthetic");
    assert_eq!(report.engine_kind(), "local-command");
    assert_eq!(report.page_count(), 3);
    assert_eq!(report.latency().samples(), 3);
    assert!(report.latency().p95_ms() >= report.latency().p50_ms());
    assert!(report.pages_per_second() > 0.0);
    assert!(report.total_page_bytes() > 0);
    assert!(report.total_text_bytes() > 0);

    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"ocr-throughput.v1\""));
    assert!(json.contains("\"dataset_kind\":\"synthetic\""));
    assert!(json.contains("\"engine_kind\":\"local-command\""));
    assert!(json.contains("\"page_count\":3"));
    assert!(json.contains("\"pages_per_second\":"));
    assert!(json.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("Synthetic OCR Candidate"));
    assert!(!json.contains("PRIVATE OCR PAYLOAD"));

    let _ = fs::remove_file(&command);
}

#[test]
fn synthetic_ocr_throughput_rejects_empty_workloads() {
    assert!(SyntheticOcrBenchmarkConfig::new(0, 5_000).is_err());
    assert!(SyntheticOcrBenchmarkConfig::new(1, 0).is_err());
}

#[test]
fn ocr_throughput_gate_rejects_synthetic_report_without_explicit_scope() {
    let report = minimal_ocr_throughput_json("synthetic", 25, 12.0, 8.5, "not_evaluated");
    let config = OcrThroughputGateConfig::new(25, 50.0, 1.0);

    let error = evaluate_ocr_throughput_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("synthetic OCR benchmark requires explicit allowance"));
}

#[test]
fn ocr_throughput_gate_accepts_explicit_synthetic_smoke_without_scale_claim() {
    let report = minimal_ocr_throughput_json("synthetic", 25, 12.0, 8.5, "not_evaluated");
    let config = OcrThroughputGateConfig::new(25, 50.0, 1.0).allow_synthetic();

    let evaluation = evaluate_ocr_throughput_gate_json(&report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "synthetic");
    assert_eq!(evaluation.page_count(), 25);
    assert_eq!(evaluation.p95_ms(), 12.0);
    assert_eq!(evaluation.pages_per_second(), 8.5);
}

fn minimal_benchmark_json(
    dataset_kind: &str,
    document_count: usize,
    query_count: usize,
    p95_ms: f64,
    zero_result_queries: usize,
    million_scale_verified: bool,
) -> String {
    format!(
        concat!(
            "{{",
            "\"schema_version\":\"benchmark.v1\",",
            "\"run_id\":\"bench_test\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"{}\",",
            "\"document_count\":{},",
            "\"query_count\":{},",
            "\"top_k\":10,",
            "\"build_ms\":1.0,",
            "\"query_total_ms\":1.0,",
            "\"qps\":100.0,",
            "\"index_size_bytes\":1000,",
            "\"query_latency_ms\":{{",
            "\"samples\":{},",
            "\"min\":1.0,",
            "\"mean\":2.0,",
            "\"p50\":2.0,",
            "\"p95\":{},",
            "\"p99\":{},",
            "\"max\":{}",
            "}},",
            "\"zero_result_queries\":{},",
            "\"total_hits\":100,",
            "\"million_scale_verified\":{},",
            "\"percentile_confidence\":\"sampled\",",
            "\"target_claim\":\"not_evaluated\",",
            "\"scope\":\"synthetic query benchmark; no raw resume text, paths, or queries included\"",
            "}}"
        ),
        dataset_kind,
        document_count,
        query_count,
        query_count,
        p95_ms,
        p95_ms,
        p95_ms,
        zero_result_queries,
        million_scale_verified,
    )
}

fn minimal_ocr_throughput_json(
    dataset_kind: &str,
    page_count: usize,
    p95_ms: f64,
    pages_per_second: f64,
    target_claim: &str,
) -> String {
    format!(
        concat!(
            "{{",
            "\"schema_version\":\"ocr-throughput.v1\",",
            "\"run_id\":\"bench_test\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"{}\",",
            "\"engine_kind\":\"local-command\",",
            "\"page_count\":{},",
            "\"total_ms\":100.0,",
            "\"pages_per_second\":{},",
            "\"total_page_bytes\":1000,",
            "\"total_text_bytes\":100,",
            "\"mean_confidence\":0.95,",
            "\"page_latency_ms\":{{",
            "\"samples\":{},",
            "\"min\":1.0,",
            "\"mean\":2.0,",
            "\"p50\":2.0,",
            "\"p95\":{},",
            "\"p99\":{},",
            "\"max\":{}",
            "}},",
            "\"target_claim\":\"{}\",",
            "\"scope\":\"synthetic OCR throughput benchmark; no raw OCR text, page bytes, command paths, or resume paths included\"",
            "}}"
        ),
        dataset_kind,
        page_count,
        pages_per_second,
        page_count,
        p95_ms,
        p95_ms,
        p95_ms,
        target_claim,
    )
}

fn ocr_fixture_script(label: &str) -> PathBuf {
    let path = temp_dir(label).join("ocr-fixture.sh");
    fs::write(
        &path,
        "#!/bin/sh\nprintf 'resume-ir-ocr-v1\\nconfidence=0.97\\ntext:\\nSynthetic OCR Candidate page %s PRIVATE OCR PAYLOAD\\n' \"$RESUME_IR_OCR_PAGE_NO\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
    }
    path
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
