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
    assert!(stdout.contains("\"generation_mode\":\"streaming\""));
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

#[test]
fn resume_benchmark_gate_accepts_explicit_synthetic_smoke_report() {
    let index_dir = temp_dir("synthetic-query-cli-gate-index");
    let report_path = temp_dir("synthetic-query-cli-gate-report").join("report.json");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "synthetic-query",
            "--index-dir",
            path_str(&index_dir),
            "--documents",
            "24",
            "--queries",
            "100",
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
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "gate",
            "--report",
            path_str(&report_path),
            "--allow-synthetic",
            "--min-documents",
            "24",
            "--min-queries",
            "100",
            "--max-p95-ms",
            "1000",
            "--max-zero-result-queries",
            "0",
        ])
        .output()
        .expect("run resume-benchmark gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "benchmark gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(&index_dir);
    remove_dir(report_path.parent().unwrap());
}

#[test]
fn resume_benchmark_gate_rejects_synthetic_without_explicit_allowance() {
    let report_dir = temp_dir("synthetic-query-cli-gate-reject");
    let report_path = report_dir.join("report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"benchmark.v1\",",
            "\"dataset_kind\":\"synthetic\",",
            "\"document_count\":1000,",
            "\"query_count\":100,",
            "\"query_latency_ms\":{\"samples\":100,\"p95\":10},",
            "\"zero_result_queries\":0,",
            "\"million_scale_verified\":false,",
            "\"target_claim\":\"not_evaluated\"}"
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "gate",
            "--report",
            path_str(&report_path),
            "--min-documents",
            "1000",
            "--min-queries",
            "100",
            "--max-p95-ms",
            "50",
        ])
        .output()
        .expect("run resume-benchmark gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("synthetic benchmark requires explicit allowance"));

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_private_query_outputs_redacted_gateable_report() {
    let query_set = private_query_set_file("private-query-cli-set", 500);
    let command = query_fixture_script("private-query-cli-command");
    let corpus_summary =
        private_query_corpus_summary_file("private-query-cli-summary", 8_720, true);
    let report_dir = temp_dir("private-query-cli-report");
    let report_path = report_dir.join("private-query-report.json");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "private-query",
            "--query-set",
            path_str(&query_set),
            "--command",
            path_str(&command),
            "--corpus-summary",
            path_str(&corpus_summary),
            "--max-queries",
            "500",
            "--top-k",
            "10",
            "--timeout-ms",
            "5000",
            "--index-size-bytes",
            "4096",
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--query-set-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "--model-manifest-sha256",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--json",
        ])
        .output()
        .expect("run resume-benchmark private-query");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"benchmark.v1\""));
    assert!(stdout.contains("\"dataset_kind\":\"private-real-corpus\""));
    assert!(stdout.contains("\"document_count\":8720"));
    assert!(stdout.contains("\"searchable_document_count\":8720"));
    assert!(stdout.contains("\"vector_indexed_document_count\":8720"));
    assert!(stdout.contains("\"corpus_summary_sha256\":\""));
    assert!(stdout.contains("\"query_count\":500"));
    assert!(stdout.contains(
        "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\""
    ));
    assert!(stdout.contains("\"target_claim\":\"benchmark_baseline_observed\""));
    assert!(stdout.contains("\"query_protocol\":\"resume-ir-query-v1\""));
    assert!(stdout.contains("\"query_mode\":\"hybrid\""));
    assert!(stdout.contains("\"retrieval_layers\":\"fulltext+field+vector+rrf\""));
    assert!(stdout.contains("\"hot_index\":true"));
    assert!(stdout.contains("\"contains_raw_resume_text\":false"));
    assert!(stdout.contains("\"contains_resume_paths\":false"));
    assert!(stdout.contains("\"contains_queries\":false"));
    assert!(!stdout.contains(path_str(&query_set)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains(path_str(&corpus_summary)));
    assert!(!stdout.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));
    assert!(!stdout.contains("private-query-sample-"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--min-documents",
            "8000",
            "--min-queries",
            "500",
            "--max-p95-ms",
            "10000",
        ])
        .output()
        .expect("run resume-benchmark gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "benchmark gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_private_query_rejects_partial_corpus_summary_without_path_leaks() {
    let query_set = private_query_set_file("private-query-cli-partial-set", 1);
    let command = query_fixture_script("private-query-cli-partial-command");
    let corpus_summary =
        private_query_corpus_summary_file("private-query-cli-partial-summary", 8_720, false);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "private-query",
            "--query-set",
            path_str(&query_set),
            "--command",
            path_str(&command),
            "--corpus-summary",
            path_str(&corpus_summary),
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--query-set-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "--model-manifest-sha256",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--json",
        ])
        .output()
        .expect("run resume-benchmark private-query with partial corpus summary");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("private_query_corpus_summary_hot_index"));
    assert!(!stderr.contains(path_str(&corpus_summary)));
    assert!(!stderr.contains(path_str(&query_set)));
    assert!(!stderr.contains(path_str(&command)));
}

#[test]
fn resume_benchmark_private_query_accepts_partial_corpus_summary_for_explicit_smoke() {
    let query_set = private_query_set_file("private-query-cli-partial-smoke-set", 1);
    let command = query_fixture_script("private-query-cli-partial-smoke-command");
    let corpus_summary =
        private_query_corpus_summary_file("private-query-cli-partial-smoke-summary", 6, false);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "private-query",
            "--query-set",
            path_str(&query_set),
            "--command",
            path_str(&command),
            "--corpus-summary",
            path_str(&corpus_summary),
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--query-set-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "--model-manifest-sha256",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--max-queries",
            "1",
            "--allow-partial-hot-index-for-smoke",
            "--json",
        ])
        .output()
        .expect("run resume-benchmark private-query with explicit partial smoke policy");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"document_count\":6"));
    assert!(stdout.contains("\"searchable_document_count\":5"));
    assert!(stdout.contains("\"vector_indexed_document_count\":4"));
    assert!(stdout.contains("\"percentile_confidence\":\"smoke\""));
    assert!(stdout.contains("\"privacy_boundary\":\"redacted_local_aggregate\""));
    assert!(!stdout.contains(path_str(&corpus_summary)));
    assert!(!stdout.contains(path_str(&query_set)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
    remove_dir(corpus_summary.parent().unwrap());
}

#[test]
fn resume_benchmark_private_query_passes_command_args_without_leaking_them() {
    let query_set = private_query_set_file("private-query-cli-command-args-set", 3);
    let command = query_fixture_script_requiring_args("private-query-cli-command-args-command");
    let corpus_summary =
        private_query_corpus_summary_file("private-query-cli-command-args-summary", 8_720, true);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "private-query",
            "--query-set",
            path_str(&query_set),
            "--command",
            path_str(&command),
            "--command-arg",
            "resume-cli",
            "--command-arg",
            "benchmark-query-protocol",
            "--corpus-summary",
            path_str(&corpus_summary),
            "--max-queries",
            "3",
            "--top-k",
            "10",
            "--timeout-ms",
            "5000",
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--query-set-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "--model-manifest-sha256",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--json",
        ])
        .output()
        .expect("run resume-benchmark private-query with command args");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"benchmark.v1\""));
    assert!(stdout.contains("\"query_count\":3"));
    assert!(stdout.contains("\"query_mode\":\"hybrid\""));
    assert!(!stdout.contains(path_str(&query_set)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains(path_str(&corpus_summary)));
    assert!(!stdout.contains("resume-cli"));
    assert!(!stdout.contains("benchmark-query-protocol"));
    assert!(!stdout.contains("REDACTION_SENTINEL_PRIVATE_QUERY"));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
    remove_dir(corpus_summary.parent().unwrap());
}

