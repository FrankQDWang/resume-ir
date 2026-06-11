use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, IndexDocument, IndexSection};
use index_vector::{PersistentVectorIndex, VectorDocument, VectorIndex};
use meta_store::{
    Document, DocumentId, DocumentStatus, FileExtension, MetaStore, ResumeVersion, ResumeVersionId,
    ResumeVisibility, UnixTimestamp,
};

#[test]
fn model_manifest_validate_accepts_reviewed_local_artifact_without_path_or_payload_leak() {
    let data_dir = temp_dir("model-manifest-valid-private-data");
    let model_file = temp_file("model-manifest-valid-private-model");
    let manifest_file = temp_file("model-manifest-valid-private-manifest");
    let model_bytes = b"SYNTHETIC REVIEWED MODEL ARTIFACT\n";
    fs::write(&model_file, model_bytes).unwrap();
    fs::write(
        &manifest_file,
        format!(
            r#"{{
  "schema_version": "resume-ir.model-manifest.v1",
  "model_pack_id": "fixture-pack-reviewed",
  "models": [
    {{
      "id": "fixture-reviewed-embedding-model",
      "type": "embedding",
      "dim": 4,
      "format": "onnx",
      "artifact": {{
        "path": "{}",
        "sha256": "57aac1132f550796663cdadce2ae702cb0bbf96b8620bc12f385d7b8aae0e492"
      }},
      "license": {{
        "id": "Apache-2.0",
        "reviewed": true
      }}
    }}
  ]
}}"#,
            json_path(&model_file)
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "model",
            "validate-manifest",
            "--manifest",
            path_str(&manifest_file),
        ])
        .output()
        .expect("validate reviewed model manifest");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("model manifest: valid"));
    assert!(stdout.contains("model pack: fixture-pack-reviewed"));
    assert!(stdout.contains("models: 1"));
    assert!(stdout.contains("model id: fixture-reviewed-embedding-model"));
    assert!(stdout.contains("type: embedding"));
    assert!(stdout.contains("dimension: 4"));
    assert!(stdout.contains("license reviewed: yes"));
    assert!(stdout.contains("checksum match: yes"));
    assert!(stdout.contains("sha256 prefix: 57aac113"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains("SYNTHETIC REVIEWED MODEL ARTIFACT"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&model_file)));
    assert!(!stdout.contains(path_str(&manifest_file)));

    remove_dir(&data_dir);
    let _ = fs::remove_file(&model_file);
    let _ = fs::remove_file(&manifest_file);
}

#[test]
fn model_manifest_validate_rejects_checksum_mismatch_without_path_or_payload_leak() {
    let data_dir = temp_dir("model-manifest-mismatch-private-data");
    let model_file = temp_file("model-manifest-mismatch-private-model");
    let manifest_file = temp_file("model-manifest-mismatch-private-manifest");
    fs::write(&model_file, b"SYNTHETIC MISMATCH MODEL ARTIFACT\n").unwrap();
    fs::write(
        &manifest_file,
        format!(
            r#"{{
  "schema_version": "resume-ir.model-manifest.v1",
  "model_pack_id": "fixture-pack-mismatch",
  "models": [
    {{
      "id": "fixture-mismatch-embedding-model",
      "type": "embedding",
      "dim": 4,
      "format": "onnx",
      "artifact": {{
        "path": "{}",
        "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
      }},
      "license": {{
        "id": "Apache-2.0",
        "reviewed": true
      }}
    }}
  ]
}}"#,
            json_path(&model_file)
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "model",
            "validate-manifest",
            "--manifest",
            path_str(&manifest_file),
        ])
        .output()
        .expect("reject checksum mismatch model manifest");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("model manifest blocked: checksum mismatch"));
    assert!(!stderr.contains("SYNTHETIC MISMATCH MODEL ARTIFACT"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&model_file)));
    assert!(!stderr.contains(path_str(&manifest_file)));

    remove_dir(&data_dir);
    let _ = fs::remove_file(&model_file);
    let _ = fs::remove_file(&manifest_file);
}

