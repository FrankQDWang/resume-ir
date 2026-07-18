mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use import_pipeline::ImportTaskOwnerLock;
use meta_store::{
    ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus,
    MetaStore, UnixTimestamp,
};
use rusqlite::{params, Connection};
use support::{assert_import_succeeded, import_text_resumes};

#[test]
fn doctor_uses_sqlcipher_metadata_by_default_without_key_or_path_leak() {
    let data_dir = temp_path("doctor-sqlcipher-private-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("metadata encryption: sqlcipher"));
    assert!(stdout.contains("ocr cache encryption: sqlcipher"));
    assert!(!stdout.contains("enable SQLCipher metadata encryption before production release"));
    assert!(!stdout.contains(path_str(&data_dir)));

    let metadata_path = meta_store::metadata_store_path(&data_dir).unwrap();
    let encrypted_bytes = fs::read(&metadata_path).unwrap();
    assert!(!encrypted_bytes.starts_with(b"SQLite format 3"));
    let metadata_key =
        fs::read_to_string(data_dir.join("metadata-secrets/metadata-sqlcipher-key-v1"))
            .expect("metadata SQLCipher key");
    assert!(!stdout.contains(metadata_key.trim()));
    assert!(MetaStore::open(metadata_path)
        .and_then(|store| store.schema_version().map(|_| ()))
        .is_err());

    remove_dir(&data_dir);
}

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
    assert!(stdout.contains("metadata encryption: sqlcipher"));
    assert!(stdout.contains("ocr cache encryption: sqlcipher"));
    assert!(!stdout.contains("enable SQLCipher metadata encryption before production release"));
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
fn export_diagnostics_redact_outputs_local_aggregate_evidence_without_paths() {
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
    assert!(stdout.contains("\"metadata_encryption\": \"sqlcipher\""));
    assert!(stdout.contains("\"ocr_cache_encryption\": \"sqlcipher\""));
    assert!(!stdout.contains("enable SQLCipher metadata encryption before production release"));
    assert!(stdout.contains("\"evidence_level\": \"local_aggregate_only\""));
    assert!(stdout.contains("\"diagnostic_scope\": {"));
    assert!(stdout.contains("\"metadata\": \"aggregate_counts\""));
    assert!(stdout.contains("\"search_index\": \"state_and_snapshot_health\""));
    assert!(stdout.contains("\"vector_index\": \"state_backend_and_counts\""));
    assert!(stdout.contains("\"query_latency\": \"aggregate_observations\""));
    assert!(stdout.contains("\"runtime_dependencies\": \"presence_only\""));
    assert!(stdout.contains("\"fault_simulations\": \"available_cases_only\""));
    assert!(stdout.contains("\"daemon_restart\""));
    assert!(stdout.contains("\"daemon_kill\""));
    assert!(stdout.contains("\"disk_space_low\""));
    assert!(stdout.contains("\"file_lock\""));
    assert!(stdout.contains("\"metadata_migration\""));
    assert!(stdout.contains("\"ocr_crash\""));
    assert!(stdout.contains("\"model_checksum\""));
    assert!(stdout.contains("\"battery_mode\""));
    assert!(stdout.contains("\"external_drive_disconnect\""));
    assert!(!stdout.contains("skeleton"));
    assert!(!stdout.contains("fake"));
    assert!(!stdout.contains("synthetic-only"));
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
    assert!(stdout.contains("\"requested_language\": \"eng\""));
    assert!(stdout.contains("\"requested_language_status\": \"available\""));
    assert!(stdout.contains("\"paths\": \"<redacted>\""));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&bin_dir)));
    assert!(!stdout.contains("chi_sim"));

    remove_dir(&data_dir);
    remove_dir(&bin_dir);
}