#[test]
fn resume_benchmark_private_query_requires_model_manifest_digest() {
    let query_set = private_query_set_file("private-query-cli-missing-model-set", 1);
    let command = query_fixture_script("private-query-cli-missing-model-command");
    let corpus_summary =
        private_query_corpus_summary_file("private-query-cli-missing-model-summary", 8_720, true);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "private-query",
            "--query-set",
            path_str(&query_set),
            "--command",
            path_str(&command),
            "--corpus-summary",
            path_str(&corpus_summary),
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--query-set-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        ])
        .output()
        .expect("run resume-benchmark private-query without model manifest digest");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("usage: resume-benchmark"));
    assert!(!stderr.contains(path_str(&query_set)));
    assert!(!stderr.contains(path_str(&command)));
    assert!(!stderr.contains(path_str(&corpus_summary)));

    remove_dir(query_set.parent().unwrap());
    remove_dir(command.parent().unwrap());
    remove_dir(corpus_summary.parent().unwrap());
}

#[test]
fn resume_benchmark_gate_accepts_private_real_corpus_release_report() {
    let report_dir = temp_dir("private-real-benchmark-cli-gate");
    let report_path = report_dir.join("benchmark-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"benchmark.v1\",",
            "\"run_id\":\"bench_private\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"private-real-corpus\",",
            "\"document_count\":1000000,",
            "\"searchable_document_count\":1000000,",
            "\"vector_indexed_document_count\":1000000,",
            "\"query_count\":500,",
            "\"top_k\":10,",
            "\"build_ms\":1.0,",
            "\"query_total_ms\":5000.0,",
            "\"qps\":100.0,",
            "\"index_size_bytes\":1000,",
            "\"query_latency_ms\":{",
            "\"samples\":500,",
            "\"min\":1.0,",
            "\"mean\":2.0,",
            "\"p50\":2.0,",
            "\"p95\":150.0,",
            "\"p99\":180.0,",
            "\"max\":190.0",
            "},",
            "\"zero_result_queries\":0,",
            "\"total_hits\":5000,",
            "\"million_scale_verified\":true,",
            "\"percentile_confidence\":\"release\",",
            "\"target_claim\":\"query_latency_target_met\",",
            "\"corpus_origin\":\"private_local\",",
            "\"privacy_boundary\":\"redacted_local_aggregate\",",
            "\"query_protocol\":\"resume-ir-query-v1\",",
            "\"query_mode\":\"hybrid\",",
            "\"retrieval_layers\":\"fulltext+field+vector+rrf\",",
            "\"hot_index\":true,",
            "\"hot_path_ocr\":false,",
            "\"hot_path_parsing\":false,",
            "\"hot_path_heavy_model_inference\":false,",
            "\"contains_raw_resume_text\":false,",
            "\"contains_resume_paths\":false,",
            "\"contains_queries\":false,",
            "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
            "\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
            "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"corpus_summary_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\"",
            "}"
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--require-million-scale",
            "--min-documents",
            "1000000",
            "--min-queries",
            "500",
            "--max-p95-ms",
            "200",
        ])
        .output()
        .expect("run resume-benchmark gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "benchmark gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_gate_accepts_private_real_smoke_report_with_explicit_allowance() {
    let report_dir = temp_dir("private-real-benchmark-cli-smoke-gate");
    let report_path = report_dir.join("benchmark-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"benchmark.v1\",",
            "\"run_id\":\"bench_private_smoke\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"private-real-corpus\",",
            "\"document_count\":1,",
            "\"searchable_document_count\":1,",
            "\"vector_indexed_document_count\":1,",
            "\"query_count\":1,",
            "\"top_k\":10,",
            "\"build_ms\":1.0,",
            "\"query_total_ms\":10.0,",
            "\"qps\":100.0,",
            "\"index_size_bytes\":1000,",
            "\"query_latency_ms\":{",
            "\"samples\":1,",
            "\"min\":1.0,",
            "\"mean\":2.0,",
            "\"p50\":2.0,",
            "\"p95\":150.0,",
            "\"p99\":180.0,",
            "\"max\":190.0",
            "},",
            "\"zero_result_queries\":0,",
            "\"total_hits\":1,",
            "\"million_scale_verified\":false,",
            "\"percentile_confidence\":\"smoke\",",
            "\"target_claim\":\"benchmark_baseline_observed\",",
            "\"corpus_origin\":\"private_local\",",
            "\"privacy_boundary\":\"redacted_local_aggregate\",",
            "\"query_protocol\":\"resume-ir-query-v1\",",
            "\"query_mode\":\"hybrid\",",
            "\"retrieval_layers\":\"fulltext+field+vector+rrf\",",
            "\"hot_index\":true,",
            "\"hot_path_ocr\":false,",
            "\"hot_path_parsing\":false,",
            "\"hot_path_heavy_model_inference\":false,",
            "\"contains_raw_resume_text\":false,",
            "\"contains_resume_paths\":false,",
            "\"contains_queries\":false,",
            "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
            "\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
            "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"corpus_summary_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\"",
            "}"
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--allow-smoke-confidence",
            "--min-documents",
            "1",
            "--min-queries",
            "1",
            "--max-p95-ms",
            "10000",
        ])
        .output()
        .expect("run resume-benchmark smoke gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "benchmark gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_gate_rejects_private_real_corpus_inconsistent_qps() {
    let report_dir = temp_dir("private-real-benchmark-cli-inconsistent-qps");
    let report_path = report_dir.join("benchmark-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"benchmark.v1\",",
            "\"run_id\":\"bench_private\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"private-real-corpus\",",
            "\"document_count\":1000000,",
            "\"searchable_document_count\":1000000,",
            "\"vector_indexed_document_count\":1000000,",
            "\"query_count\":500,",
            "\"top_k\":10,",
            "\"build_ms\":1.0,",
            "\"query_total_ms\":5000.0,",
            "\"qps\":999.0,",
            "\"index_size_bytes\":1000,",
            "\"query_latency_ms\":{",
            "\"samples\":500,",
            "\"min\":1.0,",
            "\"mean\":2.0,",
            "\"p50\":2.0,",
            "\"p95\":150.0,",
            "\"p99\":180.0,",
            "\"max\":190.0",
            "},",
            "\"zero_result_queries\":0,",
            "\"total_hits\":5000,",
            "\"million_scale_verified\":true,",
            "\"percentile_confidence\":\"release\",",
            "\"target_claim\":\"query_latency_target_met\",",
            "\"corpus_origin\":\"private_local\",",
            "\"privacy_boundary\":\"redacted_local_aggregate\",",
            "\"query_protocol\":\"resume-ir-query-v1\",",
            "\"query_mode\":\"hybrid\",",
            "\"retrieval_layers\":\"fulltext+field+vector+rrf\",",
            "\"hot_index\":true,",
            "\"hot_path_ocr\":false,",
            "\"hot_path_parsing\":false,",
            "\"hot_path_heavy_model_inference\":false,",
            "\"contains_raw_resume_text\":false,",
            "\"contains_resume_paths\":false,",
            "\"contains_queries\":false,",
            "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
            "\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
            "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"corpus_summary_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\"",
            "}"
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--require-million-scale",
            "--min-documents",
            "1000000",
            "--min-queries",
            "500",
            "--max-p95-ms",
            "200",
        ])
        .output()
        .expect("run resume-benchmark gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private real-corpus benchmark metric counts do not match scores"));

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_gate_rejects_million_release_sampled_confidence() {
    let report_dir = temp_dir("private-real-benchmark-cli-sampled-confidence");
    let report_path = report_dir.join("benchmark-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"benchmark.v1\",",
            "\"run_id\":\"bench_private\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"private-real-corpus\",",
            "\"document_count\":1000000,",
            "\"searchable_document_count\":1000000,",
            "\"vector_indexed_document_count\":1000000,",
            "\"query_count\":500,",
            "\"top_k\":10,",
            "\"build_ms\":1.0,",
            "\"query_total_ms\":5000.0,",
            "\"qps\":100.0,",
            "\"index_size_bytes\":1000,",
            "\"query_latency_ms\":{",
            "\"samples\":500,",
            "\"min\":1.0,",
            "\"mean\":2.0,",
            "\"p50\":2.0,",
            "\"p95\":150.0,",
            "\"p99\":180.0,",
            "\"max\":190.0",
            "},",
            "\"zero_result_queries\":0,",
            "\"total_hits\":5000,",
            "\"million_scale_verified\":true,",
            "\"percentile_confidence\":\"sampled\",",
            "\"target_claim\":\"query_latency_target_met\",",
            "\"corpus_origin\":\"private_local\",",
            "\"privacy_boundary\":\"redacted_local_aggregate\",",
            "\"query_protocol\":\"resume-ir-query-v1\",",
            "\"query_mode\":\"hybrid\",",
            "\"retrieval_layers\":\"fulltext+field+vector+rrf\",",
            "\"hot_index\":true,",
            "\"hot_path_ocr\":false,",
            "\"hot_path_parsing\":false,",
            "\"hot_path_heavy_model_inference\":false,",
            "\"contains_raw_resume_text\":false,",
            "\"contains_resume_paths\":false,",
            "\"contains_queries\":false,",
            "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
            "\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
            "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"corpus_summary_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\"",
            "}"
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--require-million-scale",
            "--min-documents",
            "1000000",
            "--min-queries",
            "500",
            "--max-p95-ms",
            "200",
        ])
        .output()
        .expect("run resume-benchmark gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("million-scale release benchmark requires release confidence"));

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_gate_rejects_private_real_too_few_query_samples() {
    let report_dir = temp_dir("private-real-benchmark-cli-too-few-queries");
    let report_path = report_dir.join("benchmark-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"benchmark.v1\",",
            "\"run_id\":\"bench_private\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"private-real-corpus\",",
            "\"document_count\":100000,",
            "\"searchable_document_count\":100000,",
            "\"vector_indexed_document_count\":100000,",
            "\"query_count\":200,",
            "\"top_k\":10,",
            "\"build_ms\":1.0,",
            "\"query_total_ms\":2000.0,",
            "\"qps\":100.0,",
            "\"index_size_bytes\":1000,",
            "\"query_latency_ms\":{",
            "\"samples\":200,",
            "\"min\":1.0,",
            "\"mean\":2.0,",
            "\"p50\":2.0,",
            "\"p95\":150.0,",
            "\"p99\":180.0,",
            "\"max\":190.0",
            "},",
            "\"zero_result_queries\":0,",
            "\"total_hits\":2000,",
            "\"million_scale_verified\":false,",
            "\"percentile_confidence\":\"release\",",
            "\"target_claim\":\"query_latency_target_met\",",
            "\"corpus_origin\":\"private_local\",",
            "\"privacy_boundary\":\"redacted_local_aggregate\",",
            "\"query_protocol\":\"resume-ir-query-v1\",",
            "\"query_mode\":\"hybrid\",",
            "\"retrieval_layers\":\"fulltext+field+vector+rrf\",",
            "\"hot_index\":true,",
            "\"hot_path_ocr\":false,",
            "\"hot_path_parsing\":false,",
            "\"hot_path_heavy_model_inference\":false,",
            "\"contains_raw_resume_text\":false,",
            "\"contains_resume_paths\":false,",
            "\"contains_queries\":false,",
            "\"dataset_manifest_sha256\":\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\",",
            "\"query_set_sha256\":\"abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\",",
            "\"model_manifest_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"corpus_summary_sha256\":\"1111111111111111111111111111111111111111111111111111111111111111\",",
            "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\"",
            "}"
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--min-documents",
            "100000",
            "--min-queries",
            "100",
            "--max-p95-ms",
            "200",
        ])
        .output()
        .expect("run resume-benchmark gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private real-corpus benchmark requires release query sample count"));

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_field_quality_outputs_redacted_report_and_gate() {
    let dataset_dir = temp_dir("field-quality-dataset");
    let dataset_path = dataset_dir.join("field-quality.jsonl");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(
        &dataset_path,
        concat!(
            "{\"sample_id\":\"private-case-1\",\"text\":\"Name: Synthetic Candidate\\nEmail: candidate@example.test\\nPhone: (415) 555-0132\",",
            "\"expected\":[",
            "{\"type\":\"name\",\"normalized\":\"synthetic candidate\"},",
            "{\"type\":\"email\",\"normalized\":\"candidate@example.test\"},",
            "{\"type\":\"phone\",\"normalized\":\"+14155550132\"}",
            "]}\n",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-quality",
            "--dataset",
            path_str(&dataset_path),
            "--json",
        ])
        .output()
        .expect("run field-quality benchmark");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"field-quality.v1\""));
    assert!(stdout.contains("\"dataset_kind\":\"labeled\""));
    assert!(stdout.contains("\"sample_count\":1"));
    assert!(stdout.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!stdout.contains(path_str(&dataset_path)));
    assert!(!stdout.contains("private-case-1"));
    assert!(!stdout.contains("Synthetic Candidate"));
    assert!(!stdout.contains("candidate@example.test"));
    assert!(!stdout.contains("+14155550132"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--min-samples",
            "1",
            "--min-precision",
            "0.99",
            "--min-recall",
            "0.99",
            "--min-f1",
            "0.99",
        ])
        .output()
        .expect("run field quality gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "field quality gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_private_business_field_quality_outputs_redacted_gateable_report() {
    let dataset_dir = temp_dir("private-business-field-quality-dataset");
    let dataset_path = dataset_dir.join("private-field-quality.jsonl");
    let report_path = dataset_dir.join("private-field-report.json");
    fs::write(&dataset_path, private_business_field_quality_dataset()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-quality",
            "--dataset",
            path_str(&dataset_path),
            "--private-business-labeled",
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--annotation-manifest-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "--json",
        ])
        .output()
        .expect("run private business field-quality benchmark");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"field-quality.v1\""));
    assert!(stdout.contains("\"dataset_kind\":\"private-business-labeled\""));
    assert!(stdout.contains("\"sample_count\":1"));
    assert!(stdout.contains("\"target_claim\":\"field_quality_target_met\""));
    assert!(stdout.contains("\"corpus_origin\":\"private_local\""));
    assert!(stdout.contains("\"privacy_boundary\":\"redacted_local_aggregate\""));
    assert!(stdout.contains("\"contains_raw_resume_text\":false"));
    assert!(stdout.contains("\"contains_resume_paths\":false"));
    assert!(stdout.contains("\"contains_field_values\":false"));
    assert!(stdout.contains("\"contains_sample_ids\":false"));
    assert!(stdout.contains("\"field_taxonomy\":\"resume-ir.fields.v1\""));
    assert!(!stdout.contains(path_str(&dataset_path)));
    assert!(!stdout.contains("private-field-sample-001"));
    assert!(!stdout.contains("REDACTION_SENTINEL_FIELD_VALUE"));
    assert!(!stdout.contains("Synthetic Field Candidate"));
    assert!(!stdout.contains("field-candidate@example.test"));
    assert!(!stdout.contains("Candidate_2026"));
    assert!(!stdout.contains("candidate_2026"));
    assert!(!stdout.contains("Synthetic Commerce"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "field quality gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_field_gate_requires_private_business_labeled_flag() {
    let dataset_dir = temp_dir("field-quality-private-business-reject");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(
        &report_path,
        concat!(
            "{",
            "\"schema_version\":\"field-quality.v1\",",
            "\"run_id\":\"fieldq_test\",",
            "\"platform\":\"test/test\",",
            "\"dataset_kind\":\"labeled\",",
            "\"sample_count\":1000,",
            "\"expected_mentions\":1000,",
            "\"predicted_mentions\":1000,",
            "\"overall\":{\"true_positive\":1000,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
            "\"fields\":{\"email\":{\"true_positive\":1000,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}},",
            "\"target_claim\":\"not_evaluated\",",
            "\"scope\":\"labeled field extraction quality; no raw resume text, paths, sample ids, or field values included\"",
            "}"
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1000",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private business field-quality benchmark required"));
    assert!(!String::from_utf8_lossy(&gate.stderr).contains(path_str(&report_path)));

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_field_gate_accepts_private_business_labeled_report() {
    let dataset_dir = temp_dir("field-quality-private-business-accept");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(&report_path, minimal_private_business_field_quality_json()).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1000",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "field quality gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_field_gate_requires_private_business_school_tier_metric() {
    let dataset_dir = temp_dir("field-quality-private-business-school-tier");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(
        &report_path,
        minimal_private_business_field_quality_json().replace(
            ",\"school_tier\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
            "",
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1000",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private business field quality requires production field metrics"));
    assert!(!String::from_utf8_lossy(&gate.stderr).contains(path_str(&report_path)));

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_field_gate_requires_private_business_name_metric() {
    let dataset_dir = temp_dir("field-quality-private-business-name");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(
        &report_path,
        minimal_private_business_field_quality_json().replace(
            "\"name\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
            "",
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1000",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private business field quality requires production field metrics"));
    assert!(!String::from_utf8_lossy(&gate.stderr).contains(path_str(&report_path)));

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_field_gate_requires_private_business_field_label_support() {
    let dataset_dir = temp_dir("field-quality-private-business-support");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(
        &report_path,
        minimal_private_business_field_quality_json().replace(
            "\"name\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
            "\"name\":{\"true_positive\":0,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0},",
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1000",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private business field quality requires production field support"));
    assert!(!String::from_utf8_lossy(&gate.stderr).contains(path_str(&report_path)));

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_field_gate_requires_private_business_major_metric() {
    let dataset_dir = temp_dir("field-quality-private-business-major");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(
        &report_path,
        minimal_private_business_field_quality_json().replace(
            ",\"major\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
            "",
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1000",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private business field quality requires production field metrics"));
    assert!(!String::from_utf8_lossy(&gate.stderr).contains(path_str(&report_path)));

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_field_gate_requires_private_business_location_metric() {
    let dataset_dir = temp_dir("field-quality-private-business-location");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(
        &report_path,
        minimal_private_business_field_quality_json().replace(
            ",\"location\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
            "",
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1000",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private business field quality requires production field metrics"));
    assert!(!String::from_utf8_lossy(&gate.stderr).contains(path_str(&report_path)));

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_field_gate_requires_private_business_certificate_metric() {
    let dataset_dir = temp_dir("field-quality-private-business-certificate");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(
        &report_path,
        minimal_private_business_field_quality_json().replace(
            ",\"certificate\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
            "",
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1000",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private business field quality requires production field metrics"));
    assert!(!String::from_utf8_lossy(&gate.stderr).contains(path_str(&report_path)));

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_field_gate_requires_private_business_years_experience_metric() {
    let dataset_dir = temp_dir("field-quality-private-business-years-experience");
    let report_path = dataset_dir.join("field-report.json");
    fs::write(
        &report_path,
        minimal_private_business_field_quality_json().replace(
            ",\"years_experience\":{\"true_positive\":125,\"false_positive\":0,\"false_negative\":0,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0}",
            "",
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "field-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "1000",
            "--min-precision",
            "0.93",
            "--min-recall",
            "0.93",
            "--min-f1",
            "0.93",
        ])
        .output()
        .expect("run private business field quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private business field quality requires production field metrics"));
    assert!(!String::from_utf8_lossy(&gate.stderr).contains(path_str(&report_path)));

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_dedupe_quality_outputs_redacted_report_and_gate() {
    let dataset_dir = temp_dir("dedupe-quality-cli-dataset");
    let dataset_path = dataset_dir.join("dedupe-quality.jsonl");
    let report_path = dataset_dir.join("dedupe-report.json");
    fs::write(
        &dataset_path,
        concat!(
            "{\"sample_id\":\"private-dedupe-a\",",
            "\"left\":{\"id\":\"private-left-a\",\"name\":\"Synthetic Candidate\",\"schools\":[\"Synthetic University\"],\"companies\":[\"Example Labs\"],\"skills\":[\"Java\",\"Payments\"]},",
            "\"right\":{\"id\":\"private-right-a\",\"name\":\"synthetic candidate\",\"schools\":[\"synthetic university\"],\"companies\":[\"Example Labs\"],\"skills\":[\"Java\",\"Search\"]},",
            "\"duplicate\":true}\n",
            "{\"sample_id\":\"private-dedupe-b\",",
            "\"left\":{\"id\":\"private-left-b\",\"name\":\"Synthetic Candidate\",\"schools\":[\"Synthetic University\"],\"skills\":[\"Java\"]},",
            "\"right\":{\"id\":\"private-right-b\",\"name\":\"Different Candidate\",\"schools\":[\"Synthetic University\"],\"skills\":[\"Java\"]},",
            "\"duplicate\":false}\n",
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "dedupe-quality",
            "--dataset",
            path_str(&dataset_path),
            "--json",
        ])
        .output()
        .expect("run dedupe quality benchmark");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"dedupe-quality.v1\""));
    assert!(stdout.contains("\"dataset_kind\":\"labeled\""));
    assert!(stdout.contains("\"pair_count\":2"));
    assert!(stdout.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!stdout.contains(path_str(&dataset_path)));
    assert!(!stdout.contains("private-dedupe-a"));
    assert!(!stdout.contains("private-left-a"));
    assert!(!stdout.contains("Synthetic Candidate"));
    assert!(!stdout.contains("Synthetic University"));
    assert!(!stdout.contains("Example Labs"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "dedupe-gate",
            "--report",
            path_str(&report_path),
            "--min-pairs",
            "2",
            "--min-positive-pairs",
            "1",
            "--min-precision",
            "0.99",
            "--min-recall",
            "0.99",
            "--min-f1",
            "0.99",
        ])
        .output()
        .expect("run dedupe quality gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "dedupe quality gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_private_business_dedupe_quality_outputs_redacted_gateable_report() {
    let dataset_dir = temp_dir("private-business-dedupe-quality-dataset");
    let dataset_path = dataset_dir.join("private-dedupe-quality.jsonl");
    let report_path = dataset_dir.join("private-dedupe-report.json");
    fs::write(&dataset_path, private_business_dedupe_quality_dataset()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "dedupe-quality",
            "--dataset",
            path_str(&dataset_path),
            "--private-business-labeled",
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--annotation-manifest-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "--json",
        ])
        .output()
        .expect("run private business dedupe-quality benchmark");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"dedupe-quality.v1\""));
    assert!(stdout.contains("\"dataset_kind\":\"private-business-labeled\""));
    assert!(stdout.contains("\"pair_count\":2"));
    assert!(stdout.contains("\"target_claim\":\"dedupe_quality_target_met\""));
    assert!(stdout.contains("\"corpus_origin\":\"private_local\""));
    assert!(stdout.contains("\"privacy_boundary\":\"redacted_local_aggregate\""));
    assert!(stdout.contains("\"contains_raw_resume_text\":false"));
    assert!(stdout.contains("\"contains_resume_paths\":false"));
    assert!(stdout.contains("\"contains_profile_values\":false"));
    assert!(stdout.contains("\"contains_sample_ids\":false"));
    assert!(stdout.contains("\"contains_document_ids\":false"));
    assert!(stdout.contains("\"dedupe_taxonomy\":\"resume-ir.dedupe.v1\""));
    assert!(!stdout.contains(path_str(&dataset_path)));
    assert!(!stdout.contains("private-dedupe-sample-001"));
    assert!(!stdout.contains("private-left-doc-001"));
    assert!(!stdout.contains("REDACTION_SENTINEL_DEDUPE_VALUE"));
    assert!(!stdout.contains("Synthetic Duplicate Candidate"));
    assert!(!stdout.contains("Synthetic Commerce"));
    assert!(!stdout.contains("Synthetic University"));
    assert!(!stdout.contains("Payments"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "dedupe-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-pairs",
            "2",
            "--min-positive-pairs",
            "1",
            "--min-precision",
            "0.90",
            "--min-recall",
            "0.90",
            "--min-f1",
            "0.90",
        ])
        .output()
        .expect("run private business dedupe quality gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "dedupe quality gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_dedupe_gate_accepts_private_business_labeled_report() {
    let dataset_dir = temp_dir("dedupe-quality-private-business-accept");
    let report_path = dataset_dir.join("dedupe-report.json");
    fs::write(&report_path, minimal_private_business_dedupe_quality_json()).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "dedupe-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-pairs",
            "1000",
            "--min-positive-pairs",
            "100",
            "--min-precision",
            "0.90",
            "--min-recall",
            "0.90",
            "--min-f1",
            "0.90",
        ])
        .output()
        .expect("run private business dedupe quality gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "dedupe quality gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_dedupe_gate_rejects_private_business_inconsistent_counts() {
    let dataset_dir = temp_dir("dedupe-quality-private-business-inconsistent");
    let report_path = dataset_dir.join("dedupe-report.json");
    fs::write(
        &report_path,
        minimal_private_business_dedupe_quality_json().replace(
            "\"true_positive\":100,\"false_positive\":0,\"false_negative\":0,\"true_negative\":900,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0",
            "\"true_positive\":50,\"false_positive\":50,\"false_negative\":50,\"true_negative\":850,\"precision\":1.0,\"recall\":1.0,\"f1\":1.0",
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "dedupe-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-pairs",
            "1000",
            "--min-positive-pairs",
            "100",
            "--min-precision",
            "0.90",
            "--min-recall",
            "0.90",
            "--min-f1",
            "0.90",
        ])
        .output()
        .expect("run private business dedupe quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private business dedupe quality metric counts do not match scores"));
    assert!(!String::from_utf8_lossy(&gate.stderr).contains(path_str(&report_path)));

    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_ocr_throughput_outputs_redacted_report_and_gate() {
    let command = ocr_fixture_script("ocr-throughput-cli-private-command");
    let report_dir = temp_dir("ocr-throughput-cli-report");
    let report_path = report_dir.join("ocr-report.json");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "ocr-throughput",
            "--command",
            path_str(&command),
            "--pages",
            "3",
            "--page-timeout-ms",
            "5000",
            "--json",
        ])
        .output()
        .expect("run OCR throughput benchmark");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"ocr-throughput.v1\""));
    assert!(stdout.contains("\"dataset_kind\":\"synthetic\""));
    assert!(stdout.contains("\"engine_kind\":\"local-command\""));
    assert!(stdout.contains("\"page_count\":3"));
    assert!(stdout.contains("\"pages_per_second\":"));
    assert!(stdout.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains("Synthetic OCR Candidate"));
    assert!(!stdout.contains("REDACTION_SENTINEL_OCR_TEXT"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "ocr-gate",
            "--report",
            path_str(&report_path),
            "--allow-synthetic",
            "--min-pages",
            "3",
            "--max-p95-ms",
            "5000",
            "--min-pages-per-second",
            "0.001",
        ])
        .output()
        .expect("run OCR throughput gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "OCR throughput gate passed"
    );
    assert!(gate.stderr.is_empty());

    let _ = fs::remove_file(&command);
    remove_dir(command.parent().unwrap());
    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_ocr_gate_rejects_synthetic_without_explicit_allowance() {
    let report_dir = temp_dir("ocr-throughput-cli-gate-reject");
    let report_path = report_dir.join("ocr-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"ocr-throughput.v1\",",
            "\"dataset_kind\":\"synthetic\",",
            "\"page_count\":10,",
            "\"pages_per_second\":5.0,",
            "\"page_latency_ms\":{\"samples\":10,\"p95\":10},",
            "\"target_claim\":\"not_evaluated\"}"
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "ocr-gate",
            "--report",
            path_str(&report_path),
            "--min-pages",
            "10",
            "--max-p95-ms",
            "50",
            "--min-pages-per-second",
            "1",
        ])
        .output()
        .expect("run OCR throughput gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("synthetic OCR benchmark requires explicit allowance"));

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_private_ocr_throughput_outputs_redacted_diagnostic_report() {
    let root = temp_dir("private-ocr-throughput-cli-root");
    fs::write(
        root.join("private-cli-sample.pdf"),
        b"%PDF synthetic private cli sample",
    )
    .unwrap();
    fs::write(
        root.join("ignored-private-cli-sample.docx"),
        b"ignored synthetic docx",
    )
    .unwrap();
    let renderer = pdf_render_fixture_script("private-ocr-throughput-cli-renderer");
    let ocr = ocr_fixture_script("private-ocr-throughput-cli-ocr");
    let report_dir = temp_dir("private-ocr-throughput-cli-report");
    let report_path = report_dir.join("ocr-report.json");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "private-ocr-throughput",
            "--root",
            path_str(&root),
            "--renderer-command",
            path_str(&renderer),
            "--command",
            path_str(&ocr),
            "--max-documents",
            "1",
            "--max-pages",
            "2",
            "--pages-per-document",
            "2",
            "--page-timeout-ms",
            "5000",
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--ocr-runtime-manifest-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "--renderer-manifest-sha256",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--language-pack-manifest-sha256",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "--json",
        ])
        .output()
        .expect("run private OCR throughput benchmark");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"ocr-throughput.v1\""));
    assert!(stdout.contains("\"dataset_kind\":\"private-real-corpus\""));
    assert!(stdout.contains("\"engine_kind\":\"local-command\""));
    assert!(stdout.contains("\"page_count\":2"));
    assert!(stdout.contains("\"document_count\":1"));
    assert!(stdout.contains("\"scanned_document_count\":1"));
    assert!(stdout.contains("\"failed_document_count\":0"));
    assert!(stdout.contains("\"render_failure_count\":0"));
    assert!(stdout.contains("\"ocr_failure_count\":0"));
    assert!(stdout.contains("\"run_budget_exhausted\":false"));
    assert!(stdout.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!stdout.contains("\"target_claim\":\"ocr_throughput_target_met\""));
    assert!(stdout.contains("\"contains_raw_ocr_text\":false"));
    assert!(stdout.contains("\"contains_resume_paths\":false"));
    assert!(stdout.contains("\"contains_command_paths\":false"));
    assert!(stdout.contains("\"scope\":\"private real-corpus OCR throughput benchmark; aggregate redacted report only\""));
    assert!(!stdout.contains(path_str(&root)));
    assert!(!stdout.contains(path_str(&renderer)));
    assert!(!stdout.contains(path_str(&ocr)));
    assert!(!stdout.contains("private-cli-sample.pdf"));
    assert!(!stdout.contains("REDACTION_SENTINEL_OCR_TEXT"));
    assert!(!stdout.contains("REDACTION_SENTINEL_PAGE_IMAGE"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "ocr-gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--min-pages",
            "2",
            "--max-p95-ms",
            "10000",
            "--min-pages-per-second",
            "0.001",
        ])
        .output()
        .expect("run private OCR throughput gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private real-corpus OCR benchmark requires throughput target claim"));

    remove_dir(&root);
    remove_dir(renderer.parent().unwrap());
    remove_dir(ocr.parent().unwrap());
    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_private_ocr_throughput_budget_exhaustion_is_redacted_and_not_gateable() {
    let root = temp_dir("private-ocr-throughput-cli-budget-root");
    fs::write(
        root.join("private-cli-budget-sample.pdf"),
        b"%PDF synthetic private cli budget sample",
    )
    .unwrap();
    let renderer = pdf_render_fixture_script("private-ocr-throughput-cli-budget-renderer");
    let ocr = slow_ocr_fixture_script("private-ocr-throughput-cli-budget-ocr");
    let report_dir = temp_dir("private-ocr-throughput-cli-budget-report");
    let report_path = report_dir.join("ocr-budget-report.json");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "private-ocr-throughput",
            "--root",
            path_str(&root),
            "--renderer-command",
            path_str(&renderer),
            "--command",
            path_str(&ocr),
            "--max-documents",
            "1",
            "--max-pages",
            "2",
            "--pages-per-document",
            "2",
            "--max-run-ms",
            "10",
            "--page-timeout-ms",
            "5000",
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--ocr-runtime-manifest-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "--renderer-manifest-sha256",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--language-pack-manifest-sha256",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "--json",
        ])
        .output()
        .expect("run private OCR throughput benchmark with run budget");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"page_count\":1"));
    assert!(stdout.contains("\"run_budget_exhausted\":true"));
    assert!(!stdout.contains(path_str(&root)));
    assert!(!stdout.contains(path_str(&renderer)));
    assert!(!stdout.contains(path_str(&ocr)));
    assert!(!stdout.contains("private-cli-budget-sample.pdf"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "ocr-gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--min-pages",
            "1",
            "--max-p95-ms",
            "10000",
            "--min-pages-per-second",
            "0.001",
        ])
        .output()
        .expect("run private OCR throughput gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr)
        .contains("private real-corpus OCR benchmark run budget exhausted"));

    remove_dir(&root);
    remove_dir(renderer.parent().unwrap());
    remove_dir(ocr.parent().unwrap());
    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_ocr_gate_requires_private_real_corpus_report() {
    let report_dir = temp_dir("ocr-throughput-cli-private-real-gate");
    let report_path = report_dir.join("ocr-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"ocr-throughput.v1\",",
            "\"dataset_kind\":\"synthetic\",",
            "\"page_count\":500,",
            "\"pages_per_second\":2.5,",
            "\"page_latency_ms\":{\"samples\":500,\"p95\":450},",
            "\"target_claim\":\"not_evaluated\"}"
        ),
    )
    .unwrap();

    let synthetic_gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "ocr-gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--min-pages",
            "500",
            "--max-p95-ms",
            "1000",
            "--min-pages-per-second",
            "1",
        ])
        .output()
        .expect("run OCR throughput gate");

    assert!(!synthetic_gate.status.success());
    assert!(String::from_utf8_lossy(&synthetic_gate.stderr)
        .contains("private real-corpus OCR benchmark required"));

    fs::write(
        &report_path,
        concat!(
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
        ),
    )
    .unwrap();

    let private_gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "ocr-gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--min-pages",
            "500",
            "--max-p95-ms",
            "1000",
            "--min-pages-per-second",
            "1",
        ])
        .output()
        .expect("run OCR throughput gate");

    assert!(
        private_gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&private_gate.stdout),
        String::from_utf8_lossy(&private_gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&private_gate.stdout).trim(),
        "OCR throughput gate passed"
    );
    assert!(private_gate.stderr.is_empty());

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_ocr_gate_rejects_private_real_inconsistent_throughput() {
    let report_dir = temp_dir("ocr-throughput-private-real-inconsistent");
    let report_path = report_dir.join("ocr-report.json");
    fs::write(
        &report_path,
        concat!(
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
            "\"pages_per_second\":9.9,",
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
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "ocr-gate",
            "--report",
            path_str(&report_path),
            "--require-private-real-corpus",
            "--min-pages",
            "500",
            "--max-p95-ms",
            "1000",
            "--min-pages-per-second",
            "1",
        ])
        .output()
        .expect("run OCR throughput gate");

    assert!(!gate.status.success());
    let stderr = String::from_utf8_lossy(&gate.stderr);
    assert!(stderr.contains("private real-corpus OCR throughput metric counts do not match scores"));
    assert!(!stderr.contains(path_str(&report_path)));

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_vector_quality_outputs_redacted_report_and_gate() {
    let command = embedding_fixture_script("vector-quality-cli-private-command");
    let dataset_dir = temp_dir("vector-quality-cli-dataset");
    let dataset_path = dataset_dir.join("vector-quality.jsonl");
    let report_path = dataset_dir.join("vector-report.json");
    fs::write(
        &dataset_path,
        concat!(
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
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "vector-quality",
            "--dataset",
            path_str(&dataset_path),
            "--command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "3",
            "--top-k",
            "1",
            "--json",
        ])
        .output()
        .expect("run vector quality benchmark");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"vector-quality.v1\""));
    assert!(stdout.contains("\"dataset_kind\":\"labeled\""));
    assert!(stdout.contains("\"sample_count\":2"));
    assert!(stdout.contains("\"candidate_count\":4"));
    assert!(stdout.contains("\"top_k\":1"));
    assert!(stdout.contains("\"target_claim\":\"not_evaluated\""));
    assert!(!stdout.contains(path_str(&dataset_path)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains("private-vector-case-a"));
    assert!(!stdout.contains("private-java-doc"));
    assert!(!stdout.contains("Backend Java payment search"));
    assert!(!stdout.contains("Java payment backend"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "vector-gate",
            "--report",
            path_str(&report_path),
            "--min-samples",
            "2",
            "--min-recall-at-k",
            "0.99",
            "--min-mrr",
            "0.99",
            "--min-ndcg-at-k",
            "0.99",
            "--max-zero-recall-queries",
            "0",
        ])
        .output()
        .expect("run vector quality gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "vector quality gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(command.parent().unwrap());
    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_private_business_vector_quality_outputs_redacted_gateable_report() {
    let command = embedding_fixture_script("private-business-vector-quality-cli-command");
    let dataset_dir = temp_dir("private-business-vector-quality-dataset");
    let dataset_path = dataset_dir.join("private-vector-quality.jsonl");
    let report_path = dataset_dir.join("private-vector-report.json");
    fs::write(&dataset_path, private_business_vector_quality_dataset()).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "vector-quality",
            "--dataset",
            path_str(&dataset_path),
            "--command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "3",
            "--top-k",
            "1",
            "--private-business-labeled",
            "--dataset-manifest-sha256",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "--annotation-manifest-sha256",
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            "--model-manifest-sha256",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "--json",
        ])
        .output()
        .expect("run private business vector-quality benchmark");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\":\"vector-quality.v1\""));
    assert!(stdout.contains("\"dataset_kind\":\"private-business-labeled\""));
    assert!(stdout.contains("\"sample_count\":2"));
    assert!(stdout.contains("\"candidate_count\":4"));
    assert!(stdout.contains("\"top_k\":1"));
    assert!(stdout.contains("\"target_claim\":\"vector_quality_target_met\""));
    assert!(stdout.contains("\"corpus_origin\":\"private_local\""));
    assert!(stdout.contains("\"privacy_boundary\":\"redacted_local_aggregate\""));
    assert!(stdout.contains("\"contains_raw_queries\":false"));
    assert!(stdout.contains("\"contains_candidate_text\":false"));
    assert!(stdout.contains("\"contains_resume_paths\":false"));
    assert!(stdout.contains("\"contains_sample_ids\":false"));
    assert!(stdout.contains("\"contains_candidate_ids\":false"));
    assert!(stdout.contains("\"contains_vectors\":false"));
    assert!(stdout.contains("\"vector_taxonomy\":\"resume-ir.vector-quality.v1\""));
    assert!(!stdout.contains(path_str(&dataset_path)));
    assert!(!stdout.contains(path_str(&command)));
    assert!(!stdout.contains("fixture-local-model"));
    assert!(!stdout.contains("\"dimension\""));
    assert!(!stdout.contains("private-vector-sample-001"));
    assert!(!stdout.contains("private-vector-candidate-001"));
    assert!(!stdout.contains("REDACTION_SENTINEL_VECTOR_QUERY"));
    assert!(!stdout.contains("REDACTION_SENTINEL_VECTOR_CANDIDATE"));
    assert!(!stdout.contains("1.0,0.0,0.0"));
    fs::write(&report_path, &output.stdout).unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "vector-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "2",
            "--min-recall-at-k",
            "0.90",
            "--min-mrr",
            "0.90",
            "--min-ndcg-at-k",
            "0.90",
            "--max-zero-recall-queries",
            "0",
        ])
        .output()
        .expect("run private business vector quality gate");

    assert!(
        gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gate.stdout),
        String::from_utf8_lossy(&gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&gate.stdout).trim(),
        "vector quality gate passed"
    );
    assert!(gate.stderr.is_empty());

    remove_dir(command.parent().unwrap());
    remove_dir(&dataset_dir);
}

