use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use benchmark_runner::{
    evaluate_benchmark_gate_json, evaluate_dedupe_quality_gate_json,
    evaluate_field_quality_gate_json, evaluate_ocr_throughput_gate_json,
    evaluate_vector_quality_gate_json, run_dedupe_quality_jsonl, run_field_quality_jsonl,
    run_private_business_dedupe_quality_jsonl, run_private_business_field_quality_jsonl,
    run_private_business_vector_quality_jsonl, run_private_ocr_throughput_benchmark,
    run_private_query_benchmark, run_synthetic_ocr_throughput_benchmark,
    run_synthetic_query_benchmark, run_vector_quality_jsonl, BenchmarkGateConfig,
    DedupeQualityGateConfig, FieldQualityGateConfig, OcrThroughputGateConfig,
    PrivateDedupeQualityManifestDigests, PrivateFieldQualityManifestDigests,
    PrivateOcrBenchmarkEngine, PrivateOcrManifestDigests, PrivateOcrThroughputConfig,
    PrivatePdfRenderEngine, PrivateQueryBenchmarkCommand, PrivateQueryBenchmarkConfig,
    PrivateQueryManifestDigests, PrivateVectorQualityManifestDigests, SyntheticBenchmarkConfig,
    SyntheticOcrBenchmarkConfig, SyntheticOcrBenchmarkEngine, VectorQualityConfig,
    VectorQualityGateConfig,
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
fn private_query_benchmark_outputs_redacted_gateable_report() {
    let query_set = private_query_set_file("private-query-benchmark-set", 500);
    let command = query_fixture_script("private-query-benchmark-command");
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::local_command(&command).unwrap(),
        8_720,
        manifests,
    )
    .unwrap()
    .with_max_queries(500)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap()
    .with_index_size_bytes(4096);

    let report = run_private_query_benchmark(config).unwrap();

    assert_eq!(report.document_count(), 8_720);
    assert_eq!(report.query_count(), 500);
    assert_eq!(report.top_k(), 10);
    assert_eq!(report.zero_result_queries(), 0);
    assert_eq!(report.latency().samples(), 500);
    assert!(report.qps() > 0.0);
    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"benchmark.v1\""));
    assert!(json.contains("\"dataset_kind\":\"private-real-corpus\""));
    assert!(json.contains("\"document_count\":8720"));
    assert!(json.contains("\"query_count\":500"));
    assert!(json.contains("\"target_claim\":\"query_latency_target_met\""));
    assert!(json.contains("\"query_mode\":\"hybrid\""));
    assert!(json.contains("\"retrieval_layers\":\"fulltext+field+vector+rrf\""));
    assert!(json.contains("\"hot_index\":true"));
    assert!(json.contains("\"hot_path_ocr\":false"));
    assert!(json.contains("\"hot_path_parsing\":false"));
    assert!(json.contains("\"hot_path_heavy_model_inference\":false"));
    assert!(json.contains("\"contains_raw_resume_text\":false"));
    assert!(json.contains("\"contains_resume_paths\":false"));
    assert!(json.contains("\"contains_queries\":false"));
    assert!(json.contains(
        "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\""
    ));
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));
    assert!(!json.contains("private-query-sample-"));

    let gate = BenchmarkGateConfig::new(8_000, 500, 10_000.0).require_private_real_corpus();
    let evaluation = evaluate_benchmark_gate_json(&json, gate).unwrap();
    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.document_count(), 8_720);
    assert_eq!(evaluation.query_count(), 500);

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
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
    let report = minimal_private_real_benchmark_json(100_000, 500, 150.0, false);
    let config = BenchmarkGateConfig::new(100_000, 500, 200.0).require_private_real_corpus();

    let evaluation = evaluate_benchmark_gate_json(&report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.document_count(), 100_000);
    assert_eq!(evaluation.query_count(), 500);
    assert_eq!(evaluation.p95_ms(), 150.0);
}

