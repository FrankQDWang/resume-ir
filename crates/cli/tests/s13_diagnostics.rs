use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use index_fulltext::{publish_snapshot, IndexDocument, IndexSection};
use index_vector::{PersistentVectorIndex, VectorDocument, VectorIndex};
use meta_store::{IndexState, IndexStateStatus, MetaStore, UnixTimestamp};

#[test]
fn doctor_reports_no_index_without_path_or_fake_benchmark() {
    let data_dir = temp_path("doctor-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-ir doctor"));
    assert!(stdout.contains("metadata: ok"));
    assert!(stdout.contains("search index: unavailable"));
    assert!(stdout.contains("vector index: unavailable"));
    assert!(stdout.contains("query smoke: skipped (no full-text index)"));
    assert!(stdout.contains("contact hash key: missing"));
    assert!(stdout.contains("fault simulations: available"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!data_dir
        .join("secrets")
        .join("contact-hash-key-v1")
        .exists());
    assert!(!stdout.contains("p95"));

    remove_dir(&data_dir);
}

#[test]
fn doctor_handles_corrupt_index_snapshot_without_path_leak() {
    let data_dir = temp_dir("doctor-corrupt-private-data");
    let index_dir = data_dir.join("search-index");
    fs::create_dir_all(&index_dir).unwrap();
    fs::write(index_dir.join("meta.json"), b"not a tantivy index").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with corrupt index");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("search index: corrupt"));
    assert!(stdout.contains("query smoke: skipped (index unavailable)"));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[test]
fn export_diagnostics_redact_outputs_skeleton_without_paths() {
    let data_dir = temp_path("diagnostics-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"schema_version\": \"diagnostics.v1\""));
    assert!(stdout.contains("\"redacted\": true"));
    assert!(stdout.contains("\"raw_paths\": \"<redacted>\""));
    assert!(stdout.contains("\"search_index_state\": \"unavailable\""));
    assert!(stdout.contains("\"vector_index_state\": \"unavailable\""));
    assert!(stdout.contains("\"contact_hash_key\": \"missing\""));
    assert!(stdout.contains("\"daemon_restart\""));
    assert!(stdout.contains("\"daemon_kill\""));
    assert!(stdout.contains("\"disk_space_low\""));
    assert!(stdout.contains("\"file_lock\""));
    assert!(stdout.contains("\"ocr_crash\""));
    assert!(stdout.contains("\"model_checksum\""));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!data_dir
        .join("secrets")
        .join("contact-hash-key-v1")
        .exists());

    remove_dir(&data_dir);
}

