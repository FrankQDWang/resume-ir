use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use benchmark_runner::{
    evaluate_benchmark_gate_json, evaluate_dedupe_quality_gate_json,
    evaluate_field_quality_gate_json, evaluate_ocr_throughput_gate_json,
    evaluate_vector_quality_gate_json, run_dedupe_quality_jsonl, run_field_quality_jsonl,
    run_synthetic_ocr_throughput_benchmark, run_synthetic_query_benchmark,
    run_vector_quality_jsonl, BenchmarkGateConfig, DedupeQualityGateConfig, FieldQualityGateConfig,
    OcrThroughputGateConfig, SyntheticBenchmarkConfig, SyntheticOcrBenchmarkConfig,
    SyntheticOcrBenchmarkEngine, VectorQualityConfig, VectorQualityGateConfig,
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
fn benchmark_gate_requires_private_real_corpus_metadata_for_release_evidence() {
    let report = minimal_private_real_benchmark_json(100_000, 200, 150.0, false);
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0).require_private_real_corpus();

    let evaluation = evaluate_benchmark_gate_json(&report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.document_count(), 100_000);
    assert_eq!(evaluation.query_count(), 200);
    assert_eq!(evaluation.p95_ms(), 150.0);
}

#[test]
fn benchmark_gate_rejects_private_real_report_without_hot_hybrid_evidence() {
    let mut report = minimal_benchmark_json("private-real-corpus", 100_000, 200, 150.0, 0, false)
        .replace(
            "\"target_claim\":\"not_evaluated\"",
            "\"target_claim\":\"query_latency_target_met\"",
        )
        .replace(
            "\"scope\":\"synthetic query benchmark; no raw resume text, paths, or queries included\"",
            "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\"",
        );
    report.pop();
    report.push_str(concat!(
        ",\"corpus_origin\":\"private_local\"",
        ",\"privacy_boundary\":\"redacted_local_aggregate\"",
        ",\"contains_raw_resume_text\":false",
        ",\"contains_resume_paths\":false",
        ",\"contains_queries\":false",
        ",\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"",
        ",\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\""
    ));
    report.push('}');
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires hot-index hybrid query evidence"));
}

#[test]
fn benchmark_gate_rejects_real_release_report_without_private_boundary() {
    let report = minimal_benchmark_json("private-real-corpus", 100_000, 200, 150.0, 0, false);
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires redacted local boundary"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_without_boundary_even_without_release_flag() {
    let report = minimal_benchmark_json("private-real-corpus", 100_000, 200, 150.0, 0, false);
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0);

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires redacted local boundary"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_with_extra_payload_field() {
    let mut report = minimal_private_real_benchmark_json(100_000, 200, 150.0, false);
    report.pop();
    report.push_str(",\"notes\":\"private local path /Users/frankqdwang/MLE/resume-ir\"");
    report.push('}');
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("unsupported private real-corpus benchmark field"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_with_payload_in_allowed_fields() {
    let report = minimal_private_real_benchmark_json(1_000_000, 500, 150.0, true)
        .replace(
            "\"qps\":100.0",
            "\"qps\":\"/Users/frankqdwang/private/resume.pdf\"",
        )
        .replace("\"min\":1.0", "\"min\":\"private search query\"");
    let config = BenchmarkGateConfig::new(1_000_000, 500, 200.0)
        .require_private_real_corpus()
        .require_million_scale();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires redacted local boundary"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_with_duplicate_payload_keys() {
    let report = minimal_private_real_benchmark_json(1_000_000, 500, 150.0, true)
        .replace(
            "\"run_id\":\"bench_test\"",
            "\"run_id\":\"/Users/frankqdwang/private/resume.pdf\",\"run_id\":\"bench_test\"",
        )
        .replace(
            "\"min\":1.0",
            "\"min\":\"private search query\",\"min\":1.0",
        );
    let config = BenchmarkGateConfig::new(1_000_000, 500, 200.0)
        .require_private_real_corpus()
        .require_million_scale();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error.to_string().contains("duplicate JSON object key"));
}

#[test]
fn benchmark_gate_rejects_million_release_gate_without_million_proof() {
    let report = minimal_private_real_benchmark_json(100_000, 200, 150.0, false);
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0)
        .require_private_real_corpus()
        .require_million_scale();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("million-scale benchmark required"));
}