#[test]
fn resume_benchmark_vector_gate_rejects_unproven_target_claim() {
    let report_dir = temp_dir("vector-quality-cli-gate-reject");
    let report_path = report_dir.join("vector-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"vector-quality.v1\",",
            "\"dataset_kind\":\"labeled\",",
            "\"sample_count\":10,",
            "\"candidate_count\":20,",
            "\"top_k\":5,",
            "\"recall_at_k\":1.0,",
            "\"mrr\":1.0,",
            "\"ndcg_at_k\":1.0,",
            "\"zero_recall_queries\":0,",
            "\"target_claim\":\"production_semantic_quality_met\"}"
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "vector-gate",
            "--report",
            path_str(&report_path),
            "--min-samples",
            "10",
            "--min-recall-at-k",
            "0.99",
            "--min-mrr",
            "0.99",
            "--min-ndcg-at-k",
            "0.99",
        ])
        .output()
        .expect("run vector quality gate");

    assert!(!gate.status.success());
    assert!(String::from_utf8_lossy(&gate.stderr).contains("vector target claim is not proven"));

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_vector_gate_requires_private_business_labeled_report() {
    let report_dir = temp_dir("vector-quality-private-business-gate");
    let report_path = report_dir.join("vector-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"vector-quality.v1\",",
            "\"dataset_kind\":\"labeled\",",
            "\"sample_count\":10,",
            "\"candidate_count\":20,",
            "\"top_k\":5,",
            "\"recall_at_k\":1.0,",
            "\"mrr\":1.0,",
            "\"ndcg_at_k\":1.0,",
            "\"zero_recall_queries\":0,",
            "\"target_claim\":\"not_evaluated\"}"
        ),
    )
    .unwrap();

    let ordinary_gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "vector-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "10",
            "--min-recall-at-k",
            "0.99",
            "--min-mrr",
            "0.99",
            "--min-ndcg-at-k",
            "0.99",
        ])
        .output()
        .expect("run vector quality gate");

    assert!(!ordinary_gate.status.success());
    assert!(String::from_utf8_lossy(&ordinary_gate.stderr)
        .contains("private business vector-quality benchmark required"));

    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"vector-quality.v1\",",
            "\"run_id\":\"vector_release_20260605\",",
            "\"platform\":\"macos/aarch64\",",
            "\"dataset_kind\":\"private-business-labeled\",",
            "\"sample_count\":10,",
            "\"candidate_count\":20,",
            "\"top_k\":5,",
            "\"recall_at_k\":1.0,",
            "\"mrr\":1.0,",
            "\"ndcg_at_k\":1.0,",
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
        ),
    )
    .unwrap();

    let private_gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "vector-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "10",
            "--min-recall-at-k",
            "0.99",
            "--min-mrr",
            "0.99",
            "--min-ndcg-at-k",
            "0.99",
            "--max-zero-recall-queries",
            "0",
        ])
        .output()
        .expect("run vector quality gate");

    assert!(
        private_gate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&private_gate.stdout),
        String::from_utf8_lossy(&private_gate.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&private_gate.stdout).trim(),
        "vector quality gate passed"
    );
    assert!(private_gate.stderr.is_empty());

    remove_dir(&report_dir);
}