#[test]
fn benchmark_gate_rejects_private_real_release_with_too_few_query_samples() {
    let report = minimal_private_real_benchmark_json(100_000, 200, 150.0, false);
    let config = BenchmarkGateConfig::new(100_000, 100, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires release query sample count"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_with_inconsistent_query_counts() {
    let report = minimal_private_real_benchmark_json(100_000, 200, 150.0, false)
        .replace("\"samples\":200", "\"samples\":199");
    let config = BenchmarkGateConfig::new(100_000, 100, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark counts are inconsistent"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_with_inconsistent_qps() {
    let report = minimal_private_real_benchmark_json(100_000, 200, 150.0, false)
        .replace("\"qps\":100.0", "\"qps\":999.0");
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark metric counts do not match scores"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_with_impossible_total_hits() {
    let report = minimal_private_real_benchmark_json(100_000, 200, 150.0, false)
        .replace("\"total_hits\":100", "\"total_hits\":2001");
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark counts are inconsistent"));
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
fn benchmark_gate_rejects_million_release_gate_with_sampled_confidence() {
    let report = minimal_private_real_benchmark_json(1_000_000, 500, 150.0, true);
    let config = BenchmarkGateConfig::new(1_000_000, 500, 200.0)
        .require_private_real_corpus()
        .require_million_scale();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("million-scale release benchmark requires release confidence"));
}

#[test]
fn benchmark_gate_accepts_million_release_gate_with_release_confidence() {
    let report = minimal_private_real_benchmark_json(1_000_000, 500, 150.0, true).replace(
        "\"percentile_confidence\":\"sampled\"",
        "\"percentile_confidence\":\"release\"",
    );
    let config = BenchmarkGateConfig::new(1_000_000, 500, 200.0)
        .require_private_real_corpus()
        .require_million_scale();

    let evaluation = evaluate_benchmark_gate_json(&report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.document_count(), 1_000_000);
    assert_eq!(evaluation.query_count(), 500);
    assert_eq!(evaluation.p95_ms(), 150.0);
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
fn private_business_field_quality_report_outputs_redacted_gateable_report() {
    let manifests = PrivateFieldQualityManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
    )
    .unwrap();

    let report = run_private_business_field_quality_jsonl(
        &private_business_field_quality_dataset(),
        manifests,
    )
    .unwrap();

    assert_eq!(report.dataset_kind(), "private-business-labeled");
    assert_eq!(report.sample_count(), 1);
    assert!(report.overall().f1() >= 0.95);
    assert!(report.field_metric("name").unwrap().f1() >= 0.95);
    assert!(report.field_metric("email").unwrap().f1() >= 0.995);
    assert!(report.field_metric("phone").unwrap().f1() >= 0.995);
    assert!(report.field_metric("school").unwrap().f1() >= 0.93);
    assert!(report.field_metric("school_tier").unwrap().f1() >= 0.90);
    assert!(report.field_metric("degree").unwrap().f1() >= 0.95);
    assert!(report.field_metric("major").unwrap().f1() >= 0.90);
    assert!(report.field_metric("company").unwrap().f1() >= 0.90);
    assert!(report.field_metric("title").unwrap().f1() >= 0.88);
    assert!(report.field_metric("location").unwrap().f1() >= 0.90);
    assert!(report.field_metric("skill").unwrap().f1() >= 0.92);
    assert!(report.field_metric("certificate").unwrap().f1() >= 0.90);
    assert!(report.field_metric("date_range").unwrap().f1() >= 0.93);
    assert!(report.field_metric("years_experience").unwrap().f1() >= 0.90);

    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"field-quality.v1\""));
    assert!(json.contains("\"dataset_kind\":\"private-business-labeled\""));
    assert!(json.contains("\"target_claim\":\"field_quality_target_met\""));
    assert!(json.contains("\"corpus_origin\":\"private_local\""));
    assert!(json.contains("\"privacy_boundary\":\"redacted_local_aggregate\""));
    assert!(json.contains("\"contains_raw_resume_text\":false"));
    assert!(json.contains("\"contains_resume_paths\":false"));
    assert!(json.contains("\"contains_field_values\":false"));
    assert!(json.contains("\"contains_sample_ids\":false"));
    assert!(json.contains("\"field_taxonomy\":\"resume-ir.fields.v1\""));
    assert!(json.contains(
        "\"scope\":\"private business field-quality benchmark; aggregate redacted report only\""
    ));
    assert!(!json.contains("private-field-sample-001"));
    assert!(!json.contains("REDACTION_SENTINEL_FIELD_VALUE"));
    assert!(!json.contains("Synthetic Field Candidate"));
    assert!(!json.contains("field-candidate@example.test"));
    assert!(!json.contains("Synthetic Commerce"));

    let gate = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1)
        .require_private_business_labeled();
    let evaluation = evaluate_field_quality_gate_json(&json, gate).unwrap();
    assert_eq!(evaluation.dataset_kind(), "private-business-labeled");
    assert_eq!(evaluation.sample_count(), 1);
    assert!(evaluation.f1() >= 0.93);
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
fn private_business_dedupe_quality_report_outputs_redacted_gateable_report() {
    let manifests = PrivateDedupeQualityManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
    )
    .unwrap();

    let report = run_private_business_dedupe_quality_jsonl(
        &private_business_dedupe_quality_dataset(),
        manifests,
    )
    .unwrap();

    assert_eq!(report.dataset_kind(), "private-business-labeled");
    assert_eq!(report.pair_count(), 2);
    assert_eq!(report.positive_pair_count(), 1);
    assert!(report.precision() >= 0.99);
    assert!(report.recall() >= 0.99);
    assert!(report.f1() >= 0.99);
    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"dedupe-quality.v1\""));
    assert!(json.contains("\"dataset_kind\":\"private-business-labeled\""));
    assert!(json.contains("\"target_claim\":\"dedupe_quality_target_met\""));
    assert!(json.contains("\"corpus_origin\":\"private_local\""));
    assert!(json.contains("\"privacy_boundary\":\"redacted_local_aggregate\""));
    assert!(json.contains("\"contains_raw_resume_text\":false"));
    assert!(json.contains("\"contains_resume_paths\":false"));
    assert!(json.contains("\"contains_profile_values\":false"));
    assert!(json.contains("\"contains_sample_ids\":false"));
    assert!(json.contains("\"contains_document_ids\":false"));
    assert!(json.contains(
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\""
    ));
    assert!(json.contains(
        "\"annotation_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\""
    ));
    assert!(json.contains("\"dedupe_taxonomy\":\"resume-ir.dedupe.v1\""));
    assert!(json.contains(
        "\"scope\":\"private business dedupe-quality benchmark; aggregate redacted report only\""
    ));
    assert!(!json.contains("private-dedupe-sample-001"));
    assert!(!json.contains("private-left-doc-001"));
    assert!(!json.contains("REDACTION_SENTINEL_DEDUPE_VALUE"));
    assert!(!json.contains("Synthetic Duplicate Candidate"));
    assert!(!json.contains("Synthetic Commerce"));
    assert!(!json.contains("Synthetic University"));
    assert!(!json.contains("Payments"));

    let gate = DedupeQualityGateConfig::new(0.90, 0.90, 0.90)
        .with_min_pairs(2)
        .with_min_positive_pairs(1)
        .require_private_business_labeled();
    let evaluation = evaluate_dedupe_quality_gate_json(&json, gate).unwrap();
    assert_eq!(evaluation.dataset_kind(), "private-business-labeled");
    assert_eq!(evaluation.pair_count(), 2);
    assert!(evaluation.f1() >= 0.99);
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
    assert!(!json.contains("REDACTION_SENTINEL_OCR_TEXT"));

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
        "\"failed_document_count\":50,",
        "\"render_failure_count\":50,",
        "\"ocr_failure_count\":0,",
        "\"run_budget_exhausted\":false,",
        "\"engine_kind\":\"tesseract\",",
        "\"total_ms\":200000.0,",
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
fn ocr_throughput_gate_rejects_private_real_report_with_inconsistent_page_counts() {
    let report = minimal_private_real_ocr_throughput_json(100, 200, 150, 100, 40_000.0, 2.5);
    let config = OcrThroughputGateConfig::new(1, 1_000.0, 1.0).require_private_real_corpus();

    let error = evaluate_ocr_throughput_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus OCR throughput counts are inconsistent"));
}