#[test]
fn model_manifest_validate_rejects_unreviewed_license_without_path_or_payload_leak() {
    let data_dir = temp_dir("model-manifest-unreviewed-private-data");
    let model_file = temp_file("model-manifest-unreviewed-private-model");
    let manifest_file = temp_file("model-manifest-unreviewed-private-manifest");
    fs::write(&model_file, b"SYNTHETIC UNREVIEWED MODEL ARTIFACT\n").unwrap();
    fs::write(
        &manifest_file,
        format!(
            r#"{{
  "schema_version": "resume-ir.model-manifest.v1",
  "model_pack_id": "fixture-pack-unreviewed",
  "models": [
    {{
      "id": "fixture-unreviewed-embedding-model",
      "type": "embedding",
      "dim": 4,
      "format": "onnx",
      "artifact": {{
        "path": "{}",
        "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
      }},
      "license": {{
        "id": "Proprietary",
        "reviewed": false
      }}
    }}
  ]
}}"#,
            json_path(&model_file)
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "model",
            "validate-manifest",
            "--manifest",
            path_str(&manifest_file),
        ])
        .output()
        .expect("reject unreviewed model manifest");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("model manifest blocked: license has not been reviewed"));
    assert!(!stderr.contains("SYNTHETIC UNREVIEWED MODEL ARTIFACT"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&model_file)));
    assert!(!stderr.contains(path_str(&manifest_file)));

    remove_dir(&data_dir);
    let _ = fs::remove_file(&model_file);
    let _ = fs::remove_file(&manifest_file);
}

#[test]
fn model_manifest_draft_writes_local_manifest_without_stdout_path_or_payload_leak() {
    let data_dir = temp_dir("model-manifest-draft-private-data");
    let model_file = temp_file("model-manifest-draft-private-model");
    let manifest_file = temp_file("model-manifest-draft-private-manifest");
    fs::write(&model_file, b"SYNTHETIC DRAFT MODEL ARTIFACT\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "model",
            "draft-manifest",
            "--out",
            path_str(&manifest_file),
            "--model-pack-id",
            "fixture-pack-draft",
            "--model-id",
            "fixture-draft-embedding-model",
            "--model-type",
            "embedding",
            "--dimension",
            "4",
            "--format",
            "onnx",
            "--artifact",
            path_str(&model_file),
            "--license",
            "Apache-2.0",
            "--reviewed",
        ])
        .output()
        .expect("draft model manifest");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("model manifest draft: written"));
    assert!(stdout.contains("schema: resume-ir.model-manifest.v1"));
    assert!(stdout.contains("model pack: fixture-pack-draft"));
    assert!(stdout.contains("model id: fixture-draft-embedding-model"));
    assert!(stdout.contains("license reviewed: yes"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains("SYNTHETIC DRAFT MODEL ARTIFACT"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&model_file)));
    assert!(!stdout.contains(path_str(&manifest_file)));

    let manifest = fs::read_to_string(&manifest_file).unwrap();
    assert!(manifest.contains("\"schema_version\": \"resume-ir.model-manifest.v1\""));
    assert!(manifest.contains("\"model_pack_id\": \"fixture-pack-draft\""));
    assert!(manifest.contains("\"id\": \"fixture-draft-embedding-model\""));
    assert!(manifest.contains("\"type\": \"embedding\""));
    assert!(manifest.contains("\"dim\": 4"));
    assert!(manifest.contains("\"format\": \"onnx\""));
    assert!(manifest.contains("\"reviewed\": true"));
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(
        manifest_json["models"][0]["artifact"]["path"],
        path_str(&model_file)
    );

    let validate = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "model",
            "validate-manifest",
            "--manifest",
            path_str(&manifest_file),
        ])
        .output()
        .expect("validate drafted model manifest");

    assert!(
        validate.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&validate.stdout),
        String::from_utf8_lossy(&validate.stderr)
    );

    remove_dir(&data_dir);
    let _ = fs::remove_file(&model_file);
    let _ = fs::remove_file(&manifest_file);
}

#[test]
fn model_manifest_draft_without_review_fails_validation_without_path_or_payload_leak() {
    let data_dir = temp_dir("model-manifest-draft-unreviewed-private-data");
    let model_file = temp_file("model-manifest-draft-unreviewed-private-model");
    let manifest_file = temp_file("model-manifest-draft-unreviewed-private-manifest");
    fs::write(&model_file, b"SYNTHETIC UNREVIEWED DRAFT MODEL ARTIFACT\n").unwrap();

    let draft = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "model",
            "draft-manifest",
            "--out",
            path_str(&manifest_file),
            "--model-pack-id",
            "fixture-pack-draft-unreviewed",
            "--model-id",
            "fixture-draft-unreviewed-embedding-model",
            "--model-type",
            "embedding",
            "--dimension",
            "4",
            "--format",
            "onnx",
            "--artifact",
            path_str(&model_file),
            "--license",
            "Apache-2.0",
        ])
        .output()
        .expect("draft unreviewed model manifest");

    assert!(draft.status.success());
    let draft_stdout = String::from_utf8_lossy(&draft.stdout);
    assert!(draft_stdout.contains("license reviewed: no"));
    assert!(draft_stdout.contains("paths: <redacted>"));
    assert!(!draft_stdout.contains(path_str(&model_file)));
    assert!(!draft_stdout.contains("SYNTHETIC UNREVIEWED DRAFT MODEL ARTIFACT"));

    let validate = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "model",
            "validate-manifest",
            "--manifest",
            path_str(&manifest_file),
        ])
        .output()
        .expect("reject unreviewed drafted model manifest");

    assert!(!validate.status.success());
    assert!(validate.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&validate.stderr);
    assert!(stderr.contains("model manifest blocked: license has not been reviewed"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&model_file)));
    assert!(!stderr.contains(path_str(&manifest_file)));
    assert!(!stderr.contains("SYNTHETIC UNREVIEWED DRAFT MODEL ARTIFACT"));

    remove_dir(&data_dir);
    let _ = fs::remove_file(&model_file);
    let _ = fs::remove_file(&manifest_file);
}