#[cfg(unix)]
#[test]
fn doctor_and_diagnostics_check_requested_ocr_language_without_language_dump() {
    let data_dir = temp_dir("diagnostics-ocr-runtime-custom-lang-data");
    let bin_dir = temp_dir("diagnostics-ocr-runtime-custom-lang-bin");
    write_executable(&bin_dir, "pdftoppm", "#!/bin/sh\nexit 0\n");
    write_executable(
        &bin_dir,
        "tesseract",
        r#"#!/bin/sh
if [ "$1" = "--list-langs" ]; then
  printf 'List of available languages (3):\n'
  printf 'eng\n'
  printf 'chi_sim\n'
  printf 'jpn\n'
  exit 0
fi
exit 9
"#,
    );

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("PATH", path_str(&bin_dir))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--ocr-lang",
            "chi_sim",
        ])
        .output()
        .expect("run resume-cli doctor with requested OCR language");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("ocr renderer pdftoppm: available"));
    assert!(stdout.contains("ocr engine tesseract: available"));
    assert!(stdout.contains("ocr language chi_sim: available"));
    assert!(!stdout.contains("ocr language eng:"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&bin_dir)));
    assert!(!stdout.contains("jpn"));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("PATH", path_str(&bin_dir))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
            "--ocr-lang",
            "chi_sim",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with requested OCR language");
    assert!(export.status.success());
    assert!(export.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"ocr_runtime\": {"));
    assert!(stdout.contains("\"pdftoppm\": \"available\""));
    assert!(stdout.contains("\"tesseract\": \"available\""));
    assert!(stdout.contains("\"requested_language\": \"chi_sim\""));
    assert!(stdout.contains("\"requested_language_status\": \"available\""));
    assert!(stdout.contains("\"paths\": \"<redacted>\""));
    assert!(!stdout.contains("\"tesseract_eng\""));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&bin_dir)));
    assert!(!stdout.contains("jpn"));

    remove_dir(&data_dir);
    remove_dir(&bin_dir);
}

#[cfg(unix)]
#[test]
fn doctor_and_diagnostics_check_combined_ocr_languages_without_language_dump() {
    let data_dir = temp_dir("diagnostics-ocr-runtime-combined-lang-data");
    let bin_dir = temp_dir("diagnostics-ocr-runtime-combined-lang-bin");
    write_executable(&bin_dir, "pdftoppm", "#!/bin/sh\nexit 0\n");
    write_executable(
        &bin_dir,
        "tesseract",
        r#"#!/bin/sh
if [ "$1" = "--list-langs" ]; then
  printf 'List of available languages (3):\n'
  printf 'eng\n'
  printf 'chi_sim\n'
  printf 'jpn\n'
  exit 0
fi
exit 9
"#,
    );

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("PATH", path_str(&bin_dir))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--ocr-lang",
            "eng+chi_sim",
        ])
        .output()
        .expect("run resume-cli doctor with combined OCR languages");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("ocr renderer pdftoppm: available"));
    assert!(stdout.contains("ocr engine tesseract: available"));
    assert!(stdout.contains("ocr language eng+chi_sim: available"));
    assert!(!stdout.contains("ocr language jpn:"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&bin_dir)));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .env("PATH", path_str(&bin_dir))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
            "--ocr-lang",
            "eng+chi_sim",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with combined OCR languages");
    assert!(export.status.success());
    assert!(export.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"ocr_runtime\": {"));
    assert!(stdout.contains("\"pdftoppm\": \"available\""));
    assert!(stdout.contains("\"tesseract\": \"available\""));
    assert!(stdout.contains("\"requested_language\": \"eng+chi_sim\""));
    assert!(stdout.contains("\"requested_language_status\": \"available\""));
    assert!(stdout.contains("\"paths\": \"<redacted>\""));
    assert!(!stdout.contains("\"jpn\""));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&bin_dir)));

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
fn doctor_and_diagnostics_report_the_generation_bound_disabled_vector_snapshot() {
    let data_dir = temp_dir("diagnostics-vector-v3-data");
    let source_root = temp_dir("diagnostics-vector-v3-source");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[(
            "diagnostic-vector.txt",
            "SUMMARY\nDiagnostic Candidate\nEXPERIENCE\nBuilt diagnostic Rust systems\nSKILLS\nRust",
        )],
    ));

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with v3 vector descriptor");
    assert!(doctor.status.success());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("vector index: available (disabled vector snapshot)"));
    assert!(stdout.contains("vector index vectors: 0"));
    assert!(stdout.contains("vector index tombstones: 0"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&source_root)));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with v3 vector descriptor");
    assert!(export.status.success());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"vector_index_state\": \"available\""));
    assert!(stdout.contains("\"vector_index_backend\": \"none\""));
    assert!(stdout.contains("\"vector_index_vectors\": 0"));
    assert!(stdout.contains("\"vector_index_tombstones\": 0"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&source_root)));

    remove_dir(&data_dir);
    remove_dir(&source_root);
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
fn doctor_and_diagnostics_report_the_atomic_search_publication() {
    let data_dir = temp_dir("diagnostics-index-health");
    let source_root = temp_dir("diagnostics-index-health-source");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[(
            "synthetic-diagnostic.txt",
            "SUMMARY\nDiagnostic Candidate\nEXPERIENCE\nBuilt diagnostic Java search systems\nSKILLS\nJava",
        )],
    ));

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with atomic search publication");
    assert!(doctor.status.success());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("search index: available (database Ready full-text snapshot)"));
    assert!(stdout.contains("index health: ready"));
    assert!(stdout.contains("last snapshot: present"));
    assert!(stdout.contains("search index read target: database_ready_generation"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&source_root)));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with atomic search publication");
    assert!(export.status.success());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"search_index_state\": \"available\""));
    assert!(stdout.contains("\"search_index_read_target\": \"database_ready_generation\""));
    assert!(stdout.contains("\"index_health\": \"ready\""));
    assert!(stdout.contains("\"last_snapshot\": \"present\""));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&source_root)));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

