use std::{fs, path::Path};

use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::*;

fn fixture(variant: &str, model_id: &str, quantization: &str) -> (TempDir, PathBuf) {
    let root = TempDir::new().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let model = b"synthetic onnx model";
    fs::write(root.path().join("model.onnx"), model).unwrap();
    let digest = format!("{:x}", Sha256::digest(model));
    let manifest = format!(
        "{{\"schema_version\":\"{MANIFEST_SCHEMA}\",\"variant\":\"{variant}\",\"model_id\":\"{model_id}\",\"upstream_model_id\":\"{UPSTREAM_MODEL_ID}\",\"upstream_revision\":\"{UPSTREAM_REVISION}\",\"dimension\":{DIMENSION},\"generator_onnx_version\":\"1.19.0\",\"generator_ort_version\":\"1.27.0\",\"quantization\":{quantization},\"model\":{{\"file\":\"model.onnx\",\"bytes\":{},\"sha256\":\"{digest}\"}}}}",
        model.len()
    );
    let path = root.path().join("experiment.json");
    fs::write(&path, manifest).unwrap();
    (root, path)
}

fn fp32_fixture() -> (TempDir, PathBuf) {
    fixture(
        "fp32",
        "intfloat-multilingual-e5-small-fp32-exp-r1",
        "{\"method\":\"none\",\"format\":\"fp32\",\"activation_type\":\"float32\",\"weight_type\":\"float32\",\"calibration_id\":null,\"op_types\":[\"MatMul\"],\"graph_optimization\":\"disabled\"}",
    )
}

#[test]
fn accepts_exact_fp32_manifest_in_owner_only_directory() {
    let (_root, path) = fp32_fixture();
    let artifact = ArtifactExperiment::load(&path).unwrap();
    assert_eq!(
        artifact.model_id(),
        "intfloat-multilingual-e5-small-fp32-exp-r1"
    );
    assert_eq!(artifact.dimension(), DIMENSION);
    assert_eq!(artifact.model_path().file_name().unwrap(), "model.onnx");
}

#[test]
fn accepts_both_pre_registered_static_variants() {
    for (variant, model_id, format, activation) in [
        (
            "static_qdq_s8s8",
            "intfloat-multilingual-e5-small-static-qdq-s8s8-exp-r1",
            "qdq",
            "qint8",
        ),
        (
            "static_qoperator_u8s8",
            "intfloat-multilingual-e5-small-static-qoperator-u8s8-exp-r1",
            "qoperator",
            "quint8",
        ),
    ] {
        let quantization = format!("{{\"method\":\"static\",\"format\":\"{format}\",\"activation_type\":\"{activation}\",\"weight_type\":\"qint8\",\"calibration_id\":\"{CALIBRATION_ID}\",\"op_types\":[\"MatMul\"],\"graph_optimization\":\"disabled\"}}");
        let (_root, path) = fixture(variant, model_id, &quantization);
        assert!(ArtifactExperiment::load(&path).is_ok());
    }
}

#[test]
fn rejects_identity_digest_and_unknown_field_drift() {
    let (_root, path) = fp32_fixture();
    let original = fs::read_to_string(&path).unwrap();
    for changed in [
        original.replace("fp32-exp-r1", "wrong-exp-r1"),
        original.replace("1.27.0", "1.26.0"),
        original.replace("\"bytes\":20", "\"bytes\":19"),
        original.replacen("{", "{\"unknown\":true,", 1),
    ] {
        fs::write(&path, changed).unwrap();
        assert!(matches!(
            ArtifactExperiment::load(&path),
            Err(RuntimeError::RuntimePackInvalid)
        ));
    }
}

#[cfg(unix)]
#[test]
fn rejects_non_owner_only_parent_and_symlinked_model() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    let (root, path) = fp32_fixture();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(ArtifactExperiment::load(&path).is_err());

    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let target = root.path().join("target.onnx");
    fs::rename(root.path().join("model.onnx"), &target).unwrap();
    symlink(&target, root.path().join("model.onnx")).unwrap();
    assert!(ArtifactExperiment::load(&path).is_err());
}

#[test]
fn rejects_relative_manifest_path() {
    let (_root, path) = fp32_fixture();
    assert!(ArtifactExperiment::load(Path::new("experiment.json")).is_err());
    assert!(path.is_absolute());
}

#[test]
fn environment_value_gate_accepts_only_bounded_absolute_utf8_paths() {
    let (_root, path) = fp32_fixture();
    assert_eq!(
        ArtifactExperiment::load_environment_value(path.as_os_str())
            .unwrap()
            .model_path(),
        path.parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .join("model.onnx")
    );
    for value in [
        "experiment.json".to_string(),
        format!("/{}", "x".repeat(MAX_RUNTIME_PATH_BYTES)),
    ] {
        assert!(matches!(
            ArtifactExperiment::load_environment_value(value.as_ref()),
            Err(RuntimeError::EnvironmentInvalid)
        ));
    }
    #[cfg(unix)]
    {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};
        assert!(matches!(
            ArtifactExperiment::load_environment_value(&OsString::from_vec(vec![0xff])),
            Err(RuntimeError::EnvironmentInvalid)
        ));
    }
}
