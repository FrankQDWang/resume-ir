use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "support/private_query_runner.rs"]
mod private_query_runner_support;
#[path = "support/private_query.rs"]
mod private_query_support;

use private_query_runner_support::{
    legacy_private_query_set_file, private_query_set_file_with_bucket_queries,
    private_query_set_file_with_custom_shape_query, private_query_set_file_without_summary,
    write_private_query_set_summary, write_private_query_set_summary_with_split,
};
use private_query_support::{
    assert_private_query_stage_latency, private_query_corpus_summary_json, private_query_set_file,
    private_query_set_file_with_buckets, private_query_set_file_with_buckets_and_source_kind,
    private_query_set_summary_path, private_query_test_alpha_id, private_query_test_buckets,
    private_query_test_shape, private_query_test_shape_for_bucket,
};

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
    PrivateQueryCorpusSummary, PrivateQueryManifestDigests, PrivateVectorQualityManifestDigests,
    SyntheticBenchmarkConfig, SyntheticOcrBenchmarkConfig, SyntheticOcrBenchmarkEngine,
    VectorQualityConfig, VectorQualityGateConfig,
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
    let fixture_document_count = 8_721;
    let query_set = private_query_set_file("private-query-benchmark-set", 500);
    let command = query_fixture_script("private-query-benchmark-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(fixture_document_count, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
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

    assert_eq!(report.document_count(), fixture_document_count);
    assert_eq!(report.searchable_document_count(), fixture_document_count);
    assert_eq!(
        report.vector_indexed_document_count(),
        fixture_document_count
    );
    assert_eq!(report.query_count(), 500);
    assert_eq!(report.top_k(), 10);
    assert_eq!(report.zero_result_queries(), 0);
    assert_eq!(report.latency().samples(), 500);
    assert!(report.qps() > 0.0);
    let json = report.to_redacted_json();
    assert_private_query_report_semantics(&json, fixture_document_count);
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));
    assert!(!json.contains("private-query-sample-"));

    let gate = BenchmarkGateConfig::new(8_000, 500, 10_000.0).require_private_real_corpus();
    let evaluation = evaluate_benchmark_gate_json(&json, gate).unwrap();
    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.document_count(), fixture_document_count);
    assert_eq!(evaluation.query_count(), 500);

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_uses_single_resident_batch_command() {
    let query_set = private_query_set_file("private-query-resident-batch-set", 3);
    let command = query_fixture_script_with_body(
        "private-query-resident-batch-command",
        resident_batch_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(3)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(report["query_runner"], "resident-batch-command");
    assert_eq!(report["spawn_per_query"], false);
    assert_eq!(report["query_embedding_command_invocations"], 3);
    assert_eq!(report["query_count"], 3);
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[cfg(unix)]
#[test]
fn private_query_benchmark_writes_valid_json_batch_for_quoted_queries() {
    let query_set = private_query_set_file_with_bucket_queries(
        "private-query-resident-batch-quoted-json-set",
        &[(
            "semantic",
            "\"REDACTION_SENTINEL_PRIVATE_QUERY semantic quoted\"",
        )],
    );
    let command = query_fixture_script_with_body(
        "private-query-resident-batch-quoted-json-command",
        json_parsing_resident_batch_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(1)
    .unwrap()
    .with_top_k(5)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();

    assert_eq!(report.query_count(), 1);
    assert_eq!(report.zero_result_queries(), 0);

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_repeats_query_set_to_request_sample_count() {
    let query_set = private_query_set_file("private-query-request-samples-set", 3);
    let command = query_fixture_script_with_body(
        "private-query-request-samples-command",
        resident_batch_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(3)
    .unwrap()
    .with_request_sample_count(8)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(report["query_count"], 3);
    assert_eq!(report["request_sample_count"], 8);
    assert_eq!(report["samples_per_bucket"]["and_3_5"], 8);
    assert_eq!(report["samples_per_bucket"]["single_term"], 0);
    assert_eq!(report["query_embedding_command_invocations"], 8);
    assert_eq!(report["query_latency_ms"]["samples"], 8);
    assert_private_query_stage_latency(&report, 8);
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_repeated_request_samples_use_one_resident_batch_invocation() {
    let query_set = private_query_set_file("private-query-request-samples-one-batch-set", 3);
    let command = query_fixture_script_with_body(
        "private-query-request-samples-one-batch-command",
        resident_batch_invocation_count_query_fixture_script_body(),
    );
    let counter_dir = temp_dir("private-query-resident-batch-invocation-count");
    let counter_path = counter_dir.join("invocations.txt");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command_with_args(
            &command,
            vec![counter_path.to_string_lossy().into_owned()],
        )
        .unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(3)
    .unwrap()
    .with_request_sample_count(8)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let invocation_count = fs::read_to_string(&counter_path)
        .expect("resident batch counter should be written")
        .trim()
        .parse::<usize>()
        .expect("resident batch counter should parse");
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(invocation_count, 1);
    assert_eq!(report["query_runner"], "resident-batch-command");
    assert_eq!(report["spawn_per_query"], false);
    assert_eq!(report["request_sample_count"], 8);
    assert_eq!(report["query_embedding_command_invocations"], 8);
    assert_eq!(report["query_latency_ms"]["samples"], 8);
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains(path_str(&counter_path)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
    remove_dir(&counter_dir);
}

#[test]
fn private_query_benchmark_stratifies_d10k_request_samples_from_scale_gate() {
    let query_set = private_query_set_file_with_buckets(
        "private-query-d10k-stratified-request-samples-set",
        &[
            ("single_term", 50),
            ("and_2", 75),
            ("and_3_5", 150),
            ("and_6_16", 50),
            ("field_filter", 75),
            ("hybrid", 75),
            ("semantic", 25),
        ],
    );
    let command = query_fixture_script_with_body(
        "private-query-d10k-stratified-request-samples-command",
        resident_batch_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(10_000, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(500)
    .unwrap()
    .with_request_sample_count(5_000)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(report["query_count"], 500);
    assert_eq!(report["request_sample_count"], 5_000);
    assert_eq!(report["private_scale_gate"], "D10K_private_calibration");
    let mut total_samples = 0_u64;
    for bucket in private_query_test_buckets().iter().copied() {
        let samples = report["samples_per_bucket"][bucket]
            .as_u64()
            .expect("bucket sample count should be numeric");
        assert!(samples >= 500, "{bucket} should satisfy the D10K floor");
        total_samples += samples;
    }
    assert_eq!(total_samples, 5_000);
    assert_eq!(report["query_embedding_command_invocations"], 5_000);
    assert_private_query_stage_latency(&report, 5_000);
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_synthetic_smoke_does_not_claim_scale_gate() {
    let query_set = private_query_set_file_with_buckets(
        "private-query-smoke-no-scale-gate-set",
        &[("and_3_5", 3)],
    );
    let command = query_fixture_script_with_body(
        "private-query-smoke-no-scale-gate-command",
        resident_batch_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(10_000, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_request_sample_count(3)
    .unwrap()
    .with_synthetic_smoke_evidence();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(report["dataset_kind"], "synthetic-smoke");
    assert!(report.get("private_scale_gate").is_some());
    assert_eq!(report["private_scale_gate"], serde_json::Value::Null);
    assert_eq!(report["target_claim"], "not_evaluated");

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_reports_query_latency_by_bucket() {
    let query_set = private_query_set_file_with_buckets(
        "private-query-bucket-latency-set",
        &[("single_term", 2), ("and_3_5", 2)],
    );
    let command = query_fixture_script_with_body(
        "private-query-bucket-latency-command",
        elapsed_ms_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(4)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(
        report["query_latency_by_bucket"]["single_term"]["samples"],
        2
    );
    assert_eq!(report["query_latency_by_bucket"]["single_term"]["p95"], 4.0);
    assert_eq!(report["query_latency_by_bucket"]["and_3_5"]["samples"], 2);
    assert_eq!(report["query_latency_by_bucket"]["and_3_5"]["p95"], 64.0);
    assert!(report["query_latency_by_bucket"]
        .get("field_filter")
        .is_none());
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_accepts_out_of_order_request_bound_batch_records() {
    let query_set = private_query_set_file_with_bucket_queries(
        "private-query-out-of-order-request-set",
        &[
            ("single_term", "REDACTION_SENTINEL_PRIVATE_QUERY_SINGLE"),
            (
                "and_3_5",
                "REDACTION_SENTINEL_PRIVATE_QUERY_AND backend search alpha",
            ),
        ],
    );
    let command = query_fixture_script_with_body(
        "private-query-out-of-order-request-command",
        out_of_order_request_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(2)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(
        report["query_latency_by_bucket"]["single_term"]["p95"],
        11.0
    );
    assert_eq!(report["query_latency_by_bucket"]["and_3_5"]["p95"], 44.0);
    assert_eq!(report["zero_result_queries"], 0);
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_reports_stage_latency_by_bucket() {
    let query_set = private_query_set_file_with_bucket_queries(
        "private-query-bucket-stage-set",
        &[
            ("single_term", "REDACTION_SENTINEL_PRIVATE_QUERY_SINGLE"),
            (
                "and_3_5",
                "REDACTION_SENTINEL_PRIVATE_QUERY_AND backend search alpha",
            ),
        ],
    );
    let command = query_fixture_script_with_body(
        "private-query-bucket-stage-command",
        bucket_stage_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(2)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(
        report["stage_latency_by_bucket_ms"]["single_term"]["query_parse"]["samples"],
        1
    );
    assert_eq!(
        report["stage_latency_by_bucket_ms"]["single_term"]["query_parse"]["p95"],
        1.0
    );
    assert_eq!(
        report["stage_latency_by_bucket_ms"]["and_3_5"]["query_parse"]["samples"],
        1
    );
    assert_eq!(
        report["stage_latency_by_bucket_ms"]["and_3_5"]["query_parse"]["p95"],
        21.0
    );
    assert!(report["stage_latency_by_bucket_ms"]
        .get("field_filter")
        .is_none());
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_reports_bounded_stage_histograms_by_bucket() {
    let query_set = private_query_set_file_with_bucket_queries(
        "private-query-bucket-stage-histogram-set",
        &[
            ("single_term", "REDACTION_SENTINEL_PRIVATE_QUERY_SINGLE"),
            (
                "and_3_5",
                "REDACTION_SENTINEL_PRIVATE_QUERY_AND backend search alpha",
            ),
        ],
    );
    let command = query_fixture_script_with_body(
        "private-query-bucket-stage-histogram-command",
        bucket_stage_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(2)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    let global_query_parse = &report["stage_histogram_ms"]["query_parse"];
    assert_eq!(global_query_parse["samples"], 2);
    assert_eq!(global_query_parse["bins"].as_array().unwrap().len(), 13);
    assert_eq!(stage_histogram_bin_count(global_query_parse, 10.0), 1);
    assert_eq!(stage_histogram_bin_count(global_query_parse, 25.0), 2);
    assert_eq!(global_query_parse["overflow_count"], 0);

    let single_query_parse = &report["stage_histogram_by_bucket_ms"]["single_term"]["query_parse"];
    assert_eq!(single_query_parse["samples"], 1);
    assert_eq!(stage_histogram_bin_count(single_query_parse, 1.0), 1);
    assert_eq!(single_query_parse["overflow_count"], 0);

    let and_query_parse = &report["stage_histogram_by_bucket_ms"]["and_3_5"]["query_parse"];
    assert_eq!(and_query_parse["samples"], 1);
    assert_eq!(stage_histogram_bin_count(and_query_parse, 10.0), 0);
    assert_eq!(stage_histogram_bin_count(and_query_parse, 25.0), 1);
    assert_eq!(and_query_parse["overflow_count"], 0);
    assert!(report["stage_histogram_by_bucket_ms"]
        .get("field_filter")
        .is_none());
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_d10k_static_query_set_below_unique_bucket_minimums() {
    let query_set = private_query_set_file_with_buckets(
        "private-query-d10k-undercovered-static-buckets-set",
        &[
            ("single_term", 493),
            ("and_2", 1),
            ("and_3_5", 1),
            ("and_6_16", 1),
            ("field_filter", 1),
            ("hybrid", 1),
            ("semantic", 1),
        ],
    );
    let command = query_fixture_script_with_body(
        "private-query-d10k-undercovered-static-buckets-command",
        resident_batch_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(10_000, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(500)
    .unwrap()
    .with_request_sample_count(5_000)
    .unwrap();

    let error = run_private_query_benchmark(config)
        .expect_err("D10K private calibration should require static bucket coverage");

    assert!(error
        .to_string()
        .contains("private_query_bucket_min_counts"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_defaults_d10k_request_bucket_floor() {
    let query_set = private_query_set_file_with_buckets(
        "private-query-d10k-default-bucket-floor-set",
        &[
            ("single_term", 493),
            ("and_2", 1),
            ("and_3_5", 1),
            ("and_6_16", 1),
            ("field_filter", 1),
            ("hybrid", 1),
            ("semantic", 1),
        ],
    );
    let command = query_set
        .parent()
        .unwrap()
        .join("resident-command-must-not-run");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(10_000, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(500)
    .unwrap()
    .with_request_sample_count(5_000)
    .unwrap();

    let error = run_private_query_benchmark(config)
        .expect_err("D10K private calibration should default to the bucket request floor");

    assert!(error
        .to_string()
        .contains("private_query_bucket_min_counts"));

    remove_dir(query_set.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_d10k_query_set_not_derived_from_trace_source_search() {
    let query_set = private_query_set_file_with_buckets_and_source_kind(
        "private-query-d10k-local-field-source-set",
        &[
            ("single_term", 50),
            ("and_2", 75),
            ("and_3_5", 150),
            ("and_6_16", 50),
            ("field_filter", 75),
            ("hybrid", 75),
            ("semantic", 25),
        ],
        "local_field",
    );
    let command = query_fixture_script_with_body(
        "private-query-d10k-local-field-source-command",
        resident_batch_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(10_000, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(500)
    .unwrap()
    .with_request_sample_count(5_000)
    .unwrap();

    let error = run_private_query_benchmark(config)
        .expect_err("D10K agent replay should require trace-derived query source");

    assert!(error.to_string().contains("private_query.source_kind"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_uses_record_elapsed_ms_for_latency_summary() {
    let query_set = private_query_set_file("private-query-record-elapsed-set", 4);
    let command = query_fixture_script_with_body(
        "private-query-record-elapsed-command",
        elapsed_ms_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(4)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();

    assert_eq!(report.query_count(), 4);
    assert_eq!(report.latency().samples(), 4);
    assert_eq!(report.latency().p50_ms(), 4.0);
    assert_eq!(report.latency().p95_ms(), 64.0);
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");
    assert_eq!(
        report["query_set_sha256"],
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
    );
    assert_eq!(
        report["tune_sha256"],
        "2222222222222222222222222222222222222222222222222222222222222222"
    );
    assert_eq!(
        report["holdout_sha256"],
        "3333333333333333333333333333333333333333333333333333333333333333"
    );
    assert_eq!(report["query_source"], "trace_source_search_v1");
    assert_eq!(report["tune_bucket_counts"]["and_3_5"], 3);
    assert_eq!(report["holdout_bucket_counts"]["and_3_5"], 1);

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_reports_rss_delta_observability_by_bucket() {
    let query_set =
        private_query_set_file_with_buckets("private-query-rss-delta-set", &[("and_3_5", 3)]);
    let command = query_fixture_script_with_body(
        "private-query-rss-delta-command",
        rss_delta_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(3)
    .unwrap()
    .with_top_k(10)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(report["rss_delta_mb"]["samples"], 3);
    assert_eq!(report["rss_delta_mb"]["p50"], 2.0);
    assert_eq!(report["rss_delta_mb"]["p95"], 4.0);
    assert_eq!(report["rss_delta_mb_by_bucket"]["and_3_5"]["samples"], 3);
    assert_eq!(report["rss_delta_mb_by_bucket"]["and_3_5"]["p95"], 4.0);
    assert!(report["rss_delta_mb_by_bucket"]
        .get("single_term")
        .is_none());
    assert!(report["rss_delta_mb_by_bucket"]
        .get("field_filter")
        .is_none());

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_query_set_without_v2_schema() {
    let query_set = legacy_private_query_set_file("private-query-legacy-set", 1);
    let command = query_fixture_script("private-query-legacy-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert_eq!(
        error.to_string(),
        "benchmark configuration is invalid for private_query.schema_version"
    );

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_query_set_metadata_that_does_not_match_query_text() {
    let query_set = private_query_set_file_with_bucket_queries(
        "private-query-stale-metadata-set",
        &[("and_3_5", "rust backend")],
    );
    let command = query_fixture_script("private-query-stale-metadata-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error.to_string().contains("private_query.query_shape"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_non_canonical_static_query_set_rows() {
    let query_set = private_query_set_file_with_bucket_queries(
        "private-query-non-canonical-row-set",
        &[(
            "and_3_5",
            "ＲＥＤＡＣＴＩＯＮ_SENTINEL_PRIVATE_QUERY backend backend search",
        )],
    );
    let command = query_fixture_script("private-query-non-canonical-row-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error.to_string().contains("private_query.canonical_query"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_query_set_query_outside_semantic_caps() {
    let too_many_terms = (1..=17)
        .map(|index| format!("semanticcap{}", private_query_test_alpha_id(index)))
        .collect::<Vec<_>>()
        .join(" ");
    let query_set = private_query_set_file_with_custom_shape_query(
        "private-query-semantic-cap-set",
        "and_6_16",
        &too_many_terms,
        private_query_test_shape(17, false, false, false, false, true, false),
    );
    let command = query_fixture_script("private-query-semantic-cap-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error.to_string().contains("private_query.query"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_query_set_without_redacted_summary() {
    let query_set = private_query_set_file_without_summary("private-query-missing-summary-set", 1);
    let command = query_fixture_script("private-query-missing-summary-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    let error = error.to_string();
    assert!(
        error.contains("private_query_set_summary"),
        "unexpected error: {error}"
    );

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_query_set_tail_beyond_configured_max() {
    let query_set = private_query_set_file("private-query-tail-beyond-max-set", 2);
    write_private_query_set_summary(&query_set, 1, &[("and_3_5", 1)]);
    let command = query_fixture_script("private-query-tail-beyond-max-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(1)
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error.to_string().contains("private_query_max_queries"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_duplicate_static_queries() {
    let query_set = private_query_set_file_with_bucket_queries(
        "private-query-duplicate-query-set",
        &[
            (
                "and_3_5",
                "REDACTION_SENTINEL_PRIVATE_QUERY backend search a",
            ),
            (
                "and_3_5",
                "REDACTION_SENTINEL_PRIVATE_QUERY backend search a",
            ),
        ],
    );
    let command = query_fixture_script("private-query-duplicate-query-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error.to_string().contains("private_query.duplicate_query"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_duplicate_static_sample_ids() {
    let query_set =
        temp_dir("private-query-duplicate-sample-id-set").join("private-query-set.jsonl");
    fs::write(
        &query_set,
        format!(
            "{}\n{}\n",
            serde_json::json!({
                "schema_version": "resume-ir.query-set.jsonl.v2",
                "sample_id": "private-query-sample-000001",
                "bucket": "and_3_5",
                "query": "REDACTION_SENTINEL_PRIVATE_QUERY backend search a",
                "source_kind": "trace_source_search_v1",
                "query_shape": private_query_test_shape_for_bucket("and_3_5"),
            }),
            serde_json::json!({
                "schema_version": "resume-ir.query-set.jsonl.v2",
                "sample_id": "private-query-sample-000001",
                "bucket": "and_3_5",
                "query": "REDACTION_SENTINEL_PRIVATE_QUERY backend search b",
                "source_kind": "trace_source_search_v1",
                "query_shape": private_query_test_shape_for_bucket("and_3_5"),
            }),
        ),
    )
    .unwrap();
    write_private_query_set_summary(&query_set, 2, &[("and_3_5", 2)]);
    let command = query_fixture_script("private-query-duplicate-sample-id-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private_query.duplicate_sample_id"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_summary_query_count_mismatch() {
    let query_set = private_query_set_file("private-query-summary-count-mismatch-set", 3);
    write_private_query_set_summary(&query_set, 4, &[("and_3_5", 4)]);
    let command = query_fixture_script("private-query-summary-count-mismatch-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    let error = error.to_string();
    assert!(
        error.contains("private_query_set_summary"),
        "unexpected error: {error}"
    );

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_summary_bucket_mismatch() {
    let query_set = private_query_set_file_with_buckets(
        "private-query-summary-bucket-mismatch-set",
        &[("single_term", 1), ("and_2", 1)],
    );
    write_private_query_set_summary(&query_set, 2, &[("and_3_5", 2)]);
    let command = query_fixture_script("private-query-summary-bucket-mismatch-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error.to_string().contains("private_query_set_summary"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_summary_without_holdout_split() {
    let query_set = private_query_set_file("private-query-summary-no-holdout-set", 3);
    write_private_query_set_summary_with_split(&query_set, 3, 3, 0, &[("and_3_5", 3)]);
    let command = query_fixture_script("private-query-summary-no-holdout-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error.to_string().contains("private_query_set_summary"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_summary_bucket_split_mismatch() {
    let query_set = private_query_set_file_with_buckets(
        "private-query-summary-bucket-split-mismatch-set",
        &[("single_term", 1), ("and_2", 1)],
    );
    fs::write(
        private_query_set_summary_path(&query_set),
        concat!(
            "{",
            "\"schema_version\":\"resume-ir.query-set-summary.v2\",",
            "\"privacy_boundary\":\"redacted_local_aggregate\",",
            "\"query_source\":\"trace_source_search_v1\",",
            "\"query_count\":2,",
            "\"tune_query_count\":1,",
            "\"holdout_query_count\":1,",
            "\"bucket_counts\":{\"single_term\":1,\"and_2\":1,\"and_3_5\":0,\"and_6_16\":0,\"field_filter\":0,\"hybrid\":0,\"semantic\":0},",
            "\"tune_bucket_counts\":{\"single_term\":0,\"and_2\":1,\"and_3_5\":0,\"and_6_16\":0,\"field_filter\":0,\"hybrid\":0,\"semantic\":0},",
            "\"holdout_bucket_counts\":{\"single_term\":0,\"and_2\":1,\"and_3_5\":0,\"and_6_16\":0,\"field_filter\":0,\"hybrid\":0,\"semantic\":0},",
            "\"candidate_queries_sampled\":2,",
            "\"zero_hit_queries_dropped\":0,",
            "",
            "\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
            "\"tune_sha256\":\"2222222222222222222222222222222222222222222222222222222222222222\",",
            "\"holdout_sha256\":\"3333333333333333333333333333333333333333333333333333333333333333\",",
            "\"hmac_split\":true,",
            "\"contains_raw_query_text\":false,",
            "\"contains_raw_resume_text\":false,",
            "\"contains_candidate_results\":false,",
            "\"contains_local_paths\":false",
            "}\n"
        ),
    )
    .unwrap();
    let command = query_fixture_script("private-query-summary-bucket-split-mismatch-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error.to_string().contains("private_query_set_summary"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_missing_stage_latency_attestation() {
    let query_set = private_query_set_file("private-query-missing-stage-set", 1);
    let command = query_fixture_script_with_body(
        "private-query-missing-stage-command",
        missing_stage_latency_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert_eq!(
        error.to_string(),
        "benchmark configuration is invalid for private_query_stage_latency_attestation"
    );

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_unbound_resident_batch_records() {
    let query_set = private_query_set_file("private-query-unbound-record-set", 2);
    let command = query_fixture_script_with_body(
        "private-query-unbound-record-command",
        unbound_resident_batch_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private_query_protocol_attestation"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_duplicate_resident_batch_request_ids() {
    let query_set = private_query_set_file("private-query-duplicate-record-set", 2);
    let command = query_fixture_script_with_body(
        "private-query-duplicate-record-command",
        duplicate_resident_batch_request_id_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private_query_protocol_attestation"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_missing_resident_batch_response_as_protocol_failure() {
    let query_set = private_query_set_file("private-query-missing-response-set", 2);
    let command = query_fixture_script_with_body(
        "private-query-missing-response-command",
        missing_resident_batch_response_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert_eq!(
        error.to_string(),
        "benchmark configuration is invalid for private_query_protocol_attestation"
    );

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_oversized_resident_batch_stdout() {
    let query_set = private_query_set_file("private-query-oversized-stdout-set", 1);
    let command = query_fixture_script_with_body(
        "private-query-oversized-stdout-command",
        oversized_stdout_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(1)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private_query_resident_command_output"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_missing_elapsed_ms_attestation() {
    let query_set = private_query_set_file("private-query-missing-elapsed-set", 1);
    let command = query_fixture_script_with_body(
        "private-query-missing-elapsed-command",
        missing_elapsed_ms_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(1)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert_eq!(
        error.to_string(),
        "benchmark configuration is invalid for private_query_elapsed_ms_attestation"
    );

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_missing_hybrid_protocol_attestation() {
    let query_set = private_query_set_file("private-query-missing-attestation-set", 1);
    let command = legacy_query_fixture_script("private-query-missing-attestation-command");
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(1)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private_query_protocol_attestation"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_missing_top_k_protocol_attestation() {
    let query_set = private_query_set_file("private-query-missing-top-k-set", 1);
    let command = query_fixture_script_with_body(
        "private-query-missing-top-k-command",
        missing_top_k_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(1)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private_query_top_k_attestation"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_rejects_mismatched_top_k_protocol_attestation() {
    let query_set = private_query_set_file("private-query-mismatched-top-k-set", 1);
    let command = query_fixture_script_with_body(
        "private-query-mismatched-top-k-command",
        mismatched_top_k_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(1)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let error = run_private_query_benchmark(config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private_query_top_k_attestation"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_benchmark_reports_query_embedding_runtime_attestation() {
    let query_set = private_query_set_file("private-query-query-embedding-set", 2);
    let command = query_fixture_script_with_body(
        "private-query-query-embedding-command",
        query_embedding_attestation_query_fixture_script_body(),
    );
    let corpus_summary = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, true),
    )
    .unwrap();
    let manifests = PrivateQueryManifestDigests::new(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        "1111111111111111111111111111111111111111111111111111111111111111",
    )
    .unwrap();
    let config = PrivateQueryBenchmarkConfig::new(
        &query_set,
        PrivateQueryBenchmarkCommand::resident_batch_command(&command).unwrap(),
        corpus_summary,
        manifests,
    )
    .unwrap()
    .with_max_queries(2)
    .unwrap()
    .with_top_k(10)
    .unwrap()
    .with_timeout_ms(5_000)
    .unwrap();

    let report = run_private_query_benchmark(config).unwrap();
    let json = report.to_redacted_json();
    let report: serde_json::Value =
        serde_json::from_str(&json).expect("private query report JSON should parse");

    assert_eq!(report["query_embedding_runtime"], "local-command");
    assert_eq!(report["query_embedding_command_invocations"], 2);
    assert!(!json.contains(path_str(&query_set)));
    assert!(!json.contains(path_str(&command)));
    assert!(!json.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
}

#[test]
fn private_query_corpus_summary_rejects_partial_hot_index_coverage() {
    let error = PrivateQueryCorpusSummary::from_redacted_json_bytes(
        private_query_corpus_summary_json(8_720, false),
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("private_query_corpus_summary_hot_index"));
}

#[test]
fn private_query_corpus_summary_accepts_partial_hot_index_when_explicitly_allowed_for_smoke() {
    let summary =
        PrivateQueryCorpusSummary::from_redacted_json_bytes_allowing_partial_hot_index_for_smoke(
            private_query_corpus_summary_json(6, false),
        )
        .unwrap();

    assert_eq!(summary.document_count(), 6);
    assert_eq!(summary.searchable_document_count(), 5);
    assert_eq!(summary.vector_indexed_document_count(), 4);
}

#[test]
fn private_query_corpus_summary_accepts_redacted_status_breakdowns() {
    let mut report: serde_json::Value =
        serde_json::from_slice(&private_query_corpus_summary_json(8_720, true)).unwrap();
    let object = report.as_object_mut().unwrap();
    object.insert(
        "document_status_counts".to_string(),
        serde_json::json!({
            "searchable": 8_719,
            "ocr_required": 1
        }),
    );
    object.insert(
        "ingest_job_status_counts".to_string(),
        serde_json::json!({
            "failed_retryable": 1,
            "queued": 2
        }),
    );
    object.insert(
        "ingest_job_kind_status_counts".to_string(),
        serde_json::json!({
            "ocr_document": {
                "failed_retryable": 1
            },
            "update_index": {
                "queued": 2
            }
        }),
    );
    object.insert(
        "ingest_job_failure_counts".to_string(),
        serde_json::json!({
            "ocr_page_budget_exceeded": 1
        }),
    );

    let summary =
        PrivateQueryCorpusSummary::from_redacted_json_bytes(serde_json::to_vec(&report).unwrap())
            .unwrap();

    assert_eq!(summary.document_count(), 8_720);
    assert_eq!(summary.searchable_document_count(), 8_720);
    assert_eq!(summary.vector_indexed_document_count(), 8_720);
}

#[test]
fn benchmark_gate_rejects_private_real_corpus_without_model_manifest_digest() {
    let report = minimal_private_real_benchmark_json(8_720, 500, 25.0, false).replace(
        ",\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\"",
        "",
    );
    let config = BenchmarkGateConfig::new(8_000, 500, 50.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires model manifest digest"));
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
fn benchmark_gate_rejects_private_real_report_without_query_protocol_attestation() {
    let report = minimal_private_real_benchmark_json(100_000, 500, 150.0, false)
        .replace(",\"query_protocol\":\"resume-ir-query-v2\"", "");
    let config = BenchmarkGateConfig::new(100_000, 500, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires query protocol attestation"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_without_hot_index_document_coverage() {
    let report = minimal_private_real_benchmark_json_without_hot_coverage(8_720, 500, 150.0, false);
    let config = BenchmarkGateConfig::new(8_000, 500, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires hot-index document coverage evidence"));
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
fn benchmark_gate_rejects_private_real_report_without_stage_latency() {
    let mut report: serde_json::Value = serde_json::from_str(&minimal_private_real_benchmark_json(
        100_000, 200, 150.0, false,
    ))
    .unwrap();
    report.as_object_mut().unwrap().remove("stage_latency_ms");
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report.to_string(), config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires redacted local boundary"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_without_query_latency_by_bucket() {
    let mut report: serde_json::Value = serde_json::from_str(&minimal_private_real_benchmark_json(
        100_000, 200, 150.0, false,
    ))
    .unwrap();
    report
        .as_object_mut()
        .unwrap()
        .remove("query_latency_by_bucket");
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report.to_string(), config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires redacted local boundary"));
}

#[test]
fn benchmark_gate_rejects_private_real_report_with_bucket_latency_sample_mismatch() {
    let report = minimal_private_real_benchmark_json(100_000, 200, 150.0, false).replace(
        "\"and_3_5\":{\"samples\":200",
        "\"and_3_5\":{\"samples\":199",
    );
    let config = BenchmarkGateConfig::new(100_000, 200, 200.0).require_private_real_corpus();

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
    let report = minimal_private_real_benchmark_json(100_000, 200, 150.0, false)
        .replace("\"hot_index\":true", "\"hot_index\":false");
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
fn benchmark_gate_accepts_private_real_smoke_confidence_when_explicitly_allowed() {
    let report = minimal_private_real_benchmark_json(1, 1, 150.0, false)
        .replace(
            "\"percentile_confidence\":\"sampled\"",
            "\"percentile_confidence\":\"smoke\"",
        )
        .replace("\"total_hits\":100", "\"total_hits\":1");
    let config = BenchmarkGateConfig::new(1, 1, 200.0)
        .require_private_real_corpus()
        .allow_smoke_confidence();

    let evaluation = evaluate_benchmark_gate_json(&report, config).unwrap();

    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.document_count(), 1);
    assert_eq!(evaluation.query_count(), 1);
}

#[test]
fn benchmark_gate_rejects_private_real_smoke_confidence_without_explicit_allowance() {
    let report = minimal_private_real_benchmark_json(1, 1, 150.0, false)
        .replace(
            "\"percentile_confidence\":\"sampled\"",
            "\"percentile_confidence\":\"smoke\"",
        )
        .replace("\"total_hits\":100", "\"total_hits\":1");
    let config = BenchmarkGateConfig::new(1, 1, 200.0).require_private_real_corpus();

    let error = evaluate_benchmark_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private real-corpus benchmark requires redacted local boundary"));
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
    assert!(report.field_metric("wechat").unwrap().f1() >= 0.99);
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
    assert!(!json.contains("Candidate_2026"));
    assert!(!json.contains("candidate_2026"));
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
fn field_quality_gate_rejects_private_business_report_with_inconsistent_aggregate_counts() {
    let report = inconsistent_private_business_field_quality_json();
    let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
        .with_min_samples(1_000)
        .require_private_business_labeled();

    let error = evaluate_field_quality_gate_json(&report, config).unwrap_err();

    assert!(error
        .to_string()
        .contains("private business field quality aggregate counts are inconsistent"));
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
fn ocr_throughput_gate_accepts_private_real_baseline_observed_without_latency_target() {
    let slow_baseline_report = concat!(
        "{\"schema_version\":\"ocr-throughput.v1\",",
        "\"run_id\":\"ocr_baseline_20260613\",",
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
        "\"total_ms\":1250000.0,",
        "\"page_latency_ms\":{\"samples\":500,\"p50\":2200.0,\"p95\":4200.0,\"p99\":6100.0},",
        "\"pages_per_second\":0.4,",
        "\"target_claim\":\"ocr_throughput_baseline_observed\",",
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
    let strict_config =
        OcrThroughputGateConfig::new(500, 1_000.0, 1.0).require_private_real_corpus();

    let strict_error =
        evaluate_ocr_throughput_gate_json(slow_baseline_report, strict_config).unwrap_err();

    assert!(strict_error
        .to_string()
        .contains("OCR page p95 exceeded threshold"));

    let baseline_config = OcrThroughputGateConfig::current_stage_baseline(500);
    let evaluation = evaluate_ocr_throughput_gate_json(slow_baseline_report, baseline_config)
        .expect("current-stage baseline accepts observed OCR metrics");

    assert_eq!(evaluation.dataset_kind(), "private-real-corpus");
    assert_eq!(evaluation.page_count(), 500);
    assert_eq!(evaluation.p95_ms(), 4200.0);
    assert_eq!(evaluation.pages_per_second(), 0.4);
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
fn private_ocr_throughput_benchmark_outputs_redacted_diagnostic_report() {
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
        "1111111111111111111111111111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222222222222222222222222222",
        "3333333333333333333333333333333333333333333333333333333333333333",
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
    assert!(json.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!json.contains("\"target_claim\":\"ocr_throughput_target_met\""));
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
    let error = evaluate_ocr_throughput_gate_json(&json, gate).unwrap_err();
    assert!(error
        .to_string()
        .contains("private real-corpus OCR benchmark requires throughput target claim"));

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
        "1111111111111111111111111111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222222222222222222222222222",
        "3333333333333333333333333333333333333333333333333333333333333333",
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
    let error = evaluate_ocr_throughput_gate_json(&json, gate).unwrap_err();
    assert!(error
        .to_string()
        .contains("private real-corpus OCR benchmark requires throughput target claim"));

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
        "1111111111111111111111111111111111111111111111111111111111111111",
        "2222222222222222222222222222222222222222222222222222222222222222",
        "3333333333333333333333333333333333333333333333333333333333333333",
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
    assert!(json.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!json.contains("\"target_claim\":\"ocr_throughput_target_met\""));
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
    minimal_private_real_benchmark_json_without_hot_coverage(
        document_count,
        query_count,
        p95_ms,
        million_scale_verified,
    )
    .replace(
        "\"query_count\":",
        &format!(
            "\"searchable_document_count\":{document_count},\"vector_indexed_document_count\":{document_count},\"query_count\":"
        ),
    )
}

fn minimal_private_real_benchmark_json_without_hot_coverage(
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
        "\"query_count\":",
        &format!("\"request_sample_count\":{query_count},\"query_count\":"),
    )
    .replace(
        "\"scope\":\"synthetic query benchmark; no raw resume text, paths, or queries included\"",
        "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\"",
    )
    .replace(
        "\"top_k\":10,",
        &format!(
            "\"query_source\":\"trace_source_search_v1\",\"private_scale_gate\":null,\"bucket_counts\":{},\"tune_bucket_counts\":{},\"holdout_bucket_counts\":{},\"samples_per_bucket\":{},\"top_k\":10,",
            private_query_bucket_counts_json(query_count),
            private_query_bucket_counts_json(query_count),
            private_query_bucket_counts_json(0),
            private_query_bucket_counts_json(query_count)
        ),
    )
    .replace(
        "\"zero_result_queries\":",
        &format!(
            "\"query_latency_by_bucket\":{},\"stage_latency_ms\":{},\"stage_latency_by_bucket_ms\":{},\"stage_histogram_ms\":{},\"stage_histogram_by_bucket_ms\":{},\"rss_delta_mb\":{},\"rss_delta_mb_by_bucket\":{},\"zero_result_queries\":",
            private_query_bucket_latency_json(query_count, p95_ms),
            private_query_stage_latency_json(query_count, p95_ms),
            private_query_bucket_stage_latency_json(query_count, p95_ms),
            private_query_stage_histogram_json(query_count),
            private_query_bucket_stage_histogram_json(query_count),
            private_query_latency_json(query_count, p95_ms),
            private_query_bucket_latency_json(query_count, p95_ms)
        ),
    );
    report.pop();
    report.push_str(concat!(
        ",\"corpus_origin\":\"private_local\"",
        ",\"privacy_boundary\":\"redacted_local_aggregate\"",
        ",\"query_protocol\":\"resume-ir-query-v2\"",
        ",\"query_runner\":\"resident-batch-command\"",
        ",\"spawn_per_query\":false",
        ",\"query_mode\":\"hybrid\"",
        ",\"retrieval_layers\":\"fulltext+field+vector+rrf\"",
        ",\"warm_or_cold_definition\":\"current_stage_single_resident_batch_no_extra_warmup\"",
        ",\"cache_state\":\"hot_index_fully_covered_resident_batch_os_cache_uncontrolled\"",
        ",\"query_embedding_runtime\":\"local-command\"",
    ));
    report.push_str(&format!(
        ",\"query_embedding_command_invocations\":{query_count}"
    ));
    report.push_str(concat!(
        ",\"hot_index\":true",
        ",\"hot_path_ocr\":false",
        ",\"hot_path_parsing\":false",
        ",\"hot_path_heavy_model_inference\":false",
        ",\"contains_raw_resume_text\":false",
        ",\"contains_resume_paths\":false",
        ",\"contains_queries\":false",
        ",\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"",
        ",\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\"",
        ",\"tune_sha256\":\"2222222222222222222222222222222222222222222222222222222222222222\"",
        ",\"holdout_sha256\":\"3333333333333333333333333333333333333333333333333333333333333333\"",
        ",\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\"",
        ",\"corpus_summary_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\""
    ));
    report.push('}');
    report
}

fn private_query_bucket_counts_json(query_count: usize) -> String {
    format!(
        "{{\"single_term\":0,\"and_2\":0,\"and_3_5\":{query_count},\"and_6_16\":0,\"field_filter\":0,\"hybrid\":0,\"semantic\":0}}"
    )
}

fn private_query_stage_latency_json(query_count: usize, p95_ms: f64) -> String {
    let summary = format!(
        "{{\"samples\":{query_count},\"min\":1.0,\"mean\":2.0,\"p50\":2.0,\"p95\":{p95_ms},\"p99\":{p95_ms},\"max\":{p95_ms}}}"
    );
    format!(
        "{{\"query_parse\":{summary},\"prefilter\":{summary},\"bm25\":{summary},\"ann\":{summary},\"fusion\":{summary},\"bulk_hydrate\":{summary},\"snippet\":{summary}}}"
    )
}

fn private_query_bucket_stage_latency_json(query_count: usize, p95_ms: f64) -> String {
    format!(
        "{{\"and_3_5\":{}}}",
        private_query_stage_latency_json(query_count, p95_ms)
    )
}

fn private_query_stage_histogram_json(query_count: usize) -> String {
    let histogram = private_query_histogram_json(query_count);
    format!(
        "{{\"query_parse\":{histogram},\"prefilter\":{histogram},\"bm25\":{histogram},\"ann\":{histogram},\"fusion\":{histogram},\"bulk_hydrate\":{histogram},\"snippet\":{histogram}}}"
    )
}

fn private_query_bucket_stage_histogram_json(query_count: usize) -> String {
    format!(
        "{{\"and_3_5\":{}}}",
        private_query_stage_histogram_json(query_count)
    )
}

fn private_query_histogram_json(query_count: usize) -> String {
    let bins = [
        1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0,
        60_000.0,
    ]
    .into_iter()
    .map(|le_ms| format!("{{\"le_ms\":{le_ms},\"count\":{query_count}}}"))
    .collect::<Vec<_>>()
    .join(",");
    format!("{{\"samples\":{query_count},\"bins\":[{bins}],\"overflow_count\":0}}")
}

fn private_query_latency_json(query_count: usize, p95_ms: f64) -> String {
    format!(
        "{{\"samples\":{query_count},\"min\":1.0,\"mean\":2.0,\"p50\":2.0,\"p95\":{p95_ms},\"p99\":{p95_ms},\"max\":{p95_ms}}}"
    )
}

fn private_query_bucket_latency_json(query_count: usize, p95_ms: f64) -> String {
    let summary = private_query_latency_json(query_count, p95_ms);
    format!("{{\"and_3_5\":{summary}}}")
}

fn minimal_private_business_field_quality_json() -> String {
    concat!(
        "{",
        "\"schema_version\":\"field-quality.v1\",",
        "\"run_id\":\"fieldq_test\",",
        "\"platform\":\"test/test\",",
        "\"dataset_kind\":\"private-business-labeled\",",
        "\"sample_count\":1000,",
        "\"expected_mentions\":1875,",
        "\"predicted_mentions\":1875,",
        "\"overall\":{\"true_positive\":1875,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"fields\":{",
        "\"name\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"email\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"phone\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        "\"wechat\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
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

fn inconsistent_private_business_field_quality_json() -> String {
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
        "\"wechat\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
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
    query_fixture_script_with_body(label, query_fixture_script_body())
}

fn legacy_query_fixture_script(label: &str) -> PathBuf {
    query_fixture_script_with_body(label, legacy_query_fixture_script_body())
}

fn query_fixture_script_with_body(label: &str, body: &str) -> PathBuf {
    let path = temp_dir(label).join(query_fixture_file_name());
    fs::write(&path, body).unwrap();
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

fn assert_private_query_report_semantics(json: &str, expected_document_count: usize) {
    let report: serde_json::Value =
        serde_json::from_str(json).expect("private query report JSON should parse");
    assert_eq!(report["schema_version"], "benchmark.v1");
    assert_eq!(report["dataset_kind"], "private-real-corpus");
    assert_eq!(report["target_claim"], "benchmark_baseline_observed");
    assert_eq!(report["query_protocol"], "resume-ir-query-v2");
    assert_eq!(report["query_runner"], "resident-batch-command");
    assert_eq!(report["spawn_per_query"], false);
    assert_eq!(report["query_mode"], "hybrid");
    assert_eq!(report["retrieval_layers"], "fulltext+field+vector+rrf");
    assert_eq!(report["query_embedding_runtime"], "local-command");
    assert_eq!(
        report["warm_or_cold_definition"],
        "current_stage_single_resident_batch_no_extra_warmup"
    );
    assert_eq!(
        report["cache_state"],
        "hot_index_fully_covered_resident_batch_os_cache_uncontrolled"
    );
    assert_eq!(
        report["scope"],
        "private local real-corpus query benchmark; aggregate redacted report only"
    );

    let document_count = report["document_count"]
        .as_u64()
        .expect("document_count should be a number");
    let searchable_count = report["searchable_document_count"]
        .as_u64()
        .expect("searchable_document_count should be a number");
    let vector_count = report["vector_indexed_document_count"]
        .as_u64()
        .expect("vector_indexed_document_count should be a number");
    assert_eq!(document_count, expected_document_count as u64);
    assert!(document_count >= 8_000);
    assert!(searchable_count <= document_count);
    assert!(vector_count <= searchable_count);
    assert_eq!(report["query_count"], 500);
    assert_eq!(report["request_sample_count"], 500);
    assert_eq!(report["bucket_counts"]["and_3_5"], 500);
    assert_eq!(report["bucket_counts"]["single_term"], 0);
    assert_eq!(report["bucket_counts"]["field_filter"], 0);
    assert_eq!(
        report["query_set_sha256"],
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
    );
    assert_eq!(
        report["tune_sha256"],
        "2222222222222222222222222222222222222222222222222222222222222222"
    );
    assert_eq!(
        report["holdout_sha256"],
        "3333333333333333333333333333333333333333333333333333333333333333"
    );
    assert_eq!(report["query_source"], "trace_source_search_v1");
    assert_eq!(report["tune_bucket_counts"]["and_3_5"], 400);
    assert_eq!(report["tune_bucket_counts"]["single_term"], 0);
    assert_eq!(report["holdout_bucket_counts"]["and_3_5"], 100);
    assert_eq!(report["holdout_bucket_counts"]["single_term"], 0);
    assert_eq!(report["samples_per_bucket"]["and_3_5"], 500);
    assert_eq!(report["samples_per_bucket"]["single_term"], 0);
    assert_eq!(report["samples_per_bucket"]["field_filter"], 0);
    assert_eq!(report["query_latency_by_bucket"]["and_3_5"]["samples"], 500);
    assert!(report["query_latency_by_bucket"]
        .get("single_term")
        .is_none());
    assert!(report["query_latency_by_bucket"]
        .get("field_filter")
        .is_none());
    assert_eq!(report["query_embedding_command_invocations"], 500);
    assert_eq!(report["hot_index"], true);
    assert_eq!(report["hot_path_ocr"], false);
    assert_eq!(report["hot_path_parsing"], false);
    assert_eq!(report["hot_path_heavy_model_inference"], false);
    assert_eq!(report["contains_raw_resume_text"], false);
    assert_eq!(report["contains_resume_paths"], false);
    assert_eq!(report["contains_queries"], false);
    assert_eq!(
        report["model_manifest_sha256"],
        "1111111111111111111111111111111111111111111111111111111111111111"
    );
    assert!(report["corpus_summary_sha256"]
        .as_str()
        .is_some_and(|value| value.len() == 64));
    assert_private_query_stage_latency(&report, 500);
}

fn stage_histogram_bin_count(histogram: &serde_json::Value, le_ms: f64) -> u64 {
    histogram["bins"]
        .as_array()
        .expect("stage histogram bins should be an array")
        .iter()
        .find(|bin| {
            bin["le_ms"]
                .as_f64()
                .is_some_and(|value| (value - le_ms).abs() < f64::EPSILON)
        })
        .unwrap_or_else(|| panic!("stage histogram should include <= {le_ms}ms bin"))["count"]
        .as_u64()
        .expect("stage histogram bin count should be numeric")
}

fn private_business_field_quality_dataset() -> String {
    concat!(
        "{\"sample_id\":\"private-field-sample-001\",",
        "\"text\":\"Name: Synthetic Field Candidate\\n",
        "Summary: REDACTION_SENTINEL_FIELD_VALUE\\n",
        "Email: field-candidate@example.test\\n",
        "Phone: +1 (415) 555-0132\\n",
        "WeChat: Candidate_2026\\n",
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
        "{\"type\":\"wechat\",\"normalized\":\"candidate_2026\"},",
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
    resident_batch_query_fixture_script_body()
}

#[cfg(unix)]
fn resident_batch_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "test -n \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 42\n",
        "test -f \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 43\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$RESUME_IR_QUERY_TOP_K\" \"$hits\"\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn json_parsing_resident_batch_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "test -n \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 42\n",
        "test -f \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 43\n",
        "python3 - \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" \"$RESUME_IR_QUERY_TOP_K\" <<'PY'\n",
        "import json\n",
        "import sys\n",
        "path = sys.argv[1]\n",
        "top_k = sys.argv[2]\n",
        "with open(path, encoding='utf-8') as handle:\n",
        "    for line in handle:\n",
        "        record = json.loads(line)\n",
        "        request_id = record['request_id']\n",
        "        query = record['query']\n",
        "        hits = top_k if 'REDACTION_SENTINEL_PRIVATE_QUERY' in query else '0'\n",
        "        print('resume-ir-query-v2')\n",
        "        print(f'request_id={request_id}')\n",
        "        print('mode=hybrid')\n",
        "        print('layers=fulltext+field+vector+rrf')\n",
        "        print(f'top_k={top_k}')\n",
        "        print('query_embedding_runtime=local-command')\n",
        "        print('query_embedding_invocations=1')\n",
        "        print('stage_query_parse_ms=1.0')\n",
        "        print('stage_prefilter_ms=2.0')\n",
        "        print('stage_bm25_ms=3.0')\n",
        "        print('stage_ann_ms=4.0')\n",
        "        print('stage_fusion_ms=5.0')\n",
        "        print('stage_bulk_hydrate_ms=6.0')\n",
        "        print('stage_snippet_ms=7.0')\n",
        "        print('rss_delta_mb=0.0')\n",
        "        print('elapsed_ms=8.0')\n",
        "        print(f'hits={hits}')\n",
        "        print('resume-ir-query-end')\n",
        "PY\n",
    )
}

#[cfg(unix)]
fn unbound_resident_batch_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "test -n \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 42\n",
        "test -f \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 43\n",
        "while IFS= read -r line; do\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$RESUME_IR_QUERY_TOP_K\" \"$hits\"\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn duplicate_resident_batch_request_id_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "test -n \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 42\n",
        "test -f \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 43\n",
        "while IFS= read -r line; do\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=private-query-request-1\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$RESUME_IR_QUERY_TOP_K\" \"$hits\"\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn missing_resident_batch_response_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "test -n \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 42\n",
        "test -f \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 43\n",
        "printf 'resume-ir-query-v2\\nrequest_id=private-query-request-1\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$RESUME_IR_QUERY_TOP_K\" \"$RESUME_IR_QUERY_TOP_K\"\n",
    )
}

#[cfg(unix)]
fn resident_batch_invocation_count_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "counter=\"$1\"\n",
        "test -n \"$counter\" || exit 41\n",
        "count=0\n",
        "if test -f \"$counter\"; then count=$(cat \"$counter\"); fi\n",
        "count=$((count + 1))\n",
        "printf '%s\\n' \"$count\" > \"$counter\"\n",
        "test -n \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 42\n",
        "test -f \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 43\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$RESUME_IR_QUERY_TOP_K\" \"$hits\"\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn elapsed_ms_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "elapsed=1\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nelapsed_ms=%s\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$RESUME_IR_QUERY_TOP_K\" \"$elapsed\" \"$hits\"\n",
        "  elapsed=$((elapsed * 4))\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn bucket_stage_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY_SINGLE*) parse=1.0; prefilter=2.0; bm25=3.0; ann=4.0; fusion=5.0; hydrate=6.0; snippet=7.0; elapsed=8.0; hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *REDACTION_SENTINEL_PRIVATE_QUERY_AND*) parse=21.0; prefilter=22.0; bm25=23.0; ann=24.0; fusion=25.0; hydrate=26.0; snippet=27.0; elapsed=28.0; hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) parse=1.0; prefilter=1.0; bm25=1.0; ann=1.0; fusion=1.0; hydrate=1.0; snippet=1.0; elapsed=1.0; hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=%s\\nstage_prefilter_ms=%s\\nstage_bm25_ms=%s\\nstage_ann_ms=%s\\nstage_fusion_ms=%s\\nstage_bulk_hydrate_ms=%s\\nstage_snippet_ms=%s\\nrss_delta_mb=0.0\\nelapsed_ms=%s\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$RESUME_IR_QUERY_TOP_K\" \"$parse\" \"$prefilter\" \"$bm25\" \"$ann\" \"$fusion\" \"$hydrate\" \"$snippet\" \"$elapsed\" \"$hits\"\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn rss_delta_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "rss=1\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=%s\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$RESUME_IR_QUERY_TOP_K\" \"$rss\" \"$hits\"\n",
        "  rss=$((rss * 2))\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn out_of_order_request_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "test -n \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 42\n",
        "test -f \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\" || exit 43\n",
        "printf 'resume-ir-query-v2\\nrequest_id=private-query-request-2\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nelapsed_ms=44.0\\nhits=2\\nresume-ir-query-end\\n' \"$RESUME_IR_QUERY_TOP_K\"\n",
        "printf 'resume-ir-query-v2\\nrequest_id=private-query-request-1\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nelapsed_ms=11.0\\nhits=1\\nresume-ir-query-end\\n' \"$RESUME_IR_QUERY_TOP_K\"\n",
    )
}

#[cfg(unix)]
fn missing_stage_latency_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nrss_delta_mb=0.0\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$RESUME_IR_QUERY_TOP_K\" \"$hits\"\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn oversized_stdout_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "dd if=/dev/zero bs=1048576 count=9 2>/dev/null\n",
    )
}

#[cfg(unix)]
fn missing_elapsed_ms_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nquery_embedding_runtime=local-command\\nquery_embedding_invocations=1\\nstage_query_parse_ms=1.0\\nstage_prefilter_ms=2.0\\nstage_bm25_ms=3.0\\nstage_ann_ms=4.0\\nstage_fusion_ms=5.0\\nstage_bulk_hydrate_ms=6.0\\nstage_snippet_ms=7.0\\nrss_delta_mb=0.0\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$RESUME_IR_QUERY_TOP_K\" \"$hits\"\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn missing_top_k_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$hits\"\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn mismatched_top_k_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "request_index=1\n",
        "while IFS= read -r line; do\n",
        "  request_id=\"private-query-request-$request_index\"\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=5 ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v2\\nrequest_id=%s\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=5\\nelapsed_ms=8.0\\nhits=%s\\nresume-ir-query-end\\n' \"$request_id\" \"$hits\"\n",
        "  request_index=$((request_index + 1))\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(unix)]
fn query_embedding_attestation_query_fixture_script_body() -> &'static str {
    resident_batch_query_fixture_script_body()
}

#[cfg(unix)]
fn legacy_query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "while IFS= read -r line; do\n",
        "  case \"$line\" in *REDACTION_SENTINEL_PRIVATE_QUERY*) hits=\"$RESUME_IR_QUERY_TOP_K\" ;; *) hits=0 ;; esac\n",
        "  printf 'resume-ir-query-v1\\nhits=%s\\nresume-ir-query-end\\n' \"$hits\"\n",
        "done < \"$RESUME_IR_QUERY_BATCH_INPUT_PATH\"\n",
    )
}

#[cfg(windows)]
fn ocr_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
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
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  set \"request_id=private-query-request-!request_index!\"\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=!request_id!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo stage_query_parse_ms=1.0\r\n",
        "  echo stage_prefilter_ms=2.0\r\n",
        "  echo stage_bm25_ms=3.0\r\n",
        "  echo stage_ann_ms=4.0\r\n",
        "  echo stage_fusion_ms=5.0\r\n",
        "  echo stage_bulk_hydrate_ms=6.0\r\n",
        "  echo stage_snippet_ms=7.0\r\n",
        "  echo rss_delta_mb=0.0\r\n",
        "  echo elapsed_ms=8.0\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn resident_batch_query_fixture_script_body() -> &'static str {
    query_fixture_script_body()
}

#[cfg(windows)]
fn unbound_resident_batch_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo stage_query_parse_ms=1.0\r\n",
        "  echo stage_prefilter_ms=2.0\r\n",
        "  echo stage_bm25_ms=3.0\r\n",
        "  echo stage_ann_ms=4.0\r\n",
        "  echo stage_fusion_ms=5.0\r\n",
        "  echo stage_bulk_hydrate_ms=6.0\r\n",
        "  echo stage_snippet_ms=7.0\r\n",
        "  echo rss_delta_mb=0.0\r\n",
        "  echo elapsed_ms=8.0\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn duplicate_resident_batch_request_id_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=private-query-request-1\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo stage_query_parse_ms=1.0\r\n",
        "  echo stage_prefilter_ms=2.0\r\n",
        "  echo stage_bm25_ms=3.0\r\n",
        "  echo stage_ann_ms=4.0\r\n",
        "  echo stage_fusion_ms=5.0\r\n",
        "  echo stage_bulk_hydrate_ms=6.0\r\n",
        "  echo stage_snippet_ms=7.0\r\n",
        "  echo rss_delta_mb=0.0\r\n",
        "  echo elapsed_ms=8.0\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn missing_resident_batch_response_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "echo resume-ir-query-v2\r\n",
        "echo request_id=private-query-request-1\r\n",
        "echo mode=hybrid\r\n",
        "echo layers=fulltext+field+vector+rrf\r\n",
        "echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "echo query_embedding_runtime=local-command\r\n",
        "echo query_embedding_invocations=1\r\n",
        "echo stage_query_parse_ms=1.0\r\n",
        "echo stage_prefilter_ms=2.0\r\n",
        "echo stage_bm25_ms=3.0\r\n",
        "echo stage_ann_ms=4.0\r\n",
        "echo stage_fusion_ms=5.0\r\n",
        "echo stage_bulk_hydrate_ms=6.0\r\n",
        "echo stage_snippet_ms=7.0\r\n",
        "echo rss_delta_mb=0.0\r\n",
        "echo elapsed_ms=8.0\r\n",
        "echo hits=%RESUME_IR_QUERY_TOP_K%\r\n",
        "echo resume-ir-query-end\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn bucket_stage_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  set \"request_id=private-query-request-!request_index!\"\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY_SINGLE\" >nul\r\n",
        "  if errorlevel 1 (\r\n",
        "    echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY_AND\" >nul\r\n",
        "    if errorlevel 1 (\r\n",
        "      set \"parse=1.0\" & set \"prefilter=1.0\" & set \"bm25=1.0\" & set \"ann=1.0\" & set \"fusion=1.0\" & set \"hydrate=1.0\" & set \"snippet=1.0\" & set \"elapsed=1.0\" & set \"hits=0\"\r\n",
        "    ) else (\r\n",
        "      set \"parse=21.0\" & set \"prefilter=22.0\" & set \"bm25=23.0\" & set \"ann=24.0\" & set \"fusion=25.0\" & set \"hydrate=26.0\" & set \"snippet=27.0\" & set \"elapsed=28.0\" & set \"hits=%RESUME_IR_QUERY_TOP_K%\"\r\n",
        "    )\r\n",
        "  ) else (\r\n",
        "    set \"parse=1.0\" & set \"prefilter=2.0\" & set \"bm25=3.0\" & set \"ann=4.0\" & set \"fusion=5.0\" & set \"hydrate=6.0\" & set \"snippet=7.0\" & set \"elapsed=8.0\" & set \"hits=%RESUME_IR_QUERY_TOP_K%\"\r\n",
        "  )\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=!request_id!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo stage_query_parse_ms=!parse!\r\n",
        "  echo stage_prefilter_ms=!prefilter!\r\n",
        "  echo stage_bm25_ms=!bm25!\r\n",
        "  echo stage_ann_ms=!ann!\r\n",
        "  echo stage_fusion_ms=!fusion!\r\n",
        "  echo stage_bulk_hydrate_ms=!hydrate!\r\n",
        "  echo stage_snippet_ms=!snippet!\r\n",
        "  echo rss_delta_mb=0.0\r\n",
        "  echo elapsed_ms=!elapsed!\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn rss_delta_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a rss=1\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  set \"request_id=private-query-request-!request_index!\"\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=!request_id!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo stage_query_parse_ms=1.0\r\n",
        "  echo stage_prefilter_ms=2.0\r\n",
        "  echo stage_bm25_ms=3.0\r\n",
        "  echo stage_ann_ms=4.0\r\n",
        "  echo stage_fusion_ms=5.0\r\n",
        "  echo stage_bulk_hydrate_ms=6.0\r\n",
        "  echo stage_snippet_ms=7.0\r\n",
        "  echo rss_delta_mb=!rss!\r\n",
        "  echo elapsed_ms=8.0\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a rss*=2\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn resident_batch_invocation_count_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "set \"counter=%~1\"\r\n",
        "if \"%counter%\"==\"\" exit /b 41\r\n",
        "set /a count=0\r\n",
        "if exist \"%counter%\" set /p count=<\"%counter%\"\r\n",
        "set /a count+=1\r\n",
        "echo !count!>\"%counter%\"\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  set \"request_id=private-query-request-!request_index!\"\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=!request_id!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo stage_query_parse_ms=1.0\r\n",
        "  echo stage_prefilter_ms=2.0\r\n",
        "  echo stage_bm25_ms=3.0\r\n",
        "  echo stage_ann_ms=4.0\r\n",
        "  echo stage_fusion_ms=5.0\r\n",
        "  echo stage_bulk_hydrate_ms=6.0\r\n",
        "  echo stage_snippet_ms=7.0\r\n",
        "  echo rss_delta_mb=0.0\r\n",
        "  echo elapsed_ms=8.0\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn elapsed_ms_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a elapsed=1\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  set \"request_id=private-query-request-!request_index!\"\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=!request_id!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo stage_query_parse_ms=1.0\r\n",
        "  echo stage_prefilter_ms=2.0\r\n",
        "  echo stage_bm25_ms=3.0\r\n",
        "  echo stage_ann_ms=4.0\r\n",
        "  echo stage_fusion_ms=5.0\r\n",
        "  echo stage_bulk_hydrate_ms=6.0\r\n",
        "  echo stage_snippet_ms=7.0\r\n",
        "  echo rss_delta_mb=0.0\r\n",
        "  echo elapsed_ms=!elapsed!\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a elapsed*=4\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn out_of_order_request_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "echo resume-ir-query-v2\r\n",
        "echo request_id=private-query-request-2\r\n",
        "echo mode=hybrid\r\n",
        "echo layers=fulltext+field+vector+rrf\r\n",
        "echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "echo query_embedding_runtime=local-command\r\n",
        "echo query_embedding_invocations=1\r\n",
        "echo stage_query_parse_ms=1.0\r\n",
        "echo stage_prefilter_ms=2.0\r\n",
        "echo stage_bm25_ms=3.0\r\n",
        "echo stage_ann_ms=4.0\r\n",
        "echo stage_fusion_ms=5.0\r\n",
        "echo stage_bulk_hydrate_ms=6.0\r\n",
        "echo stage_snippet_ms=7.0\r\n",
        "echo rss_delta_mb=0.0\r\n",
        "echo elapsed_ms=44.0\r\n",
        "echo hits=2\r\n",
        "echo resume-ir-query-end\r\n",
        "echo resume-ir-query-v2\r\n",
        "echo request_id=private-query-request-1\r\n",
        "echo mode=hybrid\r\n",
        "echo layers=fulltext+field+vector+rrf\r\n",
        "echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "echo query_embedding_runtime=local-command\r\n",
        "echo query_embedding_invocations=1\r\n",
        "echo stage_query_parse_ms=1.0\r\n",
        "echo stage_prefilter_ms=2.0\r\n",
        "echo stage_bm25_ms=3.0\r\n",
        "echo stage_ann_ms=4.0\r\n",
        "echo stage_fusion_ms=5.0\r\n",
        "echo stage_bulk_hydrate_ms=6.0\r\n",
        "echo stage_snippet_ms=7.0\r\n",
        "echo rss_delta_mb=0.0\r\n",
        "echo elapsed_ms=11.0\r\n",
        "echo hits=1\r\n",
        "echo resume-ir-query-end\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn missing_stage_latency_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  set \"request_id=private-query-request-!request_index!\"\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=!request_id!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo rss_delta_mb=0.0\r\n",
        "  echo elapsed_ms=8.0\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn oversized_stdout_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "powershell -NoProfile -Command \"$s='x'*9437184; [Console]::Out.Write($s)\"\r\n",
    )
}

#[cfg(windows)]
fn missing_elapsed_ms_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  set \"request_id=private-query-request-!request_index!\"\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=!request_id!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo query_embedding_runtime=local-command\r\n",
        "  echo query_embedding_invocations=1\r\n",
        "  echo stage_query_parse_ms=1.0\r\n",
        "  echo stage_prefilter_ms=2.0\r\n",
        "  echo stage_bm25_ms=3.0\r\n",
        "  echo stage_ann_ms=4.0\r\n",
        "  echo stage_fusion_ms=5.0\r\n",
        "  echo stage_bulk_hydrate_ms=6.0\r\n",
        "  echo stage_snippet_ms=7.0\r\n",
        "  echo rss_delta_mb=0.0\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn missing_top_k_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  set \"request_id=private-query-request-!request_index!\"\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=!request_id!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo elapsed_ms=8.0\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn mismatched_top_k_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "set /a request_index=1\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  set \"request_id=private-query-request-!request_index!\"\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=5\")\r\n",
        "  echo resume-ir-query-v2\r\n",
        "  echo request_id=!request_id!\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=5\r\n",
        "  echo elapsed_ms=8.0\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
        "  set /a request_index+=1\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

#[cfg(windows)]
fn query_embedding_attestation_query_fixture_script_body() -> &'static str {
    query_fixture_script_body()
}

#[cfg(windows)]
fn legacy_query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "setlocal enabledelayedexpansion\r\n",
        "if \"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\"==\"\" exit /b 42\r\n",
        "for /f \"usebackq delims=\" %%L in (\"%RESUME_IR_QUERY_BATCH_INPUT_PATH%\") do (\r\n",
        "  echo %%L | findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" >nul\r\n",
        "  if errorlevel 1 (set \"hits=0\") else (set \"hits=%RESUME_IR_QUERY_TOP_K%\")\r\n",
        "  echo resume-ir-query-v1\r\n",
        "  echo hits=!hits!\r\n",
        "  echo resume-ir-query-end\r\n",
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
