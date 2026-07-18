use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::*;

const TEST_MODEL_ASSETS: [AssetIdentity; 5] = [
    AssetIdentity::new(
        FileRole::Model,
        20,
        "ba356b6e414089c6ba991270527766e0d42f90cc45cef5f01718f31106929a36",
    ),
    AssetIdentity::new(
        FileRole::Tokenizer,
        24,
        "6b81523b55b8799bc067f82addfe4c2348af2052ca6e85586a94b2e3ec1fdc50",
    ),
    AssetIdentity::new(
        FileRole::ModelConfig,
        21,
        "854ca27f5087e23ed3948b44cad54e666407f55628b0f5e0b3cec37c1371b1c5",
    ),
    AssetIdentity::new(
        FileRole::SpecialTokensMap,
        33,
        "29a5b72cd1e78dac7c2cd04cb328697fdae9f2a6ded0e6167c149f96923bbe9b",
    ),
    AssetIdentity::new(
        FileRole::TokenizerConfig,
        31,
        "4386608228199a4cadf868cf00cc52b8eecbc2bbbccaef15e84fec5c35f41be0",
    ),
];

#[test]
fn parses_bounded_embedding_request() {
    let body = format!(
        "{INPUT_SCHEMA}\nmodel_id={MODEL_ID}\ndimension=384\ncount=2\n\
         input=query\t5\ntext:\nhello\n{BOUNDARY}\n\
         input=doc-1\t5\ntext:\nworld\n{BOUNDARY}\n"
    );
    let request = parse_input(body.as_bytes()).unwrap();
    assert_eq!(request.model_id, MODEL_ID);
    assert_eq!(request.dimension, DIMENSION);
    assert_eq!(request.inputs.len(), 2);
    assert_eq!(request.inputs[0].id, "query");
    assert_eq!(request.inputs[0].text, "hello");
}

#[test]
fn rejects_declared_length_count_and_text_budget_mismatch() {
    let wrong_length = format!(
        "{INPUT_SCHEMA}\nmodel_id={MODEL_ID}\ndimension=384\ncount=1\n\
         input=query\t4\ntext:\nhello\n{BOUNDARY}\n"
    );
    assert!(matches!(
        parse_input(wrong_length.as_bytes()),
        Err(RuntimeError::InputInvalid)
    ));
    let wrong_count = format!(
        "{INPUT_SCHEMA}\nmodel_id={MODEL_ID}\ndimension=384\ncount=0\n\
         input=query\t5\ntext:\nhello\n{BOUNDARY}\n"
    );
    assert!(matches!(
        parse_input(wrong_count.as_bytes()),
        Err(RuntimeError::InputInvalid)
    ));
    let oversized = "x".repeat(MAX_TEXT_BYTES + 1);
    let oversized_body = format!(
        "{INPUT_SCHEMA}\nmodel_id={MODEL_ID}\ndimension=384\ncount=1\n\
         input=query\t{}\ntext:\n{oversized}\n{BOUNDARY}\n",
        oversized.len()
    );
    assert!(matches!(
        parse_input(oversized_body.as_bytes()),
        Err(RuntimeError::InputBudgetExceeded)
    ));
}

#[test]
fn parses_text_by_declared_bytes_without_treating_content_as_protocol() {
    let text = format!("候选人\r\n{BOUNDARY}\nlast\n");
    let body = format!(
        "{INPUT_SCHEMA}\r\nmodel_id={MODEL_ID}\r\ndimension=384\r\ncount=1\r\n\
         input=query\t{}\r\ntext:\r\n{text}\r\n{BOUNDARY}\r\n",
        text.len()
    );
    let request = parse_input(body.as_bytes()).unwrap();
    assert_eq!(request.inputs[0].text, text);
}