#[test]
fn ocr_throughput_gate_rejects_private_real_report_with_inconsistent_throughput() {
    let report = minimal_private_real_ocr_throughput_json(500, 200, 150, 500, 200_000.0, 9.9);
    let config = OcrThroughputGateConfig::new(500, 1_000.0, 1.0).require_private_real_corpus();

    let error = evaluate_ocr_throughput_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus OCR throughput metric counts do not match scores"));
}

#[test]
fn private_ocr_throughput_benchmark_outputs_redacted_gateable_report() {
    let root = temp_dir("private-ocr-throughput-root");
    fs::write(
        root.join("private-sample.pdf"),
        b"%PDF synthetic private sample",
    )
    .unwrap();
    fs::write(
        root.join("ignored-private-sample.docx"),
        b"ignored synthetic docx",
    )
    .unwrap();
    let renderer = pdf_render_fixture_script("private-ocr-throughput-renderer");
    let ocr = ocr_fixture_script("private-ocr-throughput-ocr");
    let manifests = PrivateOcrManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        "1111111111111111111111111111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222222222222222222222222222",
    )
    .unwrap();
    let config = PrivateOcrThroughputConfig::new(
        &root,
        PrivateOcrBenchmarkEngine::local_command(&ocr).unwrap(),
        PrivatePdfRenderEngine::local_command(&renderer).unwrap(),
        manifests,
    )
    .unwrap()
    .with_max_documents(1)
    .unwrap()
    .with_max_pages(2)
    .unwrap()
    .with_pages_per_document(2)
    .unwrap()
    .with_page_timeout_ms(5_000)
    .unwrap();

    let report = run_private_ocr_throughput_benchmark(config).unwrap();

    assert_eq!(report.page_count(), 2);
    assert_eq!(report.document_count(), 1);
    assert_eq!(report.scanned_document_count(), 1);
    assert_eq!(report.failed_document_count(), 0);
    assert_eq!(report.render_failure_count(), 0);
    assert_eq!(report.ocr_failure_count(), 0);
    assert_eq!(report.engine_kind(), "local-command");
    assert_eq!(report.latency().samples(), 2);
    assert!(report.pages_per_second() > 0.0);
    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"ocr-throughput.v1\""));
    assert!(json.contains("\"dataset_kind\":\"private-real-corpus\""));
    assert!(json.contains("\"document_count\":1"));
    assert!(json.contains("\"scanned_document_count\":1"));
    assert!(json.contains("\"failed_document_count\":0"));
    assert!(json.contains("\"render_failure_count\":0"));
    assert!(json.contains("\"ocr_failure_count\":0"));
    assert!(json.contains("\"run_budget_exhausted\":false"));
    assert!(json.contains("\"page_count\":2"));
    assert!(json.contains("\"target_claim\":\"ocr_throughput_target_met\""));
    assert!(json.contains("\"contains_raw_ocr_text\":false"));
    assert!(json.contains("\"contains_resume_paths\":false"));
    assert!(json.contains("\"contains_command_paths\":false"));
    assert!(json.contains("\"scope\":\"private real-corpus OCR throughput benchmark; aggregate redacted report only\""));
    assert!(!json.contains(path_str(&root)));
    assert!(!json.contains(path_str(&renderer)));
    assert!(!json.contains(path_str(&ocr)));
    assert!(!json.contains("private-sample.pdf"));
    assert!(!json.contains("REDACTION_SENTINEL_OCR_TEXT"));
    assert!(!json.contains("REDACTION_SENTINEL_PAGE_IMAGE"));

    let gate = OcrThroughputGateConfig::new(2, 10_000.0, 0.001).require_private_real_corpus();
    let evaluation = evaluate_ocr_throughput_gate_json(&json, gate).unwrap();
    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.page_count(), 2);

    remove_dir(&root);
    remove_dir(renderer.parent().unwrap());
    remove_dir(ocr.parent().unwrap());
}

