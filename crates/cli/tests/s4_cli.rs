use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus,
    MetaStore, UnixTimestamp,
};

#[test]
fn top_level_help_lists_core_operator_workflows_without_data_dir_or_path_leak() {
    for args in [["--help"].as_slice(), ["help"].as_slice()] {
        let cwd = temp_dir("top-level-help-cwd");
        let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
            .current_dir(&cwd)
            .args(args)
            .output()
            .expect("run resume-cli top-level help");

        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("resume-cli"));
        assert!(stdout.contains("Local-first resume import and search"));
        assert!(stdout.contains("import"));
        assert!(stdout.contains("search"));
        assert!(stdout.contains("ocr preflight"));
        assert!(stdout.contains("model preflight"));
        assert!(stdout.contains("doctor"));
        assert!(stdout.contains("export-diagnostics --redact"));
        assert!(stdout.contains(
            "benchmark-query-set   Preflight or freeze local private query-set evidence."
        ));
        assert!(!stdout.contains("Draft, preflight, or freeze local private query-set evidence."));
        assert!(stdout.contains("release-readiness"));
        assert!(stdout.contains("Performance optimization is deferred"));
        assert!(!stdout.contains("/Users/"));
        assert!(!stdout.contains("PRIVATE"));
        assert!(!cwd.join("local-data").exists());
        remove_dir(&cwd);
    }
}

#[test]
fn command_help_lists_core_usage_without_data_dir_or_path_leak() {
    let cases: &[(&[&str], &[&str])] = &[
        (
            &["help", "import"],
            &["usage: resume-cli import", "--root-preset local-discovery"],
        ),
        (
            &["search", "--help"],
            &[
                "usage: resume-cli search",
                "--mode fulltext|semantic|hybrid",
            ],
        ),
        (&["ocr", "--help"], &["usage: resume-cli ocr", "preflight"]),
        (
            &["model", "--help"],
            &["usage: resume-cli model", "preflight"],
        ),
        (
            &["status", "--help"],
            &["usage: resume-cli status", "--watch-import"],
        ),
    ];

    for (args, expected_fragments) in cases {
        let cwd = temp_dir("command-help-cwd");
        let data_dir = cwd.join("private-data-dir");
        let mut command_args = vec!["--data-dir", path_str(&data_dir)];
        command_args.extend(args.iter().copied());

        let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
            .current_dir(&cwd)
            .args(command_args)
            .output()
            .expect("run resume-cli command help");

        assert!(output.status.success(), "args {args:?}");
        assert!(output.stderr.is_empty(), "args {args:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        for fragment in *expected_fragments {
            assert!(
                stdout.contains(fragment),
                "args {args:?}, missing {fragment}"
            );
        }
        assert!(!stdout.contains("/Users/"), "args {args:?}");
        assert!(!stdout.contains("PRIVATE"), "args {args:?}");
        assert!(!stdout.contains(path_str(&data_dir)), "args {args:?}");
        assert!(!data_dir.exists(), "args {args:?}");
        assert!(!cwd.join("local-data").exists(), "args {args:?}");
        remove_dir(&cwd);
    }

    let cwd = temp_dir("command-help-after-private-arg-cwd");
    let data_dir = cwd.join("private-data-dir");
    let root_dir = cwd.join("private-root-name");
    fs::create_dir_all(&root_dir).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .current_dir(&cwd)
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root_dir),
            "--help",
        ])
        .output()
        .expect("run resume-cli command help after private arg");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("usage: resume-cli import"));
    assert!(!stdout.contains("import task submitted"));
    assert!(!stdout.contains(path_str(&root_dir)));
    assert!(!data_dir.exists());
    assert!(!cwd.join("local-data").exists());
    remove_dir(&cwd);
}

#[test]
fn status_creates_store_and_reports_empty_aggregates() {
    let data_dir = temp_dir("status-data");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("indexed documents: 0"));
    assert!(stdout.contains("search index: unavailable"));
    assert!(!stdout.contains(path_str(&data_dir)));

    remove_dir(&data_dir);
}