#[test]
fn field_quality_report_scores_labeled_samples_without_raw_value_leakage() {
    let dataset = concat!(
        "{\"sample_id\":\"case-a\",\"text\":\"Name: Synthetic Candidate\\nEmail: candidate@example.test\\nPhone: +1 (415) 555-0132\\nEducation\\nBachelor of Science\\nMajor: Computer Science\\nSkills: Rust, Java\",",
        "\"expected\":[",
        "{\"type\":\"name\",\"normalized\":\"synthetic candidate\"},",
        "{\"type\":\"email\",\"normalized\":\"candidate@example.test\"},",
        "{\"type\":\"phone\",\"normalized\":\"+14155550132\"},",
        "{\"type\":\"skill\",\"normalized\":\"Rust\"},",
        "{\"type\":\"skill\",\"normalized\":\"Java\"},",
        "{\"type\":\"degree\",\"normalized\":\"bachelor\"},",
        "{\"type\":\"major\",\"normalized\":\"computer_science\"}",
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
    assert_eq!(report.expected_mentions(), 9);
    assert!(report.overall().f1() >= 0.95);
    assert!(report.field_metric("email").unwrap().f1() >= 0.99);
    assert!(report.field_metric("phone").unwrap().f1() >= 0.99);
    assert!(report.field_metric("skill").unwrap().f1() >= 0.99);
    assert!(report.field_metric("major").unwrap().f1() >= 0.99);
    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"field-quality.v1\""));
    assert!(json.contains("\"dataset_kind\":\"labeled\""));
    assert!(json.contains("\"sample_count\":2"));
    assert!(json.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!json.contains("Synthetic Candidate"));
    assert!(!json.contains("candidate@example.test"));
    assert!(!json.contains("+1 (415) 555-0132"));
    assert!(!json.contains("+14155550132"));
    assert!(!json.contains("Computer Science"));
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
fn field_quality_gate_rejects_release_evidence_without_private_business_boundary() {
    let dataset = concat!(
        "{\"text\":\"Email: candidate@example.test\\nPhone: (415) 555-0132\",",
        "\"expected\":[",
        "{\"type\":\"email\",\"normalized\":\"candidate@example.test\"},",
        "{\"type\":\"phone\",\"normalized\":\"+14155550132\"}",
        "]}\n",
    );
    let report = run_field_quality_jsonl(dataset).unwrap();
    let config = FieldQualityGateConfig::new(0.99, 0.99, 0.99)
        .with_min_samples(1)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report.to_redacted_json(), config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field-quality benchmark required"));
}

#[test]
fn field_quality_gate_accepts_private_business_labeled_release_evidence() {
    let report = minimal_private_business_field_quality_json();
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let evaluation = evaluate_field_quality_gate_json(&report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "private-business-labeled");
    assert_eq!(evaluation.sample_count(), 1_000);
    assert!(evaluation.f1() >= 0.99);
}