#[test]
fn private_ocr_throughput_benchmark_skips_failed_documents_with_redacted_aggregates() {
    let root = temp_dir("private-ocr-throughput-failures-root");
    fs::write(root.join("bad-private-sample.pdf"), b"FAIL_RENDER").unwrap();
    fs::write(
        root.join("good-private-sample.pdf"),
        b"%PDF synthetic private sample",
    )
    .unwrap();
    let renderer = flaky_pdf_render_fixture_script("private-ocr-throughput-flaky-renderer");
    let ocr = ocr_fixture_script("private-ocr-throughput-flaky-ocr");
    let manifests = PrivateOcrManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        "1111111111111111111111111111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222222222222222222222222222",
    )
    .unwrap();
    let config = PrivateOcrThroughputConfig::new(
        &root,
        PrivateOcrBenchmarkEngine::local_command(&ocr).unwrap(),
        PrivatePdfRenderEngine::local_command(&renderer).unwrap(),
        manifests,
    )
    .unwrap()
    .with_max_documents(2)
    .unwrap()
    .with_max_pages(1)
    .unwrap()
    .with_page_timeout_ms(5_000)
    .unwrap();

    let report = run_private_ocr_throughput_benchmark(config).unwrap();

    assert_eq!(report.page_count(), 1);
    assert_eq!(report.document_count(), 2);
    assert_eq!(report.scanned_document_count(), 1);
    assert_eq!(report.failed_document_count(), 1);
    assert_eq!(report.render_failure_count(), 1);
    assert_eq!(report.ocr_failure_count(), 0);
    let json = report.to_redacted_json();
    assert!(json.contains("\"failed_document_count\":1"));
    assert!(json.contains("\"render_failure_count\":1"));
    assert!(json.contains("\"ocr_failure_count\":0"));
    assert!(!json.contains(path_str(&root)));
    assert!(!json.contains(path_str(&renderer)));
    assert!(!json.contains("bad-private-sample.pdf"));
    assert!(!json.contains("FAIL_RENDER"));

    let gate = OcrThroughputGateConfig::new(1, 10_000.0, 0.001).require_private_real_corpus();
    let evaluation = evaluate_ocr_throughput_gate_json(&json, gate).unwrap();
    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.page_count(), 1);

    remove_dir(&root);
    remove_dir(renderer.parent().unwrap());
    remove_dir(ocr.parent().unwrap());
}