#[test]
fn resume_benchmark_vector_gate_rejects_private_business_impossible_top_k() {
    let report_dir = temp_dir("vector-quality-private-business-impossible-top-k");
    let report_path = report_dir.join("vector-report.json");
    fs::write(
        &report_path,
        concat!(
            "{\"schema_version\":\"vector-quality.v1\",",
            "\"run_id\":\"vector_release_20260605\",",
            "\"platform\":\"macos/aarch64\",",
            "\"dataset_kind\":\"private-business-labeled\",",
            "\"sample_count\":10,",
            "\"candidate_count\":3,",
            "\"top_k\":5,",
            "\"recall_at_k\":1.0,",
            "\"mrr\":1.0,",
            "\"ndcg_at_k\":1.0,",
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
        ),
    )
    .unwrap();

    let gate = Command::new(env!("CARGO_BIN_EXE_resume-benchmark"))
        .args([
            "vector-gate",
            "--report",
            path_str(&report_path),
            "--require-private-business-labeled",
            "--min-samples",
            "10",
            "--min-recall-at-k",
            "0.99",
            "--min-mrr",
            "0.99",
            "--min-ndcg-at-k",
            "0.99",
            "--max-zero-recall-queries",
            "0",
        ])
        .output()
        .expect("run vector quality gate");

    assert!(!gate.status.success());
    let stderr = String::from_utf8_lossy(&gate.stderr);
    assert!(stderr.contains("private business vector quality counts are inconsistent"));
    assert!(!stderr.contains(path_str(&report_path)));

    remove_dir(&report_dir);
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

fn query_fixture_script(label: &str) -> PathBuf {
    query_fixture_script_with_body(label, query_fixture_script_body())
}

fn query_fixture_script_requiring_args(label: &str) -> PathBuf {
    query_fixture_script_with_body(label, query_fixture_script_requiring_args_body())
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

fn private_query_corpus_summary_file(
    label: &str,
    document_count: usize,
    hot_index: bool,
) -> PathBuf {
    let path = temp_dir(label).join("benchmark-corpus-summary.json");
    let searchable_count = if hot_index {
        document_count
    } else {
        document_count.saturating_sub(1)
    };
    let vector_count = if hot_index {
        document_count
    } else {
        document_count.saturating_sub(2)
    };
    fs::write(
        &path,
        format!(
            concat!(
                "{{",
                "\"schema_version\":\"benchmark-corpus-summary.v1\",",
                "\"privacy_boundary\":\"redacted_local_aggregate\",",
                "\"document_count\":{},",
                "\"searchable_document_count\":{},",
                "\"vector_indexed_document_count\":{},",
                "\"active_vector_document_count\":{},",
                "\"vector_count\":{},",
                "\"vector_deleted_count\":0,",
                "\"vector_index_state\":\"available\",",
                "\"vector_search_backend\":\"hnsw_ann\",",
                "\"hot_index_fully_covered\":{},",
                "\"contains_raw_resume_text\":false,",
                "\"contains_resume_paths\":false,",
                "\"contains_queries\":false,",
                "\"contains_sample_ids\":false",
                "}}"
            ),
            document_count, searchable_count, vector_count, vector_count, vector_count, hot_index
        ),
    )
    .unwrap();
    path
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
fn query_fixture_script_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "if grep -q REDACTION_SENTINEL_PRIVATE_QUERY \"$RESUME_IR_QUERY_INPUT_PATH\"; then\n",
        "  printf 'resume-ir-query-v1\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nhits=%s\\n' \"$RESUME_IR_QUERY_TOP_K\" \"$RESUME_IR_QUERY_TOP_K\"\n",
        "else\n",
        "  printf 'resume-ir-query-v1\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nhits=0\\n' \"$RESUME_IR_QUERY_TOP_K\"\n",
        "fi\n",
    )
}

#[cfg(unix)]
fn query_fixture_script_requiring_args_body() -> &'static str {
    concat!(
        "#!/bin/sh\n",
        "if [ \"$1\" != \"resume-cli\" ] || [ \"$2\" != \"benchmark-query-protocol\" ]; then\n",
        "  exit 7\n",
        "fi\n",
        "if grep -q REDACTION_SENTINEL_PRIVATE_QUERY \"$RESUME_IR_QUERY_INPUT_PATH\"; then\n",
        "  printf 'resume-ir-query-v1\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nhits=%s\\n' \"$RESUME_IR_QUERY_TOP_K\" \"$RESUME_IR_QUERY_TOP_K\"\n",
        "else\n",
        "  printf 'resume-ir-query-v1\\nmode=hybrid\\nlayers=fulltext+field+vector+rrf\\ntop_k=%s\\nhits=0\\n' \"$RESUME_IR_QUERY_TOP_K\"\n",
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
fn query_fixture_script_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" \"%RESUME_IR_QUERY_INPUT_PATH%\" >nul\r\n",
        "if errorlevel 1 (\r\n",
        "  echo resume-ir-query-v1\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo hits=0\r\n",
        ") else (\r\n",
        "  echo resume-ir-query-v1\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
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

#[cfg(windows)]
fn query_fixture_script_requiring_args_body() -> &'static str {
    concat!(
        "@echo off\r\n",
        "if not \"%1\"==\"resume-cli\" exit /b 7\r\n",
        "if not \"%2\"==\"benchmark-query-protocol\" exit /b 7\r\n",
        "findstr /C:\"REDACTION_SENTINEL_PRIVATE_QUERY\" \"%RESUME_IR_QUERY_INPUT_PATH%\" >nul\r\n",
        "if %ERRORLEVEL%==0 (\r\n",
        "  echo resume-ir-query-v1\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo hits=%RESUME_IR_QUERY_TOP_K%\r\n",
        ") else (\r\n",
        "  echo resume-ir-query-v1\r\n",
        "  echo mode=hybrid\r\n",
        "  echo layers=fulltext+field+vector+rrf\r\n",
        "  echo top_k=%RESUME_IR_QUERY_TOP_K%\r\n",
        "  echo hits=0\r\n",
        ")\r\n",
        "exit /b 0\r\n",
    )
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