#[test]
fn doctor_and_diagnostics_report_redacted_resource_telemetry() {
    let data_dir = temp_dir("diagnostics-resource-private-data");

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with resource telemetry");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("resource telemetry: available"));
    assert!(stdout.contains("data disk total bytes: "));
    assert!(stdout.contains("data disk available bytes: "));
    assert!(stdout.contains("process memory bytes: "));
    assert!(stdout.contains("cpu cores: "));
    assert!(!stdout.contains(path_str(&data_dir)));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with resource telemetry");
    assert!(export.status.success());
    assert!(export.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"resource_telemetry\": {"));
    assert!(stdout.contains("\"status\": \"available\""));
    assert!(stdout.contains("\"paths\": \"<redacted>\""));
    assert!(stdout.contains("\"data_disk_total_bytes\": "));
    assert!(stdout.contains("\"data_disk_available_bytes\": "));
    assert!(stdout.contains("\"process_memory_bytes\": "));
    assert!(stdout.contains("\"cpu_cores\": "));
    assert!(!stdout.contains(path_str(&data_dir)));
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let telemetry = json["resource_telemetry"].as_object().unwrap();
    assert_eq!(telemetry["status"], "available");
    assert_eq!(telemetry["paths"], "<redacted>");
    assert!(telemetry["data_disk_total_bytes"].as_u64().unwrap() > 0);
    assert!(telemetry["data_disk_available_bytes"].as_u64().unwrap() > 0);
    assert!(telemetry["process_memory_bytes"].as_u64().unwrap() > 0);
    assert!(telemetry["cpu_cores"].as_u64().unwrap() > 0);

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn doctor_and_diagnostics_report_ocr_runtime_without_paths_or_language_dump() {
    let data_dir = temp_dir("diagnostics-ocr-runtime-private-data");
    let bin_dir = temp_dir("diagnostics-ocr-runtime-private-bin");
    write_executable(&bin_dir, "pdftoppm", "#!/bin/sh\nexit 0\n");
    write_executable(
        &bin_dir,
        "tesseract",
        r#"#!/bin/sh
if [ "$1" = "--list-langs" ]; then
  printf 'List of available languages (2):\n'
  printf 'eng\n'
  printf 'chi_sim\n'
  exit 0
fi
exit 9
"#,
    );

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("PATH", path_str(&bin_dir))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with OCR runtime diagnostics");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("ocr renderer pdftoppm: available"));
    assert!(stdout.contains("ocr engine tesseract: available"));
    assert!(stdout.contains("ocr language eng: available"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&bin_dir)));
    assert!(!stdout.contains("chi_sim"));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("PATH", path_str(&bin_dir))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with OCR runtime diagnostics");
    assert!(export.status.success());
    assert!(export.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"ocr_runtime\": {"));
    assert!(stdout.contains("\"pdftoppm\": \"available\""));
    assert!(stdout.contains("\"tesseract\": \"available\""));
    assert!(stdout.contains("\"tesseract_eng\": \"available\""));
    assert!(stdout.contains("\"paths\": \"<redacted>\""));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&bin_dir)));
    assert!(!stdout.contains("chi_sim"));

    remove_dir(&data_dir);
    remove_dir(&bin_dir);
}

#[cfg(unix)]
#[test]
fn doctor_reports_non_executable_ocr_tools_as_missing_without_paths() {
    let data_dir = temp_dir("diagnostics-ocr-runtime-nonexec-data");
    let bin_dir = temp_dir("diagnostics-ocr-runtime-nonexec-bin");
    write_private_file(&bin_dir, "pdftoppm", "#!/bin/sh\nexit 0\n");

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("PATH", path_str(&bin_dir))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with non-executable OCR runtime");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("ocr renderer pdftoppm: missing"));
    assert!(stdout.contains("ocr engine tesseract: missing"));
    assert!(stdout.contains("ocr language eng: missing"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&bin_dir)));

    remove_dir(&data_dir);
    remove_dir(&bin_dir);
}