#[cfg(unix)]
#[test]
fn model_preflight_json_reports_ready_embedding_runtime_without_path_or_payload_leak() {
    let data_dir = temp_dir("model-preflight-ready-private-data");
    let model_file = temp_file("model-preflight-ready-private-model");
    let manifest_file = temp_file("model-preflight-ready-private-manifest");
    let command = write_fixture_executable(
        "fixture-model-preflight-embedding",
        "#!/bin/sh\nprintf 'resume-ir-embedding-v1\\n0.1 0.2 0.3 0.4\\n'\n",
    );
    fs::write(&model_file, b"SYNTHETIC PREFLIGHT MODEL ARTIFACT\n").unwrap();
    fs::write(
        &manifest_file,
        format!(
            r#"{{
  "schema_version": "resume-ir.model-manifest.v1",
  "model_pack_id": "fixture-pack-preflight",
  "models": [
    {{
      "id": "fixture-preflight-embedding-model",
      "type": "embedding",
      "dim": 4,
      "format": "onnx",
      "artifact": {{
        "path": "{}",
        "sha256": "59d0375390fbca113f4326678f67a60fae1526667e0688e395d0faa9e137e2c5"
      }},
      "license": {{
        "id": "Apache-2.0",
        "reviewed": true
      }}
    }}
  ]
}}"#,
            json_path(&model_file)
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "model",
            "preflight",
            "--json",
            "--manifest",
            path_str(&manifest_file),
            "--embedding-command",
            path_str(&command),
            "--model-id",
            "fixture-preflight-embedding-model",
            "--dimension",
            "4",
        ])
        .output()
        .expect("run model preflight");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\": \"embedding-runtime-preflight.v1\""));
    assert!(stdout.contains("\"runtime_status\": \"ready\""));
    assert!(stdout.contains("\"model_manifest\": \"valid\""));
    assert!(stdout.contains("\"embedding_command\": \"available\""));
    assert!(stdout.contains("\"model_id\": \"fixture-preflight-embedding-model\""));
    assert!(stdout.contains("\"dimension\": 4"));
    assert!(stdout.contains("\"paths\": \"<redacted>\""));
    assert!(stdout.contains("\"remediation\": []"));
    assert!(!stdout.contains("SYNTHETIC PREFLIGHT MODEL ARTIFACT"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&model_file)));
    assert!(!stdout.contains(path_str(&manifest_file)));
    assert!(!stdout.contains(path_str(&command)));

    remove_dir(&data_dir);
    let _ = fs::remove_file(&model_file);
    let _ = fs::remove_file(&manifest_file);
}