#[test]
fn field_quality_gate_rejects_private_business_report_without_production_fields() {
    let report = minimal_private_business_field_quality_json().replace(
        ",\"date_range\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
        "",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality requires production field metrics"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_without_name_metric() {
    let report = minimal_private_business_field_quality_json().replace(
        "\"name\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality requires production field metrics"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_without_field_label_support() {
    let report = minimal_private_business_field_quality_json().replace(
        "\"name\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"name\":{\"true_positive\":0,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality requires production field support"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_with_inconsistent_metric_counts() {
    let report = minimal_private_business_field_quality_json().replace(
        "\"name\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"name\":{\"true_positive\":1,\"false_positive\":1,\"false_negative\":1,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality metric counts do not match scores"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_without_school_tier_metric() {
    let report = minimal_private_business_field_quality_json().replace(
        ",\"school_tier\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
        "",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality requires production field metrics"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_without_major_metric() {
    let report = minimal_private_business_field_quality_json().replace(
        ",\"major\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
        "",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality requires production field metrics"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_without_location_metric() {
    let report = minimal_private_business_field_quality_json().replace(
        ",\"location\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
        "",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality requires production field metrics"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_without_certificate_metric() {
    let report = minimal_private_business_field_quality_json().replace(
        ",\"certificate\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
        "",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality requires production field metrics"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_without_years_experience_metric() {
    let report = minimal_private_business_field_quality_json().replace(
        ",\"years_experience\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
        "",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality requires production field metrics"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_without_boundary_metadata() {
    let report = minimal_private_business_field_quality_json().replace(
        "\"privacy_boundary\":\"redacted_local_aggregate\"",
        "\"privacy_boundary\":\"raw_local_files\"",
    );
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality requires redacted local boundary"));
}

#[test]
fn field_quality_gate_rejects_private_business_report_with_extra_payload_field() {
    let mut report = minimal_private_business_field_quality_json();
    report.pop();
    report.push_str(",\"notes\":\"private local path /Users/frankqdwang/resume.pdf\"");
    report.push('}');
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("unsupported private business field quality field"));
}

#[test]
fn dedupe_quality_report_scores_labeled_pairs_without_profile_leakage() {
    let dataset = concat!(
        "{\"sample_id\":\"private-dedupe-a\",",
        "\"left\":{\"id\":\"private-left-a\",\"name\":\"Synthetic Candidate\",\"schools\":[\"Synthetic University\"],\"companies\":[\"Example Labs\"],\"skills\":[\"Java\",\"Payments\"]},",
        "\"right\":{\"id\":\"private-right-a\",\"name\":\"synthetic candidate\",\"schools\":[\"synthetic university\"],\"companies\":[\"Example Labs\"],\"skills\":[\"Java\",\"Search\"]},",
        "\"duplicate\":true}\n",
        "{\"sample_id\":\"private-dedupe-b\",",
        "\"left\":{\"id\":\"private-left-b\",\"name\":\"Synthetic Candidate\",\"schools\":[\"Synthetic University\"],\"companies\":[\"Example Labs\"],\"skills\":[\"Java\"]},",
        "\"right\":{\"id\":\"private-right-b\",\"name\":\"Different Candidate\",\"schools\":[\"Synthetic University\"],\"companies\":[\"Example Labs\"],\"skills\":[\"Java\"]},",
        "\"duplicate\":false}\n",
    );

    let report = run_dedupe_quality_jsonl(dataset).unwrap();

    assert_eq!(report.dataset_kind(), "labeled");
    assert_eq!(report.pair_count(), 2);
    assert_eq!(report.positive_pair_count(), 1);
    assert!(report.precision() >= 0.99);
    assert!(report.recall() >= 0.99);
    assert!(report.f1() >= 0.99);
    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"dedupe-quality.v1\""));
    assert!(json.contains("\"dataset_kind\":\"labeled\""));
    assert!(json.contains("\"pair_count\":2"));
    assert!(json.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!json.contains("private-dedupe-a"));
    assert!(!json.contains("private-left-a"));
    assert!(!json.contains("Synthetic Candidate"));
    assert!(!json.contains("Synthetic University"));
    assert!(!json.contains("Example Labs"));
    assert!(!json.contains("Payments"));
}

#[test]
fn dedupe_quality_gate_rejects_low_recall_reports() {
    let report = concat!(
        "{\"schema_version\":\"dedupe-quality.v1\",",
        "\"dataset_kind\":\"labeled\",",
        "\"pair_count\":10,",
        "\"positive_pair_count\":5,",
        "\"predicted_duplicate_pairs\":1,",
        "\"true_positive\":1,",
        "\"false_positive\":0,",
        "\"false_negative\":4,",
        "\"true_negative\":5,",
        "\"precision\":1.0,",
        "\"recall\":0.2,",
        "\"f1\":0.333,",
        "\"target_claim\":\"not_evaluated\"}"
    );
    let config = DedupeQualityGateConfig::new(0.90, 0.90, 0.90)
        .with_min_pairs(10)
        .with_min_positive_pairs(5);

    let error = evaluate_dedupe_quality_gate_json(report, config).unwrap_err();

    assert!(error.to_string().contains("dedupe recall below threshold"));
}

#[test]
fn dedupe_quality_gate_accepts_labeled_report_without_target_claim() {
    let dataset = concat!(
        "{\"left\":{\"id\":\"left-a\",\"name\":\"Synthetic Candidate\",\"schools\":[\"Synthetic University\"],\"skills\":[\"Java\"]},",
        "\"right\":{\"id\":\"right-a\",\"name\":\"synthetic candidate\",\"schools\":[\"synthetic university\"],\"skills\":[\"Java\"]},",
        "\"duplicate\":true}\n",
    );
    let report = run_dedupe_quality_jsonl(dataset).unwrap();
    let config = DedupeQualityGateConfig::new(0.99, 0.99, 0.99)
        .with_min_pairs(1)
        .with_min_positive_pairs(1);

    let evaluation = evaluate_dedupe_quality_gate_json(&report.to_redacted_json(), config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "labeled");
    assert_eq!(evaluation.pair_count(), 1);
    assert!(evaluation.f1() >= 0.99);
}

#[test]
fn dedupe_quality_gate_rejects_release_evidence_without_private_business_boundary() {
    let dataset = concat!(
        "{\"left\":{\"id\":\"left-a\",\"name\":\"Synthetic Candidate\",\"schools\":[\"Synthetic University\"],\"skills\":[\"Java\"]},",
        "\"right\":{\"id\":\"right-a\",\"name\":\"synthetic candidate\",\"schools\":[\"synthetic university\"],\"skills\":[\"Java\"]},",
        "\"duplicate\":true}\n",
    );
    let report = run_dedupe_quality_jsonl(dataset).unwrap();
    let config = DedupeQualityGateConfig::new(0.90, 0.90, 0.90)
        .with_min_pairs(1)
        .with_min_positive_pairs(1)
        .require_private_business_labeled();

    let error = evaluate_dedupe_quality_gate_json(&report.to_redacted_json(), config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business dedupe-quality benchmark required"));
}

#[test]
fn dedupe_quality_gate_accepts_private_business_labeled_release_evidence() {
    let report = minimal_private_business_dedupe_quality_json();
    let config = DedupeQualityGateConfig::new(0.90, 0.90, 0.90)
        .with_min_pairs(1_000)
        .with_min_positive_pairs(100)
        .require_private_business_labeled();

    let evaluation = evaluate_dedupe_quality_gate_json(&report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "private-business-labeled");
    assert_eq!(evaluation.pair_count(), 1_000);
    assert!(evaluation.f1() >= 0.99);
}

#[test]
fn dedupe_quality_gate_rejects_private_business_report_with_inconsistent_metric_counts() {
    let report = minimal_private_business_dedupe_quality_json().replace(
        "\"true_positive\":100,\"false_positive\":0,\"false_negative\":0,\"true_negative\":900,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0",
        "\"true_positive\":50,\"false_positive\":50,\"false_negative\":50,\"true_negative\":850,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0",
    );
    let config = DedupeQualityGateConfig::new(0.90, 0.90, 0.90)
        .with_min_pairs(1_000)
        .with_min_positive_pairs(100)
        .require_private_business_labeled();

    let error = evaluate_dedupe_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business dedupe quality metric counts do not match scores"));
}

#[test]
fn dedupe_quality_gate_rejects_private_business_report_with_inconsistent_pair_counts() {
    let report = minimal_private_business_dedupe_quality_json()
        .replace("\"positive_pair_count\":100", "\"positive_pair_count\":200");
    let config = DedupeQualityGateConfig::new(0.90, 0.90, 0.90)
        .with_min_pairs(1_000)
        .with_min_positive_pairs(100)
        .require_private_business_labeled();

    let error = evaluate_dedupe_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business dedupe quality counts are inconsistent"));
}

#[test]
fn dedupe_quality_gate_rejects_private_business_report_with_extra_payload_field() {
    let mut report = minimal_private_business_dedupe_quality_json();
    report.pop();
    report.push_str(",\"notes\":\"private local path /Users/frankqdwang/resume.pdf\"");
    report.push('}');
    let config = DedupeQualityGateConfig::new(0.90, 0.90, 0.90)
        .with_min_pairs(1_000)
        .with_min_positive_pairs(100)
        .require_private_business_labeled();

    let error = evaluate_dedupe_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("unsupported private business dedupe quality field"));
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

#[test]
fn ocr_throughput_gate_requires_private_real_release_boundary() {
    let synthetic_report =
        minimal_ocr_throughput_json("synthetic", 500, 450.0, 2.5, "not_evaluated");
    let config = OcrThroughputGateConfig::new(500, 1_000.0, 1.0).require_private_real_corpus();

    let synthetic_error = evaluate_ocr_throughput_gate_json(&synthetic_report, config).unwrap_err();

    assert!(synthetic_error
        .to_string()
        .contains("private real-corpus OCR benchmark required"));

    let private_report = concat!(
        "{\"schema_version\":\"ocr-throughput.v1\",",
        "\"run_id\":\"ocr_release_20260605\",",
        "\"platform\":\"macos/aarch64\",",
        "\"dataset_kind\":\"private-real-corpus\",",
        "\"page_count\":500,",
        "\"document_count\":200,",
        "\"scanned_document_count\":150,",
        "\"engine_kind\":\"tesseract\",",
        "\"page_latency_ms\":{\"samples\":500,\"p50\":250.0,\"p95\":450.0,\"p99\":800.0},",
        "\"pages_per_second\":2.5,",
        "\"target_claim\":\"ocr_throughput_target_met\",",
        "\"corpus_origin\":\"private_local\",",
        "\"privacy_boundary\":\"redacted_local_aggregate\",",
        "\"contains_raw_ocr_text\":false,",
        "\"contains_page_images\":false,",
        "\"contains_resume_paths\":false,",
        "\"contains_document_ids\":false,",
        "\"contains_page_ids\":false,",
        "\"contains_command_paths\":false,",
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
        "\"ocr_runtime_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
        "\"renderer_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
        "\"language_pack_manifest_sha256\":\"2222222222222222222222222222222222222222222222222222222222222222\",",
        "\"scope\":\"private real-corpus OCR throughput benchmark; aggregate redacted report only\"}"
    );

    let evaluation = evaluate_ocr_throughput_gate_json(private_report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.page_count(), 500);
    assert_eq!(evaluation.p95_ms(), 450.0);
}

#[test]
fn vector_quality_report_scores_labeled_samples_without_text_id_path_or_vector_leakage() {
    let command = embedding_fixture_script("vector-quality-private-command");
    let dataset = concat!(
        "{\"sample_id\":\"private-vector-case-a\",\"query\":\"Backend Java payment search\",",
        "\"candidates\":[",
        "{\"id\":\"private-java-doc\",\"text\":\"Java payment backend search engineer\",\"relevant\":true},",
        "{\"id\":\"private-sales-doc\",\"text\":\"Sales operations recruiter\",\"relevant\":false}",
        "]}\n",
        "{\"sample_id\":\"private-vector-case-b\",\"query\":\"Rust indexing platform\",",
        "\"candidates\":[",
        "{\"id\":\"private-rust-doc\",\"text\":\"Rust indexing platform engineer\",\"relevant\":true},",
        "{\"id\":\"private-hr-doc\",\"text\":\"HR business partner\",\"relevant\":false}",
        "]}\n",
    );
    let config = VectorQualityConfig::new(&command, "fixture-local-model", 3)
        .unwrap()
        .with_top_k(1);

    let report = run_vector_quality_jsonl(dataset, config).unwrap();

    assert_eq!(report.dataset_kind(), "labeled");
    assert_eq!(report.sample_count(), 2);
    assert_eq!(report.candidate_count(), 4);
    assert_eq!(report.top_k(), 1);
    assert!(report.recall_at_k() >= 0.99);
    assert!(report.mrr() >= 0.99);
    assert!(report.ndcg_at_k() >= 0.99);
    assert_eq!(report.zero_recall_queries(), 0);

    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"vector-quality.v1\""));
    assert!(json.contains("\"dataset_kind\":\"labeled\""));
    assert!(json.contains("\"sample_count\":2"));
    assert!(json.contains("\"candidate_count\":4"));
    assert!(json.contains("\"top_k\":1"));
    assert!(json.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("private-vector-case-a"));
    assert!(!json.contains("private-java-doc"));
    assert!(!json.contains("Backend Java payment search"));
    assert!(!json.contains("Java payment backend"));
    assert!(!json.contains("1.0,0.0,0.0"));

    remove_dir(command.parent().unwrap());
}

#[test]
fn vector_quality_gate_rejects_low_recall_reports() {
    let report = concat!(
        "{\"schema_version\":\"vector-quality.v1\",",
        "\"dataset_kind\":\"labeled\",",
        "\"sample_count\":2,",
        "\"candidate_count\":4,",
        "\"top_k\":1,",
        "\"recall_at_k\":0.5,",
        "\"mrr\":0.5,",
        "\"ndcg_at_k\":0.5,",
        "\"zero_recall_queries\":1,",
        "\"target_claim\":\"not_evaluated\"}"
    );
    let config = VectorQualityGateConfig::new(2, 0.95, 0.95, 0.95).with_max_zero_recall_queries(0);

    let error = evaluate_vector_quality_gate_json(report, config).unwrap_err();

    assert!(error.to_string().contains("vector recall below threshold"));
}

#[test]
fn vector_quality_gate_accepts_labeled_report_without_target_claim() {
    let command = embedding_fixture_script("vector-quality-gate-private-command");
    let dataset = concat!(
        "{\"query\":\"Backend Java payment search\",",
        "\"candidates\":[",
        "{\"id\":\"private-java-doc\",\"text\":\"Java payment backend search engineer\",\"relevant\":true},",
        "{\"id\":\"private-sales-doc\",\"text\":\"Sales operations recruiter\",\"relevant\":false}",
        "]}\n",
    );
    let config = VectorQualityConfig::new(&command, "fixture-local-model", 3)
        .unwrap()
        .with_top_k(1);
    let report = run_vector_quality_jsonl(dataset, config).unwrap();
    let gate_config =
        VectorQualityGateConfig::new(1, 0.99, 0.99, 0.99).with_max_zero_recall_queries(0);

    let evaluation =
        evaluate_vector_quality_gate_json(&report.to_redacted_json(), gate_config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "labeled");
    assert_eq!(evaluation.sample_count(), 1);
    assert!(evaluation.recall_at_k() >= 0.99);

    remove_dir(command.parent().unwrap());
}

#[test]
fn vector_quality_gate_requires_private_business_labeled_release_boundary() {
    let ordinary_report = concat!(
        "{\"schema_version\":\"vector-quality.v1\",",
        "\"dataset_kind\":\"labeled\",",
        "\"sample_count\":50,",
        "\"candidate_count\":200,",
        "\"top_k\":10,",
        "\"recall_at_k\":0.95,",
        "\"mrr\":0.91,",
        "\"ndcg_at_k\":0.93,",
        "\"zero_recall_queries\":0,",
        "\"target_claim\":\"not_evaluated\"}"
    );
    let config = VectorQualityGateConfig::new(50, 0.90, 0.90, 0.90)
        .with_max_zero_recall_queries(0)
        .require_private_business_labeled();

    let ordinary_error = evaluate_vector_quality_gate_json(ordinary_report, config).unwrap_err();

    assert!(ordinary_error
        .to_string()
        .contains("private business vector-quality benchmark required"));

    let private_report = concat!(
        "{\"schema_version\":\"vector-quality.v1\",",
        "\"run_id\":\"vector_release_20260605\",",
        "\"platform\":\"macos/aarch64\",",
        "\"dataset_kind\":\"private-business-labeled\",",
        "\"sample_count\":50,",
        "\"candidate_count\":200,",
        "\"top_k\":10,",
        "\"recall_at_k\":0.95,",
        "\"mrr\":0.91,",
        "\"ndcg_at_k\":0.93,",
        "\"zero_recall_queries\":0,",
        "\"target_claim\":\"vector_quality_target_met\",",
        "\"corpus_origin\":\"private_local\",",
        "\"privacy_boundary\":\"redacted_local_aggregate\",",
        "\"contains_raw_queries\":false,",
        "\"contains_candidate_text\":false,",
        "\"contains_resume_paths\":false,",
        "\"contains_sample_ids\":false,",
        "\"contains_candidate_ids\":false,",
        "\"contains_vectors\":false,",
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
        "\"annotation_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
        "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
        "\"vector_taxonomy\":\"resume-ir.vector-quality.v1\",",
        "\"scope\":\"private business vector-quality benchmark; aggregate redacted report only\"}"
    );

    let evaluation = evaluate_vector_quality_gate_json(private_report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "private-business-labeled");
    assert_eq!(evaluation.sample_count(), 50);
    assert!(evaluation.recall_at_k() >= 0.95);
}

#[test]
fn vector_quality_gate_rejects_private_business_report_with_impossible_top_k() {
    let report = minimal_private_business_vector_quality_json()
        .replace("\"candidate_count\":200", "\"candidate_count\":5");
    let config = VectorQualityGateConfig::new(50, 0.90, 0.90, 0.90)
        .with_max_zero_recall_queries(0)
        .require_private_business_labeled();

    let error = evaluate_vector_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business vector quality counts are inconsistent"));
}

#[test]
fn vector_quality_gate_rejects_private_business_report_with_impossible_zero_recall_count() {
    let report = minimal_private_business_vector_quality_json()
        .replace("\"zero_recall_queries\":0", "\"zero_recall_queries\":51");
    let config = VectorQualityGateConfig::new(50, 0.90, 0.90, 0.90)
        .with_max_zero_recall_queries(51)
        .require_private_business_labeled();

    let error = evaluate_vector_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business vector quality counts are inconsistent"));
}

#[test]
fn vector_quality_gate_rejects_private_business_report_with_inconsistent_zero_recall_metric() {
    let report = minimal_private_business_vector_quality_json()
        .replace("\"recall_at_k\":0.95", "\"recall_at_k\":1.0")
        .replace("\"zero_recall_queries\":0", "\"zero_recall_queries\":1");
    let config = VectorQualityGateConfig::new(50, 0.90, 0.90, 0.90)
        .with_max_zero_recall_queries(1)
        .require_private_business_labeled();

    let error = evaluate_vector_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business vector quality metric counts do not match scores"));
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

fn minimal_private_real_benchmark_json(
    document_count: usize,
    query_count: usize,
    p95_ms: f64,
    million_scale_verified: bool,
) -> String {
    let mut report = minimal_benchmark_json(
        "private-real-corpus",
        document_count,
        query_count,
        p95_ms,
        0,
        million_scale_verified,
    )
    .replace(
        "\"target_claim\":\"not_evaluated\"",
        "\"target_claim\":\"query_latency_target_met\"",
    )
    .replace(
        "\"scope\":\"synthetic query benchmark; no raw resume text, paths, or queries included\"",
        "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\"",
    );
    report.pop();
    report.push_str(concat!(
        ",\"corpus_origin\":\"private_local\"",
        ",\"privacy_boundary\":\"redacted_local_aggregate\"",
        ",\"query_mode\":\"hybrid\"",
        ",\"retrieval_layers\":\"fulltext+field+vector+rrf\"",
        ",\"hot_index\":true",
        ",\"hot_path_ocr\":false",
        ",\"hot_path_parsing\":false",
        ",\"hot_path_heavy_model_inference\":false",
        ",\"contains_raw_resume_text\":false",
        ",\"contains_resume_paths\":false",
        ",\"contains_queries\":false",
        ",\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"",
        ",\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\""
    ));
    report.push('}');
    report
}

fn minimal_private_business_field_quality_json() -> String {
    concat!(
        "{",
        "\"schema_version\":\"field-quality.v1\",",
        "\"run_id\":\"fieldq_test\",",
        "\"platform\":\"test/test\",",
        "\"dataset_kind\":\"private-business-labeled\",",
        "\"sample_count\":1000,",
        "\"expected_mentions\":1000,",
        "\"predicted_mentions\":1000,",
        "\"overall\":{\"true_positive\":1000,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"fields\":{",
        "\"name\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"email\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"phone\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"school\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"school_tier\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"degree\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"major\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"company\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"title\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"location\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"skill\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"certificate\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"date_range\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"years_experience\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
        "},",
        "\"target_claim\":\"field_quality_target_met\",",
        "\"corpus_origin\":\"private_local\",",
        "\"privacy_boundary\":\"redacted_local_aggregate\",",
        "\"contains_raw_resume_text\":false,",
        "\"contains_resume_paths\":false,",
        "\"contains_field_values\":false,",
        "\"contains_sample_ids\":false,",
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
        "\"annotation_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
        "\"field_taxonomy\":\"resume-ir.fields.v1\",",
        "\"scope\":\"private business field-quality benchmark; aggregate redacted report only\"",
        "}"
    )
    .to_string()
}

fn minimal_private_business_dedupe_quality_json() -> String {
    concat!(
        "{",
        "\"schema_version\":\"dedupe-quality.v1\",",
        "\"run_id\":\"dedupeq_test\",",
        "\"platform\":\"test/test\",",
        "\"dataset_kind\":\"private-business-labeled\",",
        "\"pair_count\":1000,",
        "\"positive_pair_count\":100,",
        "\"predicted_duplicate_pairs\":100,",
        "\"true_positive\":100,",
        "\"false_positive\":0,",
        "\"false_negative\":0,",
        "\"true_negative\":900,",
        "\"precision\":1.0,",
        "\"recall\":1.0,",
        "\"f1\":1.0,",
        "\"target_claim\":\"dedupe_quality_target_met\",",
        "\"corpus_origin\":\"private_local\",",
        "\"privacy_boundary\":\"redacted_local_aggregate\",",
        "\"contains_raw_resume_text\":false,",
        "\"contains_resume_paths\":false,",
        "\"contains_profile_values\":false,",
        "\"contains_sample_ids\":false,",
        "\"contains_document_ids\":false,",
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
        "\"annotation_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
        "\"dedupe_taxonomy\":\"resume-ir.dedupe.v1\",",
        "\"scope\":\"private business dedupe-quality benchmark; aggregate redacted report only\"",
        "}"
    )
    .to_string()
}

fn minimal_private_business_vector_quality_json() -> String {
    concat!(
        "{",
        "\"schema_version\":\"vector-quality.v1\",",
        "\"run_id\":\"vectorq_test\",",
        "\"platform\":\"test/test\",",
        "\"dataset_kind\":\"private-business-labeled\",",
        "\"sample_count\":50,",
        "\"candidate_count\":200,",
        "\"top_k\":10,",
        "\"recall_at_k\":0.95,",
        "\"mrr\":0.91,",
        "\"ndcg_at_k\":0.93,",
        "\"zero_recall_queries\":0,",
        "\"target_claim\":\"vector_quality_target_met\",",
        "\"corpus_origin\":\"private_local\",",
        "\"privacy_boundary\":\"redacted_local_aggregate\",",
        "\"contains_raw_queries\":false,",
        "\"contains_candidate_text\":false,",
        "\"contains_resume_paths\":false,",
        "\"contains_sample_ids\":false,",
        "\"contains_candidate_ids\":false,",
        "\"contains_vectors\":false,",
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
        "\"annotation_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
        "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
        "\"vector_taxonomy\":\"resume-ir.vector-quality.v1\",",
        "\"scope\":\"private business vector-quality benchmark; aggregate redacted report only\"",
        "}"
    )
    .to_string()
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
    let path = temp_dir(label).join(ocr_fixture_file_name());
    fs::write(&path, ocr_fixture_script_body()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
    }
    path
}

fn embedding_fixture_script(label: &str) -> PathBuf {
    let path = temp_dir(label).join(embedding_fixture_file_name());
    fs::write(&path, embedding_fixture_script_body()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
    }
    path
}

#[cfg(unix)]
fn ocr_fixture_file_name() -> &'static str {
    "ocr-fixture.sh"
}

#[cfg(windows)]
fn ocr_fixture_file_name() -> &'static str {
    "ocr-fixture.cmd"
}

#[cfg(unix)]
fn ocr_fixture_script_body() -> &'static str {
    "#!/bin/sh\nprintf 'resume-ir-ocr-v1\\nconfidence=0.97\\ntext:\\nSynthetic OCR Candidate page %s PRIVATE OCR PAYLOAD\\n' \"$RESUME_IR_OCR_PAGE_NO\"\n"
}

#[cfg(windows)]
fn ocr_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "echo resume-ir-ocr-v1\r\n",
        "echo confidence=0.97\r\n",
        "echo text:\r\n",
        "echo Synthetic OCR Candidate page %RESUME_IR_OCR_PAGE_NO% PRIVATE OCR PAYLOAD\r\n",
        "exit /b 0\r\n"
    )
}

#[cfg(unix)]
fn embedding_fixture_file_name() -> &'static str {
    "embedding-fixture.sh"
}

#[cfg(windows)]
fn embedding_fixture_file_name() -> &'static str {
    "embedding-fixture.cmd"
}

#[cfg(unix)]
fn embedding_fixture_script_body() -> &'static str {
    r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=%s\n' "$RESUME_IR_EMBEDDING_MODEL_ID"
printf 'dimension=%s\n' "$RESUME_IR_EMBEDDING_DIMENSION"
awk '
  /^input=/ {
    split(substr($0, 7), parts, "\t");
    id = parts[1];
    if (id ~ /^query-000000/ || id ~ /^candidate-000000-000000/) {
      vector = "1.0,0.0,0.0";
    } else if (id ~ /^query-000001/ || id ~ /^candidate-000001-000000/) {
      vector = "0.0,1.0,0.0";
    } else {
      vector = "0.0,0.0,1.0";
    }
    printf "vector=%s\t%s\n", id, vector;
  }
' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#
}

#[cfg(windows)]
fn embedding_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "echo resume-ir-embedding-v1\r\n",
        "echo model_id=%RESUME_IR_EMBEDDING_MODEL_ID%\r\n",
        "echo dimension=%RESUME_IR_EMBEDDING_DIMENSION%\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_EMBEDDING_INPUT_PATH%\") do (\r\n",
        "  set \"line=%%L\"\r\n",
        "  if \"!line:~0,6!\"==\"input=\" (\r\n",
        "    set \"payload=!line:~6!\"\r\n",
        "    for /f \"tokens=1\" %%I in (\"!payload!\") do set \"id=%%I\"\r\n",
        "    set \"vector=0.0,0.0,1.0\"\r\n",
        "    if \"!id!\"==\"query-000000\" set \"vector=1.0,0.0,0.0\"\r\n",
        "    if \"!id!\"==\"candidate-000000-000000\" set \"vector=1.0,0.0,0.0\"\r\n",
        "    if \"!id!\"==\"query-000001\" set \"vector=0.0,1.0,0.0\"\r\n",
        "    if \"!id!\"==\"candidate-000001-000000\" set \"vector=0.0,1.0,0.0\"\r\n",
        "    echo vector=!id!\t!vector!\r\n",
        "  )\r\n",
        ")\r\n",
        "exit /b 0\r\n"
    )
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