#[test]
fn private_ocr_throughput_benchmark_reports_run_budget_exhaustion_without_gate_clearance() {
    let root = temp_dir("private-ocr-throughput-budget-root");
    fs::write(
        root.join("private-budget-sample.pdf"),
        b"%PDF synthetic private sample",
    )
    .unwrap();
    let renderer = pdf_render_fixture_script("private-ocr-throughput-budget-renderer");
    let ocr = slow_ocr_fixture_script("private-ocr-throughput-budget-ocr");
    let manifests = PrivateOcrManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        "1111111111111111111111111111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222222222222222222222222222",
    )
    .unwrap();
    let config = PrivateOcrThroughputConfig::new(
        &root,
        PrivateOcrBenchmarkEngine::local_command(&ocr).unwrap(),
        PrivatePdfRenderEngine::local_command(&renderer).unwrap(),
        manifests,
    )
    .unwrap()
    .with_max_documents(1)
    .unwrap()
    .with_max_pages(2)
    .unwrap()
    .with_pages_per_document(2)
    .unwrap()
    .with_max_run_ms(10)
    .unwrap()
    .with_page_timeout_ms(5_000)
    .unwrap();

    let report = run_private_ocr_throughput_benchmark(config).unwrap();

    assert_eq!(report.page_count(), 1);
    assert!(report.run_budget_exhausted());
    let json = report.to_redacted_json();
    assert!(json.contains("\"run_budget_exhausted\":true"));
    assert!(!json.contains(path_str(&root)));
    assert!(!json.contains(path_str(&ocr)));

    let gate = OcrThroughputGateConfig::new(1, 10_000.0, 0.001).require_private_real_corpus();
    let error = evaluate_ocr_throughput_gate_json(&json, gate).unwrap_err();
    assert!(error
        .to_string()
        .contains("private real-corpus OCR benchmark run budget exhausted"));

    remove_dir(&root);
    remove_dir(renderer.parent().unwrap());
    remove_dir(ocr.parent().unwrap());
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
fn private_business_vector_quality_report_outputs_redacted_gateable_report() {
    let command = embedding_fixture_script("private-business-vector-quality-command");
    let manifests = PrivateVectorQualityManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = VectorQualityConfig::new(&command, "fixture-local-model", 3)
        .unwrap()
        .with_top_k(1);

    let report = run_private_business_vector_quality_jsonl(
        &private_business_vector_quality_dataset(),
        config,
        manifests,
    )
    .unwrap();

    assert_eq!(report.dataset_kind(), "private-business-labeled");
    assert_eq!(report.sample_count(), 2);
    assert_eq!(report.candidate_count(), 4);
    assert_eq!(report.top_k(), 1);
    assert!(report.recall_at_k() >= 0.99);
    assert!(report.mrr() >= 0.99);
    assert!(report.ndcg_at_k() >= 0.99);
    assert_eq!(report.zero_recall_queries(), 0);
    let json = report.to_redacted_json();
    assert!(json.contains("\"schema_version\":\"vector-quality.v1\""));
    assert!(json.contains("\"dataset_kind\":\"private-business-labeled\""));
    assert!(json.contains("\"target_claim\":\"vector_quality_target_met\""));
    assert!(json.contains("\"corpus_origin\":\"private_local\""));
    assert!(json.contains("\"privacy_boundary\":\"redacted_local_aggregate\""));
    assert!(json.contains("\"contains_raw_queries\":false"));
    assert!(json.contains("\"contains_candidate_text\":false"));
    assert!(json.contains("\"contains_resume_paths\":false"));
    assert!(json.contains("\"contains_sample_ids\":false"));
    assert!(json.contains("\"contains_candidate_ids\":false"));
    assert!(json.contains("\"contains_vectors\":false"));
    assert!(json.contains(
        "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\""
    ));
    assert!(json.contains(
        "\"annotation_manifest_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\""
    ));
    assert!(json.contains(
        "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\""
    ));
    assert!(json.contains("\"vector_taxonomy\":\"resume-ir.vector-quality.v1\""));
    assert!(json.contains(
        "\"scope\":\"private business vector-quality benchmark; aggregate redacted report only\""
    ));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("fixture-local-model"));
    assert!(!json.contains("\"dimension\""));
    assert!(!json.contains("private-vector-sample-001"));
    assert!(!json.contains("private-vector-candidate-001"));
    assert!(!json.contains("REDACTION_SENTINEL_VECTOR_QUERY"));
    assert!(!json.contains("REDACTION_SENTINEL_VECTOR_CANDIDATE"));
    assert!(!json.contains("1.0,0.0,0.0"));

    let gate = VectorQualityGateConfig::new(2, 0.90, 0.90, 0.90)
        .with_max_zero_recall_queries(0)
        .require_private_business_labeled();
    let evaluation = evaluate_vector_quality_gate_json(&json, gate).unwrap();
    assert_eq!(evaluation.dataset_kind(), "private-business-labeled");
    assert_eq!(evaluation.sample_count(), 2);
    assert!(evaluation.recall_at_k() >= 0.99);

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
            "\"query_total_ms\":{}.0,",
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
        query_count * 10,
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