#[test]
fn model_preflight_json_blocks_missing_embedding_command_without_path_leak() {
    let data_dir = temp_dir("model-preflight-missing-command-private-data");
    let model_file = temp_file("model-preflight-missing-command-private-model");
    let manifest_file = temp_file("model-preflight-missing-command-private-manifest");
    let missing_command = temp_file("model-preflight-missing-command-private-bin");
    fs::write(&model_file, b"SYNTHETIC PREFLIGHT MODEL ARTIFACT\n").unwrap();
    fs::write(
        &manifest_file,
        format!(
            r#"{{
  "schema_version": "resume-ir.model-manifest.v1",
  "model_pack_id": "fixture-pack-preflight",
  "models": [
    {{
      "id": "fixture-preflight-embedding-model",
      "type": "embedding",
      "dim": 4,
      "format": "onnx",
      "artifact": {{
        "path": "{}",
        "sha256": "59d0375390fbca113f4326678f67a60fae1526667e0688e395d0faa9e137e2c5"
      }},
      "license": {{
        "id": "Apache-2.0",
        "reviewed": true
      }}
    }}
  ]
}}"#,
            json_path(&model_file)
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "model",
            "preflight",
            "--json",
            "--manifest",
            path_str(&manifest_file),
            "--embedding-command",
            path_str(&missing_command),
            "--model-id",
            "fixture-preflight-embedding-model",
            "--dimension",
            "4",
        ])
        .output()
        .expect("run missing model preflight");

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("embedding runtime preflight blocked"));
    assert!(stdout.contains("\"schema_version\": \"embedding-runtime-preflight.v1\""));
    assert!(stdout.contains("\"runtime_status\": \"blocked\""));
    assert!(stdout.contains("\"model_manifest\": \"valid\""));
    assert!(stdout.contains("\"embedding_command\": \"missing\""));
    assert!(stdout.contains("configure --embedding-command with a local executable"));
    assert!(stdout.contains("\"paths\": \"<redacted>\""));
    assert!(!stdout.contains("SYNTHETIC PREFLIGHT MODEL ARTIFACT"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&model_file)));
    assert!(!stdout.contains(path_str(&manifest_file)));
    assert!(!stdout.contains(path_str(&missing_command)));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&missing_command)));

    remove_dir(&data_dir);
    let _ = fs::remove_file(&model_file);
    let _ = fs::remove_file(&manifest_file);
}

#[test]
fn embed_worker_without_command_reports_blocked_without_path_leak() {
    let data_dir = temp_dir("embed-worker-no-command-data");
    let fixture_root = fixture_root();
    import_fixtures(&data_dir, &fixture_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "embed-worker", "--once"])
        .output()
        .expect("run embed worker without command");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("embedding worker blocked: local embedding command not configured"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&fixture_root)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn embed_worker_runs_local_command_and_persists_vector_snapshot_without_hiding_search_results() {
    let data_dir = temp_dir("embed-worker-command-data");
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-embedding-worker",
        r#"#!/bin/sh
if [ ! -s "$RESUME_IR_EMBEDDING_INPUT_PATH" ]; then
  exit 7
fi
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { sub(/^input=/, "", $1); printf "vector=%s\t0.5,0.5,0.5,0.5\n", $1 }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
printf 'metadata=synthetic-fixture\n'
"#,
    );
    import_fixtures(&data_dir, &fixture_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "embed-worker",
            "--once",
            "--command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "4",
            "--max-docs",
            "8",
            "--max-text-bytes",
            "100000",
        ])
        .output()
        .expect("run embed worker with local command");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("embedding worker: completed"));
    assert!(stdout.contains("model id: fixture-local-model"));
    assert!(stdout.contains("dimension: 4"));
    assert!(stdout.contains("documents considered: 2"));
    assert!(stdout.contains("documents embedded: 2"));
    assert!(stdout.contains("vector inputs: "));
    assert!(stdout_value(&stdout, "vector inputs: ") > 2);
    assert!(stdout.contains("vector index: available (hnsw ann vector snapshot)"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status");
    assert!(status.status.success());
    assert!(status.stderr.is_empty());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("searchable documents: 2"));
    assert!(status_stdout.contains("vector index: available (hnsw ann vector snapshot)"));
    assert!(stdout_value(&status_stdout, "vector index vectors: ") > 2);
    assert!(!status_stdout.contains(path_str(&data_dir)));
    assert!(!status_stdout.contains(path_str(&fixture_root)));

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search after embedding");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let search_stdout = String::from_utf8_lossy(&search.stdout);
    assert!(search_stdout.contains("results: 2"));
    assert!(search_stdout.contains("synthetic-java-platform.pdf"));
    assert!(search_stdout.contains("synthetic-java-engineer.docx"));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn semantic_and_hybrid_search_use_persistent_vector_snapshot_with_local_query_embedding() {
    let data_dir = temp_dir("semantic-search-data");
    let query_dir = temp_dir("semantic-search-query-file-private-input");
    let query_file = query_dir.join("query.txt");
    fs::write(&query_file, "SemanticOnlyToken\n").unwrap();
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-semantic-search-embedding",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t1,0,0,0\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );
    import_fixtures(&data_dir, &fixture_root);

    let embed = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "embed-worker",
            "--once",
            "--command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "4",
            "--max-docs",
            "8",
            "--max-text-bytes",
            "100000",
        ])
        .output()
        .expect("run embed worker before semantic search");
    assert!(
        embed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&embed.stdout),
        String::from_utf8_lossy(&embed.stderr)
    );

    for mode in ["semantic", "hybrid"] {
        let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
            .args([
                "--data-dir",
                path_str(&data_dir),
                "search",
                "--query-file",
                path_str(&query_file),
                "--mode",
                mode,
                "--embedding-command",
                path_str(&command),
                "--model-id",
                "fixture-local-model",
                "--top-k",
                "20",
            ])
            .output()
            .expect("run semantic or hybrid search");
        assert!(
            search.status.success(),
            "mode: {mode}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&search.stdout),
            String::from_utf8_lossy(&search.stderr)
        );
        assert!(search.stderr.is_empty());
        let stdout = String::from_utf8_lossy(&search.stdout);
        assert!(
            stdout.contains("results: 2"),
            "mode: {mode}\nstdout:\n{stdout}"
        );
        assert!(stdout.contains("synthetic-java-platform.pdf"));
        assert!(stdout.contains("synthetic-java-engineer.docx"));
        assert!(!stdout.contains("SemanticOnlyToken"));
        assert!(!stdout.contains(path_str(&query_file)));
        assert!(!stdout.contains(path_str(&query_dir)));
        assert!(!stdout.contains(path_str(&data_dir)));
        assert!(!stdout.contains(path_str(&fixture_root)));
    }

    remove_dir(&query_dir);
    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn benchmark_query_protocol_runs_hybrid_search_without_result_or_query_leaks() {
    let data_dir = temp_dir("benchmark-query-protocol-data");
    let query_dir = temp_dir("benchmark-query-protocol-private-input");
    let query_file = query_dir.join("query.txt");
    fs::write(&query_file, "SemanticOnlyToken\n").unwrap();
    let fixture_root = fixture_root();
    let command = write_fixture_executable(
        "fixture-benchmark-query-protocol-embedding",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t1,0,0,0\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );
    import_fixtures(&data_dir, &fixture_root);

    let embed = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "embed-worker",
            "--once",
            "--command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "4",
            "--max-docs",
            "8",
            "--max-text-bytes",
            "100000",
        ])
        .output()
        .expect("run embed worker before benchmark query protocol");
    assert!(
        embed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&embed.stdout),
        String::from_utf8_lossy(&embed.stderr)
    );

    let protocol = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-query-protocol",
            "--embedding-command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "4",
        ])
        .env("RESUME_IR_QUERY_INPUT_PATH", path_str(&query_file))
        .env("RESUME_IR_QUERY_TOP_K", "20")
        .env("RESUME_IR_QUERY_MODE", "hybrid")
        .output()
        .expect("run benchmark query protocol");

    assert!(
        protocol.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&protocol.stdout),
        String::from_utf8_lossy(&protocol.stderr)
    );
    assert!(protocol.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&protocol.stdout);
    assert_eq!(stdout, "resume-ir-query-v1\nhits=2\n");
    assert!(!stdout.contains("SemanticOnlyToken"));
    assert!(!stdout.contains("synthetic-java-platform.pdf"));
    assert!(!stdout.contains("synthetic-java-engineer.docx"));
    assert!(!stdout.contains(path_str(&query_file)));
    assert!(!stdout.contains(path_str(&query_dir)));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&fixture_root)));

    remove_dir(&query_dir);
    remove_dir(&data_dir);
}