#[test]
fn doctor_and_diagnostics_report_persistent_vector_snapshot_without_path_or_values() {
    let data_dir = temp_dir("diagnostics-vector-private-data");
    let vector_index = PersistentVectorIndex::open(data_dir.join("vector-index"), 4).unwrap();
    vector_index
        .upsert(vec![
            VectorDocument::new("vec_java", "doc_java", vec![1.0, 0.0, 0.0, 0.0]).unwrap(),
            VectorDocument::new("vec_data", "doc_data", vec![0.0, 1.0, 0.0, 0.0]).unwrap(),
        ])
        .unwrap();
    vector_index.mark_deleted(&["vec_data"]).unwrap();

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with vector index");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("vector index: available (vector snapshot)"));
    assert!(stdout.contains("vector index vectors: 2"));
    assert!(stdout.contains("vector index tombstones: 1"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains("1.0"));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with vector index");
    assert!(export.status.success());
    assert!(export.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"vector_index_state\": \"available\""));
    assert!(stdout.contains("\"vector_index_vectors\": 2"));
    assert!(stdout.contains("\"vector_index_tombstones\": 1"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains("1.0"));

    remove_dir(&data_dir);
}

#[test]
fn doctor_and_diagnostics_report_invalid_contact_hash_key_without_leaks() {
    let data_dir = temp_dir("diagnostics-invalid-key");
    let key_path = data_dir.join("secrets").join("contact-hash-key-v1");
    fs::create_dir_all(key_path.parent().unwrap()).unwrap();
    fs::write(&key_path, "not-a-real-contact-key\n").unwrap();

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with invalid contact key");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("contact hash key: invalid"));
    assert!(!stdout.contains("not-a-real-contact-key"));
    assert!(!stdout.contains(path_str(&data_dir)));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with invalid contact key");
    assert!(export.status.success());
    assert!(export.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"contact_hash_key\": \"invalid\""));
    assert!(!stdout.contains("not-a-real-contact-key"));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn doctor_reports_unreadable_contact_hash_key_without_leaks() {
    use std::os::unix::fs::PermissionsExt;

    let data_dir = temp_dir("diagnostics-unreadable-key");
    let secrets_dir = data_dir.join("secrets");
    let key_path = secrets_dir.join("contact-hash-key-v1");
    fs::create_dir_all(&secrets_dir).unwrap();
    fs::write(&key_path, format!("{}\n", "c".repeat(64))).unwrap();
    fs::set_permissions(&secrets_dir, fs::Permissions::from_mode(0o000)).unwrap();

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with unreadable contact key");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("contact hash key: unreadable"));
    assert!(!stdout.contains("c".repeat(64).as_str()));
    assert!(!stdout.contains(path_str(&data_dir)));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with unreadable contact key");
    assert!(export.status.success());
    assert!(export.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"contact_hash_key\": \"unreadable\""));
    assert!(!stdout.contains("c".repeat(64).as_str()));
    assert!(!stdout.contains(path_str(&data_dir)));

    fs::set_permissions(&secrets_dir, fs::Permissions::from_mode(0o700)).unwrap();
    remove_dir(&data_dir);
}

#[test]
fn doctor_and_diagnostics_report_metadata_index_health_with_active_snapshot() {
    let data_dir = temp_dir("diagnostics-index-health");
    publish_snapshot(
        &data_dir.join("search-index"),
        "fulltext-1800002000-1-0-0",
        [IndexDocument {
            doc_id: "doc_diagnostic".to_string(),
            version_id: "ver_diagnostic".to_string(),
            file_name: "synthetic-diagnostic.pdf".to_string(),
            clean_text: "diagnostic Java search text".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "diagnostic Java".to_string(),
            }],
            is_deleted: false,
        }],
    )
    .unwrap();
    fs::create_dir_all(data_dir.join("search-index").join("staging").join("orphan")).unwrap();

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_index_state(&IndexState {
            manifest_version: "fulltext-s25-test".to_string(),
            snapshot_token: Some("fulltext-1800002000-1-0-0".to_string()),
            status: IndexStateStatus::Stale,
            updated_at: UnixTimestamp::from_unix_seconds(1_800_002_000),
        })
        .unwrap();

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with active snapshot");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("search index: available (full-text snapshot)"));
    assert!(stdout.contains("index health: stale"));
    assert!(stdout.contains("last snapshot: present"));
    assert!(stdout.contains("staging orphans: 1"));
    assert!(!stdout.contains(path_str(&data_dir)));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with active snapshot");
    assert!(export.status.success());
    assert!(export.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"search_index_state\": \"available\""));
    assert!(stdout.contains("\"search_index_read_target\": \"published_snapshot\""));
    assert!(stdout.contains("\"index_health\": \"stale\""));
    assert!(stdout.contains("\"last_snapshot\": \"present\""));
    assert!(stdout.contains("\"staging_orphans\": 1"));
    assert!(!stdout.contains("fulltext-1800002000-1-0-0"));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[test]
