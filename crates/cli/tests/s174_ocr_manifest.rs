use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn ocr_manifest_validate_accepts_reviewed_local_runtime_without_path_or_payload_leak() {
    let data_dir = temp_dir("ocr-manifest-valid-private-data");
    let tesseract_file = temp_file("ocr-manifest-valid-private-tesseract");
    let pdftoppm_file = temp_file("ocr-manifest-valid-private-pdftoppm");
    let manifest_file = temp_file("ocr-manifest-valid-private-manifest");
    let tesseract_bytes = b"SYNTHETIC TESSERACT RUNTIME\n";
    let pdftoppm_bytes = b"SYNTHETIC PDFTOPPM RUNTIME\n";
    fs::write(&tesseract_file, tesseract_bytes).unwrap();
    fs::write(&pdftoppm_file, pdftoppm_bytes).unwrap();
    fs::write(
        &manifest_file,
        format!(
            r#"{{
  "schema_version": "resume-ir.ocr-runtime-manifest.v1",
  "runtime_pack_id": "fixture-ocr-pack-reviewed",
  "components": [
    {{
      "id": "fixture-tesseract",
      "kind": "ocr-engine",
      "engine": "tesseract",
      "version": "5.5.1",
      "artifact": {{
        "path": "{}",
        "sha256": "f4c4eb4c45e595f803f076791dd942e6fd8bb93076207f8830ed6b8694f11e4a"
      }},
      "license": {{
        "id": "Apache-2.0",
        "reviewed": true
      }}
    }},
    {{
      "id": "fixture-pdftoppm",
      "kind": "pdf-renderer",
      "engine": "poppler-pdftoppm",
      "version": "25.12.0",
      "artifact": {{
        "path": "{}",
        "sha256": "571699d70504c3e505293c25953a85c38bdc8c13681aed7f7e3c4ce77fc8245f"
      }},
      "license": {{
        "id": "GPL-2.0-or-later",
        "reviewed": true
      }}
    }}
  ],
  "languages": [
    {{
      "id": "eng",
      "artifact": {{
        "path": "{}",
        "sha256": "f4c4eb4c45e595f803f076791dd942e6fd8bb93076207f8830ed6b8694f11e4a"
      }},
      "license": {{
        "id": "Apache-2.0",
        "reviewed": true
      }}
    }}
  ]
}}"#,
            json_path(&tesseract_file),
            json_path(&pdftoppm_file),
            json_path(&tesseract_file)
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "ocr",
            "validate-manifest",
            "--manifest",
            path_str(&manifest_file),
        ])
        .output()
        .expect("validate reviewed OCR manifest");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("ocr runtime manifest: valid"));
    assert!(stdout.contains("runtime pack: fixture-ocr-pack-reviewed"));
    assert!(stdout.contains("components: 2"));
    assert!(stdout.contains("component id: fixture-tesseract"));
    assert!(stdout.contains("kind: ocr-engine"));
    assert!(stdout.contains("engine: tesseract"));
    assert!(stdout.contains("component id: fixture-pdftoppm"));
    assert!(stdout.contains("kind: pdf-renderer"));
    assert!(stdout.contains("engine: poppler-pdftoppm"));
    assert!(stdout.contains("languages: 1"));
    assert!(stdout.contains("language id: eng"));
    assert!(stdout.contains("license reviewed: yes"));
    assert!(stdout.contains("checksum match: yes"));
    assert!(stdout.contains("paths: <redacted>"));
    assert!(!stdout.contains("SYNTHETIC TESSERACT RUNTIME"));
    assert!(!stdout.contains("SYNTHETIC PDFTOPPM RUNTIME"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&tesseract_file)));
    assert!(!stdout.contains(path_str(&pdftoppm_file)));
    assert!(!stdout.contains(path_str(&manifest_file)));

    remove_dir(&data_dir);
    remove_file(&tesseract_file);
    remove_file(&pdftoppm_file);
    remove_file(&manifest_file);
}

#[test]
fn ocr_manifest_validate_rejects_checksum_mismatch_without_path_or_payload_leak() {
    let data_dir = temp_dir("ocr-manifest-mismatch-private-data");
    let runtime_file = temp_file("ocr-manifest-mismatch-private-runtime");
    let manifest_file = temp_file("ocr-manifest-mismatch-private-manifest");
    fs::write(&runtime_file, b"SYNTHETIC MISMATCH OCR RUNTIME\n").unwrap();
    fs::write(
        &manifest_file,
        format!(
            r#"{{
  "schema_version": "resume-ir.ocr-runtime-manifest.v1",
  "runtime_pack_id": "fixture-ocr-pack-mismatch",
  "components": [
    {{
      "id": "fixture-tesseract",
      "kind": "ocr-engine",
      "engine": "tesseract",
      "version": "5.5.1",
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
            json_path(&runtime_file)
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "ocr",
            "validate-manifest",
            "--manifest",
            path_str(&manifest_file),
        ])
        .output()
        .expect("reject checksum mismatch OCR manifest");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ocr runtime manifest blocked: checksum mismatch"));
    assert!(!stderr.contains("SYNTHETIC MISMATCH OCR RUNTIME"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&runtime_file)));
    assert!(!stderr.contains(path_str(&manifest_file)));

    remove_dir(&data_dir);
    remove_file(&runtime_file);
    remove_file(&manifest_file);
}

#[test]
fn ocr_manifest_validate_rejects_unreviewed_license_without_path_or_payload_leak() {
    let data_dir = temp_dir("ocr-manifest-unreviewed-private-data");
    let runtime_file = temp_file("ocr-manifest-unreviewed-private-runtime");
    let manifest_file = temp_file("ocr-manifest-unreviewed-private-manifest");
    fs::write(&runtime_file, b"SYNTHETIC UNREVIEWED OCR RUNTIME\n").unwrap();
    fs::write(
        &manifest_file,
        format!(
            r#"{{
  "schema_version": "resume-ir.ocr-runtime-manifest.v1",
  "runtime_pack_id": "fixture-ocr-pack-unreviewed",
  "components": [
    {{
      "id": "fixture-tesseract",
      "kind": "ocr-engine",
      "engine": "tesseract",
      "version": "5.5.1",
      "artifact": {{
        "path": "{}",
        "sha256": "2cdb7f5b2d08814f424ca66697dc66ca8b9aa7736a3ee222fab373146923f138"
      }},
      "license": {{
        "id": "Proprietary",
        "reviewed": false
      }}
    }}
  ]
}}"#,
            json_path(&runtime_file)
        ),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "ocr",
            "validate-manifest",
            "--manifest",
            path_str(&manifest_file),
        ])
        .output()
        .expect("reject unreviewed OCR manifest");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ocr runtime manifest blocked: license has not been reviewed"));
    assert!(!stderr.contains("SYNTHETIC UNREVIEWED OCR RUNTIME"));
    assert!(!stderr.contains(path_str(&data_dir)));
    assert!(!stderr.contains(path_str(&runtime_file)));
    assert!(!stderr.contains(path_str(&manifest_file)));

    remove_dir(&data_dir);
    remove_file(&runtime_file);
    remove_file(&manifest_file);
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s174-cli-{label}-{unique}"))
}

fn temp_file(label: &str) -> PathBuf {
    let path = temp_path(label);
    remove_file(&path);
    path
}

fn temp_dir(label: &str) -> PathBuf {
    let path = temp_path(label);
    remove_dir(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().expect("test paths are utf-8")
}

fn json_path(path: &Path) -> String {
    path_str(path).replace('\\', "\\\\").replace('"', "\\\"")
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn remove_file(path: &Path) {
    let _ = fs::remove_file(path);
}