fn minimal_private_real_ocr_throughput_json(
    page_count: usize,
    document_count: usize,
    scanned_document_count: usize,
    samples: usize,
    total_ms: f64,
    pages_per_second: f64,
) -> String {
    format!(
        concat!(
            "{{",
            "\"schema_version\":\"ocr-throughput.v1\",",
            "\"run_id\":\"ocr_release_test\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"private-real-corpus\",",
            "\"page_count\":{},",
            "\"document_count\":{},",
            "\"scanned_document_count\":{},",
            "\"failed_document_count\":0,",
            "\"render_failure_count\":0,",
            "\"ocr_failure_count\":0,",
            "\"run_budget_exhausted\":false,",
            "\"engine_kind\":\"tesseract\",",
            "\"total_ms\":{},",
            "\"page_latency_ms\":{{\"samples\":{},\"p50\":250.0,\"p95\":450.0,\"p99\":800.0}},",
            "\"pages_per_second\":{},",
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
            "\"scope\":\"private real-corpus OCR throughput benchmark; aggregate redacted report only\"",
            "}}"
        ),
        page_count,
        document_count,
        scanned_document_count,
        total_ms,
        samples,
        pages_per_second,
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

fn slow_ocr_fixture_script(label: &str) -> PathBuf {
    let path = temp_dir(label).join(ocr_fixture_file_name());
    fs::write(&path, slow_ocr_fixture_script_body()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
    }
    path
}

fn pdf_render_fixture_script(label: &str) -> PathBuf {
    let path = temp_dir(label).join(pdf_render_fixture_file_name());
    fs::write(&path, pdf_render_fixture_script_body()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
    }
    path
}

fn flaky_pdf_render_fixture_script(label: &str) -> PathBuf {
    let path = temp_dir(label).join(pdf_render_fixture_file_name());
    fs::write(&path, flaky_pdf_render_fixture_script_body()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
    }
    path
}

fn query_fixture_script(label: &str) -> PathBuf {
    let path = temp_dir(label).join(query_fixture_file_name());
    fs::write(&path, query_fixture_script_body()).unwrap();
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

fn private_query_set_file(label: &str, query_count: usize) -> PathBuf {
    let path = temp_dir(label).join("private-query-set.jsonl");
    let mut lines = String::new();
    for index in 0..query_count {
        lines.push_str(&format!(
            "{{\"sample_id\":\"private-query-sample-{index:06}\",\"query\":\"REDACTION_SENTINEL_PRIVATE_QUERY backend search {index}\"}}\n"
        ));
    }
    fs::write(&path, lines).unwrap();
    path
}

fn private_business_field_quality_dataset() -> String {
    concat!(
        "{\"sample_id\":\"private-field-sample-001\",",
        "\"text\":\"Name: Synthetic Field Candidate\\n",
        "Summary: REDACTION_SENTINEL_FIELD_VALUE\\n",
        "Email: field-candidate@example.test\\n",
        "Phone: +1 (415) 555-0132\\n",
        "Education\\n",
        "School: Synthetic 985 University (985/211/双一流)\\n",
        "Degree: Bachelor of Engineering\\n",
        "Major: Computer Science\\n",
        "Location: Shanghai\\n",
        "Experience\\n",
        "Company: Synthetic Commerce Inc.\\n",
        "Title: Product Manager\\n",
        "2020年1月 - 2024年3月\\n",
        "Certifications\\n",
        "PMP\\n",
        "Skills: Rust, Java\",",
        "\"expected\":[",
        "{\"type\":\"name\",\"normalized\":\"synthetic field candidate\"},",
        "{\"type\":\"email\",\"normalized\":\"field-candidate@example.test\"},",
        "{\"type\":\"phone\",\"normalized\":\"+14155550132\"},",
        "{\"type\":\"school\",\"normalized\":\"synthetic 985 university (985/211/双一流)\"},",
        "{\"type\":\"school_tier\",\"normalized\":\"985\"},",
        "{\"type\":\"school_tier\",\"normalized\":\"211\"},",
        "{\"type\":\"school_tier\",\"normalized\":\"double_first_class\"},",
        "{\"type\":\"degree\",\"normalized\":\"bachelor\"},",
        "{\"type\":\"major\",\"normalized\":\"computer_science\"},",
        "{\"type\":\"location\",\"normalized\":\"shanghai\"},",
        "{\"type\":\"company\",\"normalized\":\"synthetic commerce\"},",
        "{\"type\":\"title\",\"normalized\":\"product_manager\"},",
        "{\"type\":\"date_range\",\"normalized\":\"2020-01/2024-03\"},",
        "{\"type\":\"years_experience\",\"normalized\":\"4.2\"},",
        "{\"type\":\"certificate\",\"normalized\":\"pmp\"},",
        "{\"type\":\"skill\",\"normalized\":\"Rust\"},",
        "{\"type\":\"skill\",\"normalized\":\"Java\"}",
        "]}\n"
    )
    .to_string()
}

fn private_business_dedupe_quality_dataset() -> String {
    concat!(
        "{\"sample_id\":\"private-dedupe-sample-001\",",
        "\"left\":{\"id\":\"private-left-doc-001\",\"name\":\"Synthetic Duplicate Candidate\",\"schools\":[\"Synthetic University\"],\"companies\":[\"Synthetic Commerce\"],\"skills\":[\"Rust\",\"Payments\",\"REDACTION_SENTINEL_DEDUPE_VALUE\"]},",
        "\"right\":{\"id\":\"private-right-doc-001\",\"name\":\"synthetic duplicate candidate\",\"schools\":[\"synthetic university\"],\"companies\":[\"Synthetic Commerce\"],\"skills\":[\"Rust\",\"Search\"]},",
        "\"duplicate\":true}\n",
        "{\"sample_id\":\"private-dedupe-sample-002\",",
        "\"left\":{\"id\":\"private-left-doc-002\",\"name\":\"Synthetic Duplicate Candidate\",\"schools\":[\"Synthetic University\"],\"companies\":[\"Synthetic Commerce\"],\"skills\":[\"Rust\"]},",
        "\"right\":{\"id\":\"private-right-doc-002\",\"name\":\"Different Synthetic Candidate\",\"schools\":[\"Synthetic University\"],\"companies\":[\"Synthetic Commerce\"],\"skills\":[\"Rust\"]},",
        "\"duplicate\":false}\n",
    )
    .to_string()
}

fn private_business_vector_quality_dataset() -> String {
    concat!(
        "{\"sample_id\":\"private-vector-sample-001\",\"query\":\"REDACTION_SENTINEL_VECTOR_QUERY backend java payment\",",
        "\"candidates\":[",
        "{\"id\":\"private-vector-candidate-001\",\"text\":\"REDACTION_SENTINEL_VECTOR_CANDIDATE Java payment backend search engineer\",\"relevant\":true},",
        "{\"id\":\"private-vector-candidate-002\",\"text\":\"Synthetic sales operations\",\"relevant\":false}",
        "]}\n",
        "{\"sample_id\":\"private-vector-sample-002\",\"query\":\"REDACTION_SENTINEL_VECTOR_QUERY rust indexing\",",
        "\"candidates\":[",
        "{\"id\":\"private-vector-candidate-003\",\"text\":\"REDACTION_SENTINEL_VECTOR_CANDIDATE Rust indexing platform engineer\",\"relevant\":true},",
        "{\"id\":\"private-vector-candidate-004\",\"text\":\"Synthetic HR partner\",\"relevant\":false}",
        "]}\n",
    )
    .to_string()
}

#[cfg(unix)]
fn ocr_fixture_file_name() -> &'static str {
    "ocr-fixture.sh"
}

#[cfg(unix)]
fn pdf_render_fixture_file_name() -> &'static str {
    "pdf-render-fixture.sh"
}

#[cfg(windows)]
fn ocr_fixture_file_name() -> &'static str {
    "ocr-fixture.cmd"
}