#[test]
fn doctor_and_search_fail_closed_when_the_published_generation_is_corrupt() {
    let data_dir = temp_dir("diagnostics-snapshot-corrupt");
    let source_root = temp_dir("diagnostics-snapshot-corrupt-source");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[(
            "synthetic-corrupt.txt",
            "SUMMARY\nCorrupt Candidate\nEXPERIENCE\nBuilt PRIVATE_DIAGNOSTIC_SENTINEL systems\nSKILLS\nRust",
        )],
    ));
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let generation = store.search_projection_state().unwrap().generation.unwrap();
    fs::write(
        data_dir
            .join("search-index")
            .join("snapshots")
            .join(&generation)
            .join("fulltext.snapshot.enc"),
        b"not a valid Ready generation",
    )
    .unwrap();

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "PRIVATE_DIAGNOSTIC_SENTINEL",
        ])
        .output()
        .expect("run resume-cli search with corrupt publication");
    assert!(!search.status.success());
    assert!(search.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&search.stderr).contains("PRIVATE_DIAGNOSTIC_SENTINEL"));

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run resume-cli doctor with corrupt publication");
    assert!(doctor.status.success());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("search index: corrupt"));
    assert!(stdout.contains("search index read target: database_ready_generation"));
    assert!(stdout.contains("snapshot fallback: none"));
    assert!(!stdout.contains(&generation));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&source_root)));

    let export = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "export-diagnostics",
            "--redact",
        ])
        .output()
        .expect("run resume-cli export-diagnostics with corrupt publication");
    assert!(export.status.success());
    let stdout = String::from_utf8_lossy(&export.stdout);
    assert!(stdout.contains("\"search_index_state\": \"corrupt\""));
    assert!(stdout.contains("\"snapshot_fallback\": \"none\""));
    assert!(!stdout.contains(&generation));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&source_root)));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