#[test]
fn benchmark_corpus_summary_json_reports_redacted_hot_index_coverage_without_private_leaks() {
    let data_dir = temp_dir("benchmark-corpus-summary-private-data");
    let document_a_id = DocumentId::from_non_secret_parts(&["s278", "summary-doc-a"]);
    let document_a_version_id =
        ResumeVersionId::from_non_secret_parts(&["s278", "summary-version-a"]);
    let document_b_id = DocumentId::from_non_secret_parts(&["s278", "summary-doc-b"]);
    let document_b_version_id =
        ResumeVersionId::from_non_secret_parts(&["s278", "summary-version-b"]);
    let ocr_document_id = DocumentId::from_non_secret_parts(&["s278", "summary-ocr-doc"]);
    let private_text = "SyntheticBenchmarkSecretToken local resume text";

    seed_searchable_metadata(
        &data_dir,
        &document_a_id,
        &document_a_version_id,
        "synthetic-summary-a.pdf",
        private_text,
    );
    seed_searchable_metadata(
        &data_dir,
        &document_b_id,
        &document_b_version_id,
        "synthetic-summary-b.docx",
        "Synthetic benchmark corpus second resume",
    );
    seed_document_metadata(
        &data_dir,
        &ocr_document_id,
        "synthetic-summary-needs-ocr.pdf",
        DocumentStatus::OcrRequired,
    );

    let vector_index = PersistentVectorIndex::open(data_dir.join("vector-index"), 4).unwrap();
    vector_index
        .upsert(vec![
            VectorDocument::new_for_model(
                "fixture-summary-model",
                format!("fixture-summary-model:{document_a_version_id}"),
                document_a_id.to_string(),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap(),
            VectorDocument::new_for_model(
                "fixture-summary-model",
                format!("fixture-summary-model:{document_a_version_id}:section:0"),
                document_a_id.to_string(),
                vec![0.9, 0.1, 0.0, 0.0],
            )
            .unwrap(),
            VectorDocument::new_for_model(
                "fixture-summary-model",
                format!("fixture-summary-model:{document_b_version_id}"),
                document_b_id.to_string(),
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .unwrap(),
            VectorDocument::new_for_model(
                "fixture-summary-model",
                "fixture-summary-model:orphan-vector",
                "orphan-private-doc",
                vec![0.0, 0.0, 1.0, 0.0],
            )
            .unwrap(),
            VectorDocument::new_for_model(
                "fixture-summary-model",
                "fixture-summary-model:deleted-vector",
                "deleted-private-doc",
                vec![0.0, 0.0, 0.0, 1.0],
            )
            .unwrap(),
        ])
        .unwrap();
    vector_index
        .mark_deleted(&["fixture-summary-model:deleted-vector"])
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "benchmark-corpus-summary",
            "--json",
        ])
        .output()
        .expect("run benchmark corpus summary");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report: serde_json::Value =
        serde_json::from_str(&stdout).expect("benchmark corpus summary json");

    assert_eq!(report["schema_version"], "benchmark-corpus-summary.v1");
    assert_eq!(report["privacy_boundary"], "redacted_local_aggregate");
    assert_eq!(report["document_count"], 3);
    assert_eq!(report["searchable_document_count"], 2);
    assert_eq!(report["vector_indexed_document_count"], 2);
    assert_eq!(report["active_vector_document_count"], 3);
    assert_eq!(report["vector_count"], 5);
    assert_eq!(report["vector_deleted_count"], 1);
    assert_eq!(report["vector_index_state"], "available");
    assert_eq!(report["vector_search_backend"], "hnsw_ann");
    assert_eq!(report["hot_index_fully_covered"], false);
    assert_eq!(report["contains_raw_resume_text"], false);
    assert_eq!(report["contains_resume_paths"], false);
    assert_eq!(report["contains_queries"], false);
    assert_eq!(report["contains_sample_ids"], false);
    assert!(!stdout.contains(private_text));
    assert!(!stdout.contains("synthetic-summary-a.pdf"));
    assert!(!stdout.contains("synthetic-summary-b.docx"));
    assert!(!stdout.contains("synthetic-summary-needs-ocr.pdf"));
    assert!(!stdout.contains("orphan-private-doc"));
    assert!(!stdout.contains("deleted-private-doc"));
    assert!(!stdout.contains("summary-doc-a"));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn semantic_search_reports_missing_vector_snapshot_even_when_dimension_is_supplied() {
    let data_dir = temp_dir("semantic-missing-vector-snapshot-data");
    let command = write_fixture_executable(
        "fixture-semantic-missing-vector-snapshot-embedding",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t1,0,0,0\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "SemanticOnlyToken",
            "--mode",
            "semantic",
            "--embedding-command",
            path_str(&command),
            "--model-id",
            "fixture-local-model",
            "--dimension",
            "4",
        ])
        .output()
        .expect("run semantic search without a vector snapshot");

    assert!(!search.status.success());
    assert!(search.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&search.stderr);
    assert!(stderr.contains("semantic search unavailable: vector index is missing"));
    assert!(!stderr.contains("SemanticOnlyToken"));
    assert!(!stderr.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn semantic_search_uses_only_vectors_for_requested_model() {
    let data_dir = temp_dir("semantic-model-scope-data");
    let old_document_id = DocumentId::from_non_secret_parts(&["s54", "old-model-doc"]);
    let old_version_id = ResumeVersionId::from_non_secret_parts(&["s54", "old-model-version"]);
    let current_document_id = DocumentId::from_non_secret_parts(&["s54", "current-model-doc"]);
    let current_version_id =
        ResumeVersionId::from_non_secret_parts(&["s54", "current-model-version"]);
    seed_searchable_metadata(
        &data_dir,
        &old_document_id,
        &old_version_id,
        "synthetic-old-model.pdf",
        "old model only semantic vector",
    );
    seed_searchable_metadata(
        &data_dir,
        &current_document_id,
        &current_version_id,
        "synthetic-current-model.pdf",
        "current model semantic vector",
    );
    seed_fulltext_index(
        &data_dir,
        [
            IndexDocument {
                doc_id: old_document_id.to_string(),
                version_id: old_version_id.to_string(),
                file_name: "synthetic-old-model.pdf".to_string(),
                clean_text: "old model only vector document".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "old model only vector document".to_string(),
                }],
                is_deleted: false,
            },
            IndexDocument {
                doc_id: current_document_id.to_string(),
                version_id: current_version_id.to_string(),
                file_name: "synthetic-current-model.pdf".to_string(),
                clean_text: "current model vector document".to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: "current model vector document".to_string(),
                }],
                is_deleted: false,
            },
        ],
    );

    let vector_index = PersistentVectorIndex::open(data_dir.join("vector-index"), 4).unwrap();
    vector_index
        .upsert(vec![
            VectorDocument::new_for_model(
                "model-a",
                format!("model-a:{old_version_id}"),
                old_document_id.to_string(),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap(),
            VectorDocument::new_for_model(
                "model-b",
                format!("model-b:{current_version_id}"),
                current_document_id.to_string(),
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .unwrap(),
        ])
        .unwrap();

    let command = write_fixture_executable(
        "fixture-semantic-model-scope-embedding",
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=model-b\n'
printf 'dimension=4\n'
awk -F '\t' '/^input=/ { id=$1; sub(/^input=/, "", id); printf "vector=%s\t1,0,0,0\n", id }' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#,
    );

    for mode in ["semantic", "hybrid"] {
        let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
            .args([
                "--data-dir",
                path_str(&data_dir),
                "search",
                "SemanticOnlyToken",
                "--mode",
                mode,
                "--embedding-command",
                path_str(&command),
                "--model-id",
                "model-b",
                "--dimension",
                "4",
                "--vector-top-k",
                "1",
                "--top-k",
                "1",
            ])
            .output()
            .expect("run semantic or hybrid search with mixed-model vector snapshot");

        assert!(
            search.status.success(),
            "mode: {mode}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&search.stdout),
            String::from_utf8_lossy(&search.stderr)
        );
        assert!(search.stderr.is_empty());
        let stdout = String::from_utf8_lossy(&search.stdout);
        assert!(stdout.contains("results: 1"), "mode: {mode}\n{stdout}");
        assert!(
            stdout.contains("synthetic-current-model.pdf"),
            "mode: {mode}\n{stdout}"
        );
        assert!(
            !stdout.contains("synthetic-old-model.pdf"),
            "mode: {mode}\n{stdout}"
        );
        assert!(!stdout.contains("SemanticOnlyToken"));
        assert!(!stdout.contains(path_str(&data_dir)));
    }

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn semantic_and_hybrid_search_can_rank_section_vectors_over_document_vectors() {
    let data_dir = temp_dir("semantic-section-vector-data");
    let document_only_id = DocumentId::from_non_secret_parts(&["s55", "document-only-doc"]);
    let document_only_version_id =
        ResumeVersionId::from_non_secret_parts(&["s55", "document-only-version"]);
    let section_match_id = DocumentId::from_non_secret_parts(&["s55", "section-match-doc"]);
    let section_match_version_id =
        ResumeVersionId::from_non_secret_parts(&["s55", "section-match-version"]);
    let document_only_text = "Profile\nSynthetic operations platform summary.";
    let section_match_text = "\
Summary
Synthetic general profile.

Experience
Synthetic section-level retrieval specialist.
";
    seed_searchable_metadata(
        &data_dir,
        &document_only_id,
        &document_only_version_id,
        "synthetic-document-vector.pdf",
        document_only_text,
    );
    seed_searchable_metadata(
        &data_dir,
        &section_match_id,
        &section_match_version_id,
        "synthetic-section-vector.pdf",
        section_match_text,
    );
    seed_fulltext_index(
        &data_dir,
        [
            IndexDocument {
                doc_id: document_only_id.to_string(),
                version_id: document_only_version_id.to_string(),
                file_name: "synthetic-document-vector.pdf".to_string(),
                clean_text: document_only_text.to_string(),
                sections: vec![IndexSection {
                    section_type: "profile".to_string(),
                    text: document_only_text.to_string(),
                }],
                is_deleted: false,
            },
            IndexDocument {
                doc_id: section_match_id.to_string(),
                version_id: section_match_version_id.to_string(),
                file_name: "synthetic-section-vector.pdf".to_string(),
                clean_text: section_match_text.to_string(),
                sections: vec![IndexSection {
                    section_type: "experience".to_string(),
                    text: section_match_text.to_string(),
                }],
                is_deleted: false,
            },
        ],
    );

    let command_body = format!(
        r#"#!/bin/sh
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-section-model\n'
printf 'dimension=4\n'
awk -F '\t' -v doc_a='{document_only_version_id}' -v doc_b='{section_match_version_id}' '/^input=/ {{
  id=$1
  sub(/^input=/, "", id)
  values="0,0,1,0"
  if (id == "query") {{
    values="1,0,0,0"
  }} else if (index(id, doc_b ":section:") == 1) {{
    values="1,0,0,0"
  }} else if (id == doc_a) {{
    values="0.8,0.6,0,0"
  }} else if (id == doc_b) {{
    values="0,1,0,0"
  }}
  printf "vector=%s\t%s\n", id, values
}}' "$RESUME_IR_EMBEDDING_INPUT_PATH"
"#
    );
    let command = write_fixture_executable("fixture-section-vector-embedding", &command_body);

    let embed = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "embed-worker",
            "--once",
            "--command",
            path_str(&command),
            "--model-id",
            "fixture-section-model",
            "--dimension",
            "4",
            "--max-docs",
            "8",
            "--max-text-bytes",
            "100000",
        ])
        .output()
        .expect("run embed worker before section semantic search");
    assert!(
        embed.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&embed.stdout),
        String::from_utf8_lossy(&embed.stderr)
    );

    for mode in ["semantic", "hybrid"] {
        let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
            .args([
                "--data-dir",
                path_str(&data_dir),
                "search",
                "NeedleVectorQuery",
                "--mode",
                mode,
                "--embedding-command",
                path_str(&command),
                "--model-id",
                "fixture-section-model",
                "--dimension",
                "4",
                "--vector-top-k",
                "1",
                "--top-k",
                "1",
            ])
            .output()
            .expect("run semantic or hybrid section search");

        assert!(
            search.status.success(),
            "mode: {mode}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&search.stdout),
            String::from_utf8_lossy(&search.stderr)
        );
        assert!(search.stderr.is_empty());
        let stdout = String::from_utf8_lossy(&search.stdout);
        assert!(stdout.contains("results: 1"), "mode: {mode}\n{stdout}");
        assert!(
            stdout.contains("synthetic-section-vector.pdf"),
            "mode: {mode}\n{stdout}"
        );
        assert!(
            !stdout.contains("synthetic-document-vector.pdf"),
            "mode: {mode}\n{stdout}"
        );
        assert!(!stdout.contains("NeedleVectorQuery"));
        assert!(!stdout.contains(path_str(&data_dir)));
    }

    remove_dir(&data_dir);
}