#[cfg(windows)]
fn pdf_render_fixture_file_name() -> &'static str {
    "pdf-render-fixture.cmd"
}

#[cfg(unix)]
fn query_fixture_file_name() -> &'static str {
    "query-fixture.sh"
}

#[cfg(windows)]
fn query_fixture_file_name() -> &'static str {
    "query-fixture.cmd"
}

#[cfg(unix)]
fn ocr_fixture_script_body() -> &'static str {
    "#!/bin/sh\nprintf 'resume-ir-ocr-v1\\nconfidence=0.97\\ntext:\\nSynthetic OCR Candidate page %s REDACTION_SENTINEL_OCR_TEXT\\n' \"$RESUME_IR_OCR_PAGE_NO\"\n"
}

#[cfg(unix)]
fn slow_ocr_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "sleep 0.05\n",
        "printf 'resume-ir-ocr-v1\\nconfidence=0.97\\ntext:\\nSynthetic OCR Candidate page %s REDACTION_SENTINEL_OCR_TEXT\\n' \"$RESUME_IR_OCR_PAGE_NO\"\n",
    )
}

#[cfg(unix)]
fn pdf_render_fixture_script_body() -> &'static str {
    "#!/bin/sh\nprintf 'REDACTION_SENTINEL_PAGE_IMAGE %s SYNTHETIC_PIXELS' \"$RESUME_IR_PDF_RENDER_PAGE_NO\"\n"
}

