use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn local_model_promotes_safe_gray_and_missing_model_fails_closed() {
    let root = temp_dir("source");
    fs::write(
        root.join("synthetic-profile.txt"),
        "PROFILE\nPlatform engineer with Rust and distributed systems experience.\nINVOICE",
    )
    .unwrap();
    let artifact = root.parent().unwrap().join("synthetic-model.json");
    write_synthetic_artifact(&artifact);

    let data = temp_dir("promotion-data");
    let missing = run_import(&data, &root, &artifact.with_extension("missing"));
    assert!(
        missing.status.success(),
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );
    assert!(String::from_utf8_lossy(&missing.stdout).contains("searchable documents: 0"));
    assert!(String::from_utf8_lossy(&missing.stdout)
        .contains("resume classifier promotion: fail_closed_disabled"));

    let enabled = run_import(&data, &root, &artifact);
    assert!(
        enabled.status.success(),
        "{}",
        String::from_utf8_lossy(&enabled.stderr)
    );
    assert!(String::from_utf8_lossy(&enabled.stdout).contains("searchable documents: 1"));
    assert!(
        String::from_utf8_lossy(&enabled.stdout).contains("resume classifier promotion: enabled")
    );

    let removed = run_import(&data, &root, &artifact.with_extension("missing"));
    assert!(
        removed.status.success(),
        "{}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(String::from_utf8_lossy(&removed.stdout).contains("searchable documents: 0"));

    remove_temp(&data);
    remove_temp(&root);
    let _ = fs::remove_file(artifact);
}

fn run_import(data: &Path, root: &Path, artifact: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data),
            "import",
            "--root",
            path_str(root),
            "--resume-classifier-model",
            path_str(artifact),
        ])
        .output()
        .expect("run resume-cli import")
}

fn write_synthetic_artifact(path: &Path) {
    let model = json!({
        "schema": "resume_ir_linear_promotion_v1",
        "classifier_epoch": "precision_first_v4",
        "feature_contract": "bounded_normalized_text_plus_structure_v1",
        "max_input_chars": 128,
        "threshold": 0.7,
        "intercept": 0.0,
        "features": [{"ngram": "pla", "idf": 1.0, "coefficient": 1.0}]
    });
    let model_json = serde_json::to_string(&model).unwrap();
    let digest = format!("{:x}", Sha256::digest(model_json.as_bytes()));
    fs::write(
        path,
        serde_json::to_vec(&json!({"model_json": model_json, "model_sha256": digest})).unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s173-{label}-{nonce}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn remove_temp(path: &Path) {
    match fs::remove_dir_all(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove temp dir: {error}"),
    }
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}