#[test]
fn runtime_pack_requires_exact_identity_digests() {
    let fixture = RuntimePackFixture::new();
    let pack =
        RuntimePack::load_with_expected_model_assets_for_test(fixture.root(), &TEST_MODEL_ASSETS)
            .unwrap();
    assert_eq!(pack.model_id(), MODEL_ID);
    assert_eq!(pack.file_count(), 6);

    fs::write(fixture.root().join("model.onnx"), b"changed").unwrap();
    fixture.write_manifest(None);
    assert!(matches!(
        RuntimePack::load_with_expected_model_assets_for_test(fixture.root(), &TEST_MODEL_ASSETS),
        Err(RuntimeError::RuntimePackInvalid)
    ));
}

#[test]
fn runtime_pack_rejects_unknown_fields_and_path_escape() {
    let fixture = RuntimePackFixture::new();
    let manifest = fs::read_to_string(fixture.root().join("runtime-pack.json")).unwrap();
    let unknown = manifest.replacen(
        "\"schema_version\"",
        "\"unexpected\":true,\"schema_version\"",
        1,
    );
    fs::write(fixture.root().join("runtime-pack.json"), unknown).unwrap();
    assert!(matches!(
        RuntimePack::load_with_expected_model_assets_for_test(fixture.root(), &TEST_MODEL_ASSETS),
        Err(RuntimeError::RuntimePackInvalid)
    ));

    fixture.write_manifest(None);
    let manifest = fs::read_to_string(fixture.root().join("runtime-pack.json")).unwrap();
    fs::write(
        fixture.root().join("runtime-pack.json"),
        manifest.replacen("\"file\": \"model.onnx\"", "\"file\": \"../model.onnx\"", 1),
    )
    .unwrap();
    assert!(matches!(
        RuntimePack::load_with_expected_model_assets_for_test(fixture.root(), &TEST_MODEL_ASSETS),
        Err(RuntimeError::RuntimePackInvalid)
    ));
}

#[test]
fn runtime_panic_boundary_fails_closed_with_generic_error() {
    let error = run_with_panic_boundary(|| panic!("synthetic runtime panic")).unwrap_err();
    assert!(matches!(error, RuntimeError::RuntimeUnavailable));
    assert_eq!(error.to_string(), "ONNX runtime is unavailable");
}