fn import_fixtures(data_dir: &Path, fixture_root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(fixture_root),
        ])
        .output()
        .expect("import fixtures");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn seed_fulltext_index<const N: usize>(data_dir: &Path, documents: [IndexDocument; N]) {
    let index = FullTextIndex::open_or_create(&data_dir.join("search-index")).unwrap();
    index.replace_documents(documents).unwrap();
    index.commit().unwrap();
}

fn seed_searchable_metadata(
    data_dir: &Path,
    document_id: &DocumentId,
    version_id: &ResumeVersionId,
    file_name: &str,
    text: &str,
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_054_000);
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: format!("synthetic://{file_name}"),
            normalized_path: format!("synthetic/{file_name}"),
            file_name: file_name.to_string(),
            extension: FileExtension::Pdf,
            byte_size: 256,
            mtime: now,
            content_hash: Some(format!("{file_name}-hash")),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::Searchable,
        })
        .unwrap();
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id.clone(),
            document_id: document_id.clone(),
            candidate_id: None,
            parse_version: "parser-v1".to_string(),
            schema_version: "schema-v1".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: Some(text.to_string()),
            clean_text: Some(text.to_string()),
            quality_score: Some(0.9),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();
}

fn seed_document_metadata(
    data_dir: &Path,
    document_id: &DocumentId,
    file_name: &str,
    status: DocumentStatus,
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_054_000);
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: format!("synthetic://{file_name}"),
            normalized_path: format!("synthetic/{file_name}"),
            file_name: file_name.to_string(),
            extension: FileExtension::Pdf,
            byte_size: 256,
            mtime: now,
            content_hash: Some(format!("{file_name}-hash")),
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status,
        })
        .unwrap();
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes")
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s39-cli-{label}-{unique}"));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn temp_file(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s39-cli-{label}-{unique}.tmp"))
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn json_path(path: &Path) -> String {
    path_str(path).replace('\\', "\\\\").replace('"', "\\\"")
}

fn remove_dir(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

fn stdout_value(output: &str, prefix: &str) -> usize {
    output
        .lines()
        .find_map(|line| line.strip_prefix(prefix))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| panic!("missing numeric line with prefix {prefix:?} in:\n{output}"))
}

#[cfg(unix)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = temp_dir("embed-worker-command-bin");
    let path = directory.join(name);
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}