#[test]
fn doctor_pending_import_task_boundary_reports_post_boundary_without_path_leak() {
    let data_dir = temp_dir("doctor-pending-import-boundary-healthy");
    let root_dir = temp_dir("doctor-pending-import-boundary-root");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--pending-import-task-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor pending import task boundary");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-ir doctor"));
    assert!(stdout.contains(
        "pending import task boundary: unexpected_success_then_post_pending_task_boundary"
    ));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_pending_import_task_boundary_reports_pending_import_task_by_root_failure_without_path_or_key_leak(
) {
    let data_dir = temp_dir("doctor-pending-import-boundary-missing-table");
    let root_dir = temp_dir("doctor-pending-import-boundary-broken-root");
    let metadata_key = {
        let store = MetaStore::open_data_dir(&data_dir).unwrap();
        store.run_migrations().unwrap();
        fs::read_to_string(meta_store::metadata_encryption_key_path(&data_dir)).unwrap()
    };
    let connection = open_encrypted_metadata_connection(&data_dir);
    connection
        .execute_batch("ALTER TABLE import_task RENAME TO import_task_missing;")
        .unwrap();
    drop(connection);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--pending-import-task-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor pending import task boundary broken import task table");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("pending import task boundary: pending_import_task_query_failure"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));
    assert!(!stdout.contains(metadata_key.trim()));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_pending_import_task_boundary_reports_row_materialization_failure_without_path_or_key_leak(
) {
    let data_dir = temp_dir("doctor-pending-import-boundary-corrupt-row");
    let root_dir = temp_dir("doctor-pending-import-boundary-corrupt-row-root");
    let canonical_root = fs::canonicalize(&root_dir).unwrap();
    let metadata_key = {
        let store = MetaStore::open_data_dir(&data_dir).unwrap();
        store.run_migrations().unwrap();
        fs::read_to_string(meta_store::metadata_encryption_key_path(&data_dir)).unwrap()
    };
    let connection = open_encrypted_metadata_connection(&data_dir);
    connection
        .execute(
            "\
            INSERT INTO import_task (
                id, root_path, status, queued_at_seconds, updated_at_seconds
            )
            VALUES (?1, ?2, 'queued', 1, 1)",
            params!["diagnostic-materialization-task", path_str(&canonical_root)],
        )
        .unwrap();
    connection
        .execute("UPDATE import_task SET id = zeroblob(16)", [])
        .unwrap();
    drop(connection);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--pending-import-task-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor pending import task boundary corrupt import task row");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout
        .contains("pending import task boundary: pending_import_task_row_materialization_failure"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));
    assert!(!stdout.contains(path_str(&canonical_root)));
    assert!(!stdout.contains(metadata_key.trim()));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_post_pending_import_task_recovery_boundary_reports_recovered_running_task_without_path_or_key_leak(
) {
    let data_dir = temp_dir("doctor-post-pending-import-recovery-healthy");
    let root_dir = temp_dir("doctor-post-pending-import-recovery-root");
    seed_running_import_task(&data_dir, &root_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--post-pending-import-task-recovery-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor post pending import task recovery boundary");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-ir doctor"));
    assert!(stdout.contains(
        "post pending import task recovery boundary: stale_running_task_recovered_before_post_boundary"
    ));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_post_pending_import_task_recovery_boundary_reports_post_boundary_without_path_leak() {
    let data_dir = temp_dir("doctor-post-pending-import-recovery-post-boundary");
    let root_dir = temp_dir("doctor-post-pending-import-recovery-post-boundary-root");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--post-pending-import-task-recovery-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor post pending import task recovery boundary");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(
        "post pending import task recovery boundary: unexpected_success_then_post_pending_import_task_recovery_boundary"
    ));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_post_pending_import_task_recovery_boundary_reports_lock_bound_without_path_or_key_leak() {
    let data_dir = temp_dir("doctor-post-pending-import-recovery-lock-bound");
    let root_dir = temp_dir("doctor-post-pending-import-recovery-lock-root");
    let task_id = seed_running_import_task(&data_dir, &root_dir);
    let _owner_lock = ImportTaskOwnerLock::acquire(&data_dir, &task_id).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--post-pending-import-task-recovery-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor post pending import task recovery boundary lock bound");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout
        .contains("post pending import task recovery boundary: stale_running_task_lock_bound"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_post_pending_import_task_recovery_boundary_reports_status_update_failure_without_path_or_key_leak(
) {
    let data_dir = temp_dir("doctor-post-pending-import-recovery-update-failure");
    let root_dir = temp_dir("doctor-post-pending-import-recovery-update-root");
    seed_running_import_task(&data_dir, &root_dir);
    let connection = open_encrypted_metadata_connection(&data_dir);
    connection
        .execute_batch(
            "\
            CREATE TRIGGER import_task_block_status_update
            BEFORE UPDATE OF status ON import_task
            BEGIN
                SELECT RAISE(FAIL, 'diagnostic update blocked');
            END;
            ",
        )
        .unwrap();
    drop(connection);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--post-pending-import-task-recovery-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor post pending import task recovery boundary update failure");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(
        "post pending import task recovery boundary: stale_running_task_status_update_failure"
    ));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_post_pending_import_task_recovery_boundary_reports_row_refresh_failure_without_path_or_key_leak(
) {
    let data_dir = temp_dir("doctor-post-pending-import-recovery-row-refresh-failure");
    let root_dir = temp_dir("doctor-post-pending-import-recovery-row-refresh-root");
    seed_running_import_task(&data_dir, &root_dir);
    let connection = open_encrypted_metadata_connection(&data_dir);
    connection
        .execute_batch(
            "\
            CREATE TRIGGER import_task_delete_after_status_update
            AFTER UPDATE OF status ON import_task
            BEGIN
                DELETE FROM import_task WHERE id = NEW.id;
            END;
            ",
        )
        .unwrap();
    drop(connection);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--post-pending-import-task-recovery-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect(
            "run resume-cli doctor post pending import task recovery boundary row refresh failure",
        );

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(
        "post pending import task recovery boundary: stale_running_task_row_refresh_failure"
    ));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_post_recovery_retained_lineage_convergence_boundary_reports_recoverable_nonterminal_without_path_or_key_leak(
) {
    let data_dir = temp_dir("doctor-post-recovery-lineage-recoverable");
    let root_dir = temp_dir("doctor-post-recovery-lineage-recoverable-root");
    seed_import_task_with_status(&data_dir, &root_dir, ImportTaskStatus::FailedRetryable);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--post-recovery-retained-lineage-convergence-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor post recovery retained lineage convergence boundary");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(
        "post recovery retained lineage convergence boundary: retained_lineage_still_recoverable_after_reentry"
    ));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_post_recovery_retained_lineage_convergence_boundary_reports_running_without_visible_progress_without_path_or_key_leak(
) {
    let data_dir = temp_dir("doctor-post-recovery-lineage-running-no-progress");
    let root_dir = temp_dir("doctor-post-recovery-lineage-running-no-progress-root");
    let task_id = seed_import_task_with_status(&data_dir, &root_dir, ImportTaskStatus::Running);
    let _owner_lock = ImportTaskOwnerLock::acquire(&data_dir, &task_id).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--post-recovery-retained-lineage-convergence-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor post recovery running without visible progress");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(
        "post recovery retained lineage convergence boundary: retained_lineage_running_without_visible_progress_yet"
    ));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_post_recovery_retained_lineage_convergence_boundary_reports_visible_progress_without_path_or_key_leak(
) {
    let data_dir = temp_dir("doctor-post-recovery-lineage-visible-progress");
    let root_dir = temp_dir("doctor-post-recovery-lineage-visible-progress-root");
    let task_id = seed_import_task_with_status(&data_dir, &root_dir, ImportTaskStatus::Running);
    seed_import_scan_scope(
        &data_dir,
        &root_dir,
        &task_id,
        ImportScanScopeCounts {
            searchable_documents: 1,
            ..ImportScanScopeCounts::default()
        },
    );
    let _owner_lock = ImportTaskOwnerLock::acquire(&data_dir, &task_id).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--post-recovery-retained-lineage-convergence-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor post recovery visible progress");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(
        "post recovery retained lineage convergence boundary: retained_lineage_converged_to_visible_progress"
    ));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn doctor_post_recovery_retained_lineage_convergence_boundary_reports_completed_lineage_without_path_or_key_leak(
) {
    let data_dir = temp_dir("doctor-post-recovery-lineage-completed");
    let root_dir = temp_dir("doctor-post-recovery-lineage-completed-root");
    let task_id = seed_import_task_with_status(&data_dir, &root_dir, ImportTaskStatus::Completed);
    seed_import_scan_scope(
        &data_dir,
        &root_dir,
        &task_id,
        ImportScanScopeCounts {
            searchable_documents: 1,
            ..ImportScanScopeCounts::default()
        },
    );

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "doctor",
            "--post-recovery-retained-lineage-convergence-boundary",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli doctor post recovery completed lineage");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(
        "post recovery retained lineage convergence boundary: retained_lineage_converged_past_pending_task_boundary"
    ));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
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

