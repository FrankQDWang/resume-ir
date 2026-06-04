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
    assert!(!stdout.contains("PRIVATE OCR PAYLOAD"));
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

fn embedding_fixture_script(label: &str) -> PathBuf {
    let path = temp_dir(label).join("embedding-fixture.sh");
    fs::write(
        &path,
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
"#,
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
