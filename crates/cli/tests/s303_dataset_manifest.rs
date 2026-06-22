use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn privacy_dataset_manifest_writes_redacted_local_corpus_manifest_without_path_or_payload_leak() {
    let data_dir = temp_dir("dataset-manifest-data");
    let private_root = temp_dir("dataset-manifest-private-root");
    let out_dir = temp_dir("dataset-manifest-private-out");
    let nested = private_root.join("nested-private-folder");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        private_root.join("Alice Private Resume.pdf"),
        b"PRIVATE PDF RESUME PAYLOAD",
    )
    .unwrap();
    fs::write(
        nested.join("Bob Confidential Resume.docx"),
        b"PRIVATE DOCX RESUME PAYLOAD",
    )
    .unwrap();
    fs::write(
        private_root.join("Carol Secret Resume.doc"),
        b"PRIVATE DOC RESUME PAYLOAD",
    )
    .unwrap();
    fs::write(private_root.join("raw-private-photo.jpg"), b"PRIVATE JPG").unwrap();
    fs::write(private_root.join(".hidden-private.pdf"), b"PRIVATE HIDDEN").unwrap();
    let manifest_path = out_dir.join("dataset-manifest.json");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "privacy",
            "dataset-manifest",
            "--root",
            path_str(&private_root),
            "--out",
            path_str(&manifest_path),
            "--max-files",
            "10",
        ])
        .output()
        .expect("draft redacted dataset manifest");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("dataset manifest: written"));
    assert!(stdout.contains("schema: resume-ir.dataset-manifest.v1"));
    assert!(stdout.contains("privacy boundary: local_only_redacted_dataset_manifest"));
    assert!(stdout.contains("files: 3"));
    assert!(stdout.contains("manifest sha256: "));
    for forbidden in [
        path_str(&data_dir),
        path_str(&private_root),
        path_str(&out_dir),
        "Alice",
        "Bob",
        "Carol",
        "Private Resume",
        "Confidential Resume",
        "Secret Resume",
        "PRIVATE",
        "PAYLOAD",
    ] {
        assert!(!stdout.contains(forbidden), "stdout leaked {forbidden}");
    }

    let manifest = fs::read_to_string(&manifest_path).unwrap();
    let report: serde_json::Value = serde_json::from_str(&manifest).expect("dataset manifest JSON");
    assert_eq!(report["schema_version"], "resume-ir.dataset-manifest.v1");
    assert_eq!(
        report["privacy_boundary"],
        "local_only_redacted_dataset_manifest"
    );
    assert_eq!(report["dataset_kind"], "private-local-corpus");
    assert_eq!(report["scan_profile"], "explicit");
    assert_eq!(report["file_count"], 3);
    assert_eq!(report["extension_counts"]["pdf"], 1);
    assert_eq!(report["extension_counts"]["docx"], 1);
    assert_eq!(report["extension_counts"]["doc"], 1);
    assert_eq!(report["contains_paths"], false);
    assert_eq!(report["contains_file_names"], false);
    assert_eq!(report["contains_raw_resume_text"], false);
    assert_eq!(report["contains_file_hashes"], false);
    let corpus_fingerprint = report["corpus_fingerprint_sha256"]
        .as_str()
        .expect("corpus fingerprint");
    assert_eq!(corpus_fingerprint.len(), 64);
    assert!(corpus_fingerprint
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit()));
    assert!(report.get("files").is_none());
    assert!(report.get("paths").is_none());
    for forbidden in [
        path_str(&data_dir),
        path_str(&private_root),
        path_str(&out_dir),
        "Alice",
        "Bob",
        "Carol",
        "Private Resume",
        "Confidential Resume",
        "Secret Resume",
        "raw-private-photo",
        "PRIVATE",
        "PAYLOAD",
    ] {
        assert!(!manifest.contains(forbidden), "manifest leaked {forbidden}");
    }

    remove_dir(&data_dir);
    remove_dir(&private_root);
    remove_dir(&out_dir);
}

#[test]
fn privacy_dataset_manifest_rejects_missing_root_without_path_leak() {
    let data_dir = temp_dir("dataset-manifest-missing-data");
    let private_root = temp_dir("dataset-manifest-missing-private-root");
    let out_dir = temp_dir("dataset-manifest-missing-private-out");
    remove_dir(&private_root);
    let manifest_path = out_dir.join("dataset-manifest.json");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "privacy",
            "dataset-manifest",
            "--root",
            path_str(&private_root),
            "--out",
            path_str(&manifest_path),
        ])
        .output()
        .expect("reject missing dataset root");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("dataset manifest blocked: root must exist and be readable"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&private_root)));
    assert!(!stderr.contains(path_str(&out_dir)));
    assert!(!stderr.contains("PRIVATE"));
    assert!(!manifest_path.exists());

    remove_dir(&data_dir);
    remove_dir(&out_dir);
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s303-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