#[test]
fn status_reports_latest_import_scan_progress_without_path_leak() {
    let data_dir = temp_dir("status-import-progress-data");
    let private_root = temp_dir("status-import-progress-private-root");
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_700_000_200);
    let task_id = ImportTaskId::from_non_secret_parts(&["status-import-progress"]);
    store
        .insert_import_task(&ImportTask {
            id: task_id.clone(),
            root_path: path_str(&private_root).to_string(),
            status: ImportTaskStatus::Running,
            queued_at: now,
            started_at: Some(now),
            finished_at: None,
            updated_at: now,
        })
        .unwrap();
    store
        .upsert_import_scan_scope(&ImportScanScope {
            import_task_id: task_id,
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: path_str(&private_root).to_string(),
            canonical_root_path: path_str(&private_root).to_string(),
            files_discovered: 9,
            ignored_entries: 2,
            scan_errors: 1,
            searchable_documents: 4,
            ocr_required_documents: 1,
            ocr_jobs_queued: 1,
            failed_documents: 1,
            deleted_documents: 0,
            scan_budget_kind: None,
            scan_budget_limit: None,
            scan_budget_observed: None,
            scan_budget_exhausted: false,
            updated_at: now,
        })
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status with import progress");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("latest import files discovered: 9"));
    assert!(stdout.contains("latest import searchable documents: 4"));
    assert!(stdout.contains("latest import ocr required documents: 1"));
    assert!(stdout.contains("latest import scan errors: 1"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn status_watch_import_without_ipc_reports_local_import_scan_progress_without_path_leak() {
    let data_dir = temp_dir("status-watch-import-local-data");
    let private_root = temp_dir("status-watch-import-local-private-root");
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_700_000_201);
    let task_id = ImportTaskId::from_non_secret_parts(&["status-watch-import-local"]);
    store
        .insert_import_task(&ImportTask {
            id: task_id.clone(),
            root_path: path_str(&private_root).to_string(),
            status: ImportTaskStatus::Completed,
            queued_at: now,
            started_at: Some(now),
            finished_at: Some(now),
            updated_at: now,
        })
        .unwrap();
    store
        .upsert_import_scan_scope(&ImportScanScope {
            import_task_id: task_id,
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: path_str(&private_root).to_string(),
            canonical_root_path: path_str(&private_root).to_string(),
            files_discovered: 3,
            ignored_entries: 1,
            scan_errors: 0,
            searchable_documents: 2,
            ocr_required_documents: 0,
            ocr_jobs_queued: 0,
            failed_documents: 0,
            deleted_documents: 1,
            scan_budget_kind: None,
            scan_budget_limit: None,
            scan_budget_observed: None,
            scan_budget_exhausted: false,
            updated_at: now,
        })
        .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "status",
            "--watch-import",
        ])
        .output()
        .expect("run resume-cli local status --watch-import");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resume-ir status"));
    assert!(stdout.contains("latest import files discovered: 3"));
    assert!(stdout.contains("latest import searchable documents: 2"));
    assert!(stdout.contains("latest import deleted documents: 1"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&private_root)));

    remove_dir(&data_dir);
    remove_dir(&private_root);
}

#[test]
fn import_root_submits_persistent_task_without_path_leak() {
    let data_dir = temp_dir("import-data");
    let root_dir = temp_dir("import-root-private-name");

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run resume-cli import");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("import task submitted"));
    assert!(stdout.contains("task id: imp_"));
    assert!(stdout.contains("status: completed"));
    assert!(stdout.contains("files discovered: 0"));
    assert!(!stdout.contains(path_str(&root_dir)));

    let status = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "status"])
        .output()
        .expect("run resume-cli status after import");
    assert!(status.status.success());
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    assert!(status_stdout.contains("import tasks queued: 0"));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn import_rejects_duplicate_root_and_profile_flags_without_path_leak() {
    let data_dir = temp_dir("duplicate-import-data");
    let root_dir = temp_dir("duplicate-import-root-private-name");

    let duplicate_root = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root_dir),
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run duplicate root import");
    assert!(!duplicate_root.status.success());
    assert!(duplicate_root.stdout.is_empty());
    let duplicate_root_stderr = String::from_utf8_lossy(&duplicate_root.stderr);
    assert!(duplicate_root_stderr.contains("usage: resume-cli import"));
    assert!(!duplicate_root_stderr.contains(path_str(&root_dir)));

    let duplicate_profile = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root_dir),
            "--profile",
            "explicit",
            "--profile",
            "discovery",
        ])
        .output()
        .expect("run duplicate profile import");
    assert!(!duplicate_profile.status.success());
    assert!(duplicate_profile.stdout.is_empty());
    let duplicate_profile_stderr = String::from_utf8_lossy(&duplicate_profile.stderr);
    assert!(duplicate_profile_stderr.contains("usage: resume-cli import"));
    assert!(!duplicate_profile_stderr.contains(path_str(&root_dir)));

    let mixed_preset_and_root = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root-preset",
            "local-discovery",
            "--root",
            path_str(&root_dir),
        ])
        .output()
        .expect("run mixed root-preset and root import");
    assert!(!mixed_preset_and_root.status.success());
    assert!(mixed_preset_and_root.stdout.is_empty());
    let mixed_stderr = String::from_utf8_lossy(&mixed_preset_and_root.stderr);
    assert!(mixed_stderr.contains("usage: resume-cli import"));
    assert!(!mixed_stderr.contains(path_str(&root_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn import_rejects_overlapping_roots_without_path_leak() {
    let data_dir = temp_dir("overlap-import-data");
    let root_dir = temp_dir("overlap-import-root-private-name");
    let child_dir = root_dir.join("child");
    fs::create_dir_all(&child_dir).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root_dir),
            "--root",
            path_str(&child_dir),
        ])
        .output()
        .expect("run overlapping root import");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("import roots must be distinct and non-overlapping"));
    assert!(!stderr.contains(path_str(&root_dir)));
    assert!(!stderr.contains(path_str(&child_dir)));

    remove_dir(&data_dir);
    remove_dir(&root_dir);
}

#[test]
fn search_without_index_returns_unavailable_message_without_echoing_query() {
    let data_dir = temp_path("search-data");
    let sensitive_query = "Java PRIVATE_TOKEN";

    assert!(!data_dir.exists());

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", sensitive_query])
        .output()
        .expect("run resume-cli search");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("search index not available yet"));
    assert!(stdout.contains("results: 0"));
    assert!(!stdout.contains(sensitive_query));
    assert!(!data_dir.exists());

    remove_dir(&data_dir);
}

fn temp_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-s4-cli-{label}-{unique}"))
}

fn temp_dir(label: &str) -> PathBuf {
    let path = temp_path(label);
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}
