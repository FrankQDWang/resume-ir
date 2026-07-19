use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use import_pipeline::{current_import_processing_contract, ImportOptions, LinearPromotionPolicy};
use meta_store::{
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportRootKind, ImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus, ReadMetaStore, UnixTimestamp,
};
use serde_json::json;
use sha2::{Digest, Sha256};

#[test]
fn daemon_import_uses_the_bundled_classifier_model_by_default() {
    let data_dir = temp_dir("daemon-classifier-data");
    let root = temp_dir("daemon-classifier-root");
    fs::write(
        root.join("safe-gray.txt"),
        "PROFILE\nPlatform engineer with Rust experience.\nINVOICE\n",
    )
    .unwrap();
    let canonical_root = fs::canonicalize(&root).unwrap();
    let model = data_dir.join("bundled-classifier-model.json");
    write_synthetic_bundled_model(&model);
    seed_queued_import_task(&data_dir, &canonical_root, &model);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--once",
            "--work-imports-once",
            "--resume-classifier-model",
            path_str(&model),
        ])
        .output()
        .expect("run daemon import worker with bundled classifier model");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("import worker processed: 1"),
        "stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("import worker searchable documents: 1"),
        "stdout:\n{stdout}"
    );
    assert!(!stdout.contains(path_str(&model)));
    assert!(!stdout.contains(path_str(&root)));

    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let summary = store.status_summary().unwrap();
    assert_eq!(summary.searchable_documents, 1);

    remove_dir(&data_dir);
    remove_dir(&root);
}

fn write_synthetic_bundled_model(path: &Path) {
    let model = json!({
        "schema": "resume_ir_linear_promotion_v1",
        "classifier_epoch": "precision_first_v4",
        "feature_contract": "bounded_normalized_text_plus_structure_v1",
        "max_input_chars": 128,
        "threshold": 0.7,
        "intercept": 0.0,
        "features": [{"ngram": "pla", "idf": 1.0, "coefficient": 12.0}]
    });
    let model_json = serde_json::to_string(&model).unwrap();
    let model_sha256 = format!("{:x}", Sha256::digest(model_json.as_bytes()));
    fs::write(
        path,
        serde_json::to_vec(&json!({
            "model_json": model_json,
            "model_sha256": model_sha256
        }))
        .unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
    }
}

fn seed_queued_import_task(data_dir: &Path, canonical_root: &Path, model: &Path) {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data directory is owned"),
    };
    let store = owner.open_store().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_700_300_000);
    let task_id = ImportTaskId::from_non_secret_parts(&["s82", "bundled-classifier"]);
    let task = ImportTask {
        id: task_id.clone(),
        root_path: path_str(canonical_root).to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
        import_task_id: task_id,
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: path_str(canonical_root).to_string(),
        canonical_root_path: path_str(canonical_root).to_string(),
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: None,
        scan_budget_limit: None,
        scan_budget_observed: None,
        scan_budget_exhausted: false,
        updated_at: now,
    };
    let processing_contract = current_import_processing_contract(&ImportOptions {
        linear_promotion: LinearPromotionPolicy::load_bundled(model),
        ..ImportOptions::default()
    })
    .unwrap();
    store
        .activate_migration_rebuild_contract(&processing_contract, now)
        .unwrap();
    store
        .insert_import_task_with_scan_scope(&task, &scope, &processing_contract)
        .unwrap();
}

fn temp_dir(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "resume-ir-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    root
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}