fn open_encrypted_metadata_connection(data_dir: &Path) -> Connection {
    let metadata_key = fs::read_to_string(meta_store::metadata_encryption_key_path(data_dir))
        .expect("read metadata SQLCipher key");
    let connection = Connection::open(meta_store::metadata_store_path(data_dir).unwrap())
        .expect("open metadata db");
    connection
        .execute_batch(&format!("PRAGMA key = \"x'{}'\";", metadata_key.trim()))
        .expect("apply metadata SQLCipher key");
    connection
        .query_row("SELECT count(*) FROM sqlite_master", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("verify metadata SQLCipher key");
    connection
}

fn seed_running_import_task(data_dir: &Path, root_dir: &Path) -> ImportTaskId {
    seed_import_task_with_status(data_dir, root_dir, ImportTaskStatus::Running)
}

fn seed_import_task_with_status(
    data_dir: &Path,
    root_dir: &Path,
    status: ImportTaskStatus,
) -> ImportTaskId {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let canonical_root = fs::canonicalize(root_dir).unwrap();
    let queued_at = UnixTimestamp::from_unix_seconds(1_700_000_000);
    let started_at = UnixTimestamp::from_unix_seconds(1_700_000_010);
    let finished_at = matches!(
        status,
        ImportTaskStatus::Completed
            | ImportTaskStatus::FailedRetryable
            | ImportTaskStatus::FailedPermanent
    )
    .then_some(UnixTimestamp::from_unix_seconds(1_700_000_020));
    let id = ImportTaskId::from_non_secret_parts(&["s13", "running-import-task"]);
    store
        .insert_import_task(&ImportTask {
            id: id.clone(),
            root_path: path_str(&canonical_root).to_string(),
            status,
            queued_at,
            started_at: Some(started_at),
            finished_at,
            updated_at: finished_at.unwrap_or(started_at),
        })
        .unwrap();
    id
}

#[derive(Default)]
struct ImportScanScopeCounts {
    searchable_documents: u64,
    ocr_required_documents: u64,
    ocr_jobs_queued: u64,
    failed_documents: u64,
    deleted_documents: u64,
}

fn seed_import_scan_scope(
    data_dir: &Path,
    root_dir: &Path,
    task_id: &ImportTaskId,
    counts: ImportScanScopeCounts,
) {
    let store = MetaStore::open_data_dir(data_dir).unwrap();
    store.run_migrations().unwrap();
    let canonical_root = fs::canonicalize(root_dir).unwrap();
    let root_path = path_str(&canonical_root).to_string();
    store
        .upsert_import_scan_scope(&ImportScanScope {
            import_task_id: task_id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: root_path.clone(),
            canonical_root_path: root_path,
            files_discovered: 32,
            ignored_entries: 0,
            scan_errors: 0,
            searchable_documents: counts.searchable_documents,
            ocr_required_documents: counts.ocr_required_documents,
            ocr_jobs_queued: counts.ocr_jobs_queued,
            failed_documents: counts.failed_documents,
            deleted_documents: counts.deleted_documents,
            scan_budget_kind: None,
            scan_budget_limit: None,
            scan_budget_observed: None,
            scan_budget_exhausted: false,
            updated_at: UnixTimestamp::from_unix_seconds(1_700_000_030),
        })
        .unwrap();
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