fn doctor_and_search_use_last_good_snapshot_after_active_snapshot_corruption() {
    let data_dir = temp_dir("diagnostics-snapshot-recovered");
    let index_root = data_dir.join("search-index");
    let (recovered_doc_id, recovered_version_id) = seed_searchable_metadata(&data_dir);
    publish_snapshot(
        &index_root,
        "fulltext-1800003000-1-0-0",
        [IndexDocument {
            doc_id: recovered_doc_id.clone(),
            version_id: recovered_version_id,
            file_name: "synthetic-recovered.pdf".to_string(),
            clean_text: "diagnostic recovered Java snapshot".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "diagnostic recovered Java".to_string(),
            }],
            is_deleted: false,
        }],
    )
    .unwrap();
    publish_snapshot(
        &index_root,
        "fulltext-1800004000-1-0-0",
        [IndexDocument {
            doc_id: "doc_corrupt_active".to_string(),
            version_id: "ver_corrupt_active".to_string(),
            file_name: "synthetic-corrupt-active.pdf".to_string(),
            clean_text: "diagnostic corrupt active Rust snapshot".to_string(),
            sections: vec![IndexSection {
                section_type: "experience".to_string(),
                text: "diagnostic corrupt active Rust".to_string(),
            }],
            is_deleted: false,
        }],
    )
    .unwrap();
    fs::write(
        index_root
            .join("snapshots")
            .join("fulltext-1800004000-1-0-0")
            .join("meta.json"),
        b"not a valid active snapshot",
    )
    .unwrap();

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search with recovered snapshot");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&search.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains(&format!("doc_id: {recovered_doc_id}")));
    assert!(!stdout.contains("doc_corrupt_active"));
    assert!(!stdout.contains(path_str(&data_dir)));

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with recovered snapshot");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("search index: recovered (full-text snapshot)"));
    assert!(stdout.contains("search index read target: published_snapshot"));
    assert!(stdout.contains("snapshot fallback: used"));
    assert!(stdout.contains("query smoke: ok"));
    assert!(!stdout.contains("fulltext-1800004000-1-0-0"));
    assert!(!stdout.contains("fulltext-1800003000-1-0-0"));
    assert!(!stdout.contains(path_str(&data_dir)));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with recovered snapshot");
    assert!(export.status.success());
    assert!(export.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"search_index_state\": \"recovered\""));
    assert!(stdout.contains("\"snapshot_fallback\": \"used\""));
    assert!(!stdout.contains("fulltext-1800004000-1-0-0"));
    assert!(!stdout.contains("fulltext-1800003000-1-0-0"));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

fn seed_searchable_metadata(data_dir: &Path) -> (String, String) {
    use meta_store::{
        Document, DocumentId, DocumentStatus, FileExtension, ResumeVersion, ResumeVersionId,
        ResumeVisibility,
    };

    let now = UnixTimestamp::from_unix_seconds(1_800_003_000);
    let document_id = DocumentId::from_non_secret_parts(&["s26", "recovered"]);
    let version_id = ResumeVersionId::from_non_secret_parts(&["s26", "recovered-version"]);
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_document(&Document {
            id: document_id.clone(),
            source_uri: "synthetic://recovered".to_string(),
            normalized_path: "synthetic/recovered.pdf".to_string(),
            file_name: "synthetic-recovered.pdf".to_string(),
            extension: FileExtension::Pdf,
            byte_size: 128,
            mtime: now,
            content_hash: Some("synthetic-recovered-content-hash".to_string()),
            text_hash: Some("synthetic-recovered-text-hash".to_string()),
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
            raw_text: Some("diagnostic recovered Java snapshot".to_string()),
            clean_text: Some("diagnostic recovered Java snapshot".to_string()),
            quality_score: Some(0.8),
            visibility: ResumeVisibility::Searchable,
        })
        .unwrap();

    (document_id.to_string(), version_id.to_string())
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s13-cli-{label}-{unique}"))
}

fn temp_dir(label: &str) -> PathBuf {
    let path = temp_path(label);
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

#[cfg(unix)]
fn write_executable(directory: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
fn write_private_file(directory: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = directory.join(name);
    fs::write(&path, body).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(&path, permissions).unwrap();
    path
}