#[test]
fn invalid_runtime_library_failure_is_bounded() {
    let mut probe = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::invalid_runtime_library_failure_probe",
            "--ignored",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        if let Some(status) = probe.try_wait().unwrap() {
            assert!(
                status.success(),
                "runtime failure probe exited with {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            probe.kill().unwrap();
            probe.wait().unwrap();
            panic!("invalid ONNX runtime library did not fail within two seconds");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "invoked in a supervised child process by invalid_runtime_library_failure_is_bounded"]
fn invalid_runtime_library_failure_probe() {
    let fixture = RuntimePackFixture::new();
    let pack =
        RuntimePack::load_with_expected_model_assets_for_test(fixture.root(), &TEST_MODEL_ASSETS)
            .unwrap();
    assert!(matches!(
        initialize_model(&pack, /*intra_threads*/ 1),
        Err(RuntimeError::RuntimeUnavailable)
    ));
}

#[cfg(unix)]
#[test]
fn runtime_pack_rejects_symlinked_components() {
    use std::os::unix::fs::symlink;

    let fixture = RuntimePackFixture::new();
    let outside = fixture.root().parent().unwrap().join("outside-model.onnx");
    fs::rename(fixture.root().join("model.onnx"), &outside).unwrap();
    symlink(&outside, fixture.root().join("model.onnx")).unwrap();
    assert!(matches!(
        RuntimePack::load_with_expected_model_assets_for_test(fixture.root(), &TEST_MODEL_ASSETS),
        Err(RuntimeError::RuntimePackInvalid)
    ));
}

#[test]
fn prefixes_query_and_passage_inputs_explicitly() {
    let query = EmbeddingRuntimeInput {
        id: "query".to_string(),
        text: "synthetic query".to_string(),
    };
    let passage = EmbeddingRuntimeInput {
        id: "document".to_string(),
        text: "synthetic passage".to_string(),
    };
    assert_eq!(prefixed_text(&query), "query: synthetic query");
    assert_eq!(prefixed_text(&passage), "passage: synthetic passage");
    assert_eq!(
        prefixed_resident_text(EmbeddingRole::Query, "synthetic query"),
        "query: synthetic query"
    );
    assert_eq!(
        prefixed_resident_text(EmbeddingRole::Passage, "synthetic passage"),
        "passage: synthetic passage"
    );
}

#[test]
fn mean_pool_applies_attention_mask_and_validates_shape() {
    let mut values = vec![1.0; DIMENSION];
    values.extend(vec![20.0; DIMENSION]);
    values.extend(vec![5.0; DIMENSION]);
    assert_eq!(
        mean_pool(&[1, 3, DIMENSION as i64], &values, &[1, 0, 1]).unwrap(),
        vec![3.0; DIMENSION]
    );
    let pooled = vec![0.25; DIMENSION];
    assert_eq!(
        mean_pool(&[1, DIMENSION as i64], &pooled, &[1]).unwrap(),
        pooled
    );
    assert!(matches!(
        mean_pool(&[1, 2, DIMENSION as i64], &values[..2 * DIMENSION], &[0, 0]),
        Err(RuntimeError::OutputInvalid)
    ));
}

#[test]
fn output_rejects_non_finite_and_normalizes_vectors() {
    let request = EmbeddingRequest {
        model_id: MODEL_ID.to_string(),
        dimension: 2,
        inputs: vec![EmbeddingRuntimeInput {
            id: "query".to_string(),
            text: "<redacted>".to_string(),
        }],
    };
    let output = format_output(&request, vec![vec![3.0, 4.0]]).unwrap();
    assert_eq!(
        output,
        format!(
            "{OUTPUT_SCHEMA}\nmodel_id={MODEL_ID}\ndimension=2\nvector=query\t0.600000024,0.800000012\n"
        )
    );
    assert!(matches!(
        format_output(&request, vec![vec![f32::NAN, 1.0]]),
        Err(RuntimeError::OutputInvalid)
    ));
}

struct RuntimePackFixture {
    directory: TempDir,
}

impl RuntimePackFixture {
    fn new() -> Self {
        let fixture = Self {
            directory: tempfile::tempdir().unwrap(),
        };
        for file in [
            "libonnxruntime.dylib",
            "model.onnx",
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ] {
            fs::write(fixture.root().join(file), format!("synthetic-{file}")).unwrap();
        }
        fixture.write_manifest(None);
        fixture
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }

    fn write_manifest(&self, model_override: Option<&str>) {
        let files = [
            ("runtime_library", "libonnxruntime.dylib"),
            ("model", model_override.unwrap_or("model.onnx")),
            ("tokenizer", "tokenizer.json"),
            ("model_config", "config.json"),
            ("special_tokens_map", "special_tokens_map.json"),
            ("tokenizer_config", "tokenizer_config.json"),
        ];
        let entries = files
            .iter()
            .map(|(role, file)| {
                let path = self.root().join(file);
                let bytes = fs::read(&path).unwrap_or_default();
                let digest = Sha256::digest(&bytes);
                serde_json::json!({
                    "role": role,
                    "file": file,
                    "bytes": bytes.len(),
                    "sha256": format!("{digest:x}"),
                })
            })
            .collect::<Vec<_>>();
        let manifest = serde_json::json!({
            "schema_version": PACK_SCHEMA,
            "runtime_pack_id": PACK_ID,
            "model_id": MODEL_ID,
            "upstream_model_id": UPSTREAM_MODEL_ID,
            "upstream_revision": UPSTREAM_REVISION,
            "upstream_model_file": "onnx/model_qint8_avx512_vnni.onnx",
            "quantization": "dynamic_int8",
            "dimension": DIMENSION,
            "provider": "cpu",
            "network_access": "disabled",
            "license_reviewed": true,
            "model_license": "MIT",
            "onnxruntime_license": "MIT",
            "files": entries,
        });
        fs::write(
            self.root().join("runtime-pack.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }
}