#[cfg(unix)]
fn flaky_pdf_render_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "if grep -q FAIL_RENDER \"$RESUME_IR_PDF_RENDER_INPUT_PATH\"; then\n",
        "  exit 7\n",
        "fi\n",
        "printf 'REDACTION_SENTINEL_PAGE_IMAGE %s SYNTHETIC_PIXELS' \"$RESUME_IR_PDF_RENDER_PAGE_NO\"\n",
    )
}

#[cfg(unix)]
fn query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "if grep -q REDACTION_SENTINEL_PRIVATE_QUERY \"$RESUME_IR_QUERY_INPUT_PATH\"; then\n",
        "  printf 'resume-ir-query-v1\\nhits=%s\\n' \"$RESUME_IR_QUERY_TOP_K\"\n",
        "else\n",
        "  printf 'resume-ir-query-v1\\nhits=0\\n'\n",
        "fi\n",
    )
}

#[cfg(windows)]
fn ocr_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "echo resume-ir-ocr-v1\r\n",
        "echo confidence=0.97\r\n",
        "echo text:\r\n",
        "echo Synthetic OCR Candidate page %RESUME_IR_OCR_PAGE_NO% REDACTION_SENTINEL_OCR_TEXT\r\n",
        "exit /b 0\r\n"
    )
}

#[cfg(windows)]
fn slow_ocr_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "ping -n 2 127.0.0.1 >nul\r\n",
        "echo resume-ir-ocr-v1\r\n",
        "echo confidence=0.97\r\n",
        "echo text:\r\n",
        "echo Synthetic OCR Candidate page %RESUME_IR_OCR_PAGE_NO% REDACTION_SENTINEL_OCR_TEXT\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn pdf_render_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "echo REDACTION_SENTINEL_PAGE_IMAGE %RESUME_IR_PDF_RENDER_PAGE_NO% SYNTHETIC_PIXELS\r\n",
        "exit /b 0\r\n"
    )
}

#[cfg(windows)]
fn flaky_pdf_render_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "findstr /C:FAIL_RENDER \"%RESUME_IR_PDF_RENDER_INPUT_PATH%\" >nul 2>nul\r\n",
        "if %ERRORLEVEL%==0 exit /b 7\r\n",
        "echo REDACTION_SENTINEL_PAGE_IMAGE %RESUME_IR_PDF_RENDER_PAGE_NO% SYNTHETIC_PIXELS\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" \"%RESUME_IR_QUERY_INPUT_PATH%\" >nul\r\n",
        "if errorlevel 1 (\r\n",
        "  echo resume-ir-query-v1\r\n",
        "  echo hits=0\r\n",
        ") else (\r\n",
        "  echo resume-ir-query-v1\r\n",
        "  echo hits=%RESUME_IR_QUERY_TOP_K%\r\n",
        ")\r\n",
        "exit /b 0\r\n",
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
