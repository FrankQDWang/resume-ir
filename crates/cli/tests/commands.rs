fn run_cli(args: &[&str]) -> (i32, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code = resume_cli::run(args.iter().copied(), &mut stdout, &mut stderr);

    (
        code,
        String::from_utf8(stdout).expect("stdout utf8"),
        String::from_utf8(stderr).expect("stderr utf8"),
    )
}

fn run_cli_with_state(args: &[&str], state_dir: &std::path::Path) -> (i32, String, String) {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let code =
        resume_cli::run_with_state_dir(args.iter().copied(), &mut stdout, &mut stderr, state_dir);

    (
        code,
        String::from_utf8(stdout).expect("stdout utf8"),
        String::from_utf8(stderr).expect("stderr utf8"),
    )
}

fn fixture_path(path: &str) -> String {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("../..")
        .join(path)
        .to_string_lossy()
        .into_owned()
}

fn unique_state_dir(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("resume_ir_cli_state_{name}_{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("clean state dir");
    }
    std::fs::create_dir_all(&dir).expect("create state dir");
    dir
}

#[test]
fn status_prints_user_readable_health() {
    let (code, stdout, stderr) = run_cli(&["resume-cli", "status"]);

    assert_eq!(code, 0);
    assert!(stdout.contains("health: ok"));
    assert!(stdout.contains("active_profile: balanced"));
    assert!(stderr.is_empty());
}

#[test]
fn import_root_writes_snapshot_summary() {
    let root = fixture_path("tests/fixtures/empty");
    let state_dir = unique_state_dir("empty_import");
    let (code, stdout, stderr) =
        run_cli_with_state(&["resume-cli", "import", "--root", &root], &state_dir);

    assert_eq!(code, 0);
    assert!(stdout.contains("import_job: completed"));
    assert!(stdout.contains("indexed_documents: 0"));
    assert!(stderr.is_empty());
}

#[test]
fn import_root_reports_missing_directory_without_panic() {
    let (code, stdout, stderr) =
        run_cli(&["resume-cli", "import", "--root", "tests/fixtures/missing"]);

    assert_eq!(code, 1);
    assert!(stdout.is_empty());
    assert!(stderr.contains("root is not a readable directory"));
}

#[test]
fn search_returns_ranked_hits_with_snippets() {
    let (code, stdout, stderr) = run_cli(&["resume-cli", "search", "Java"]);

    assert_eq!(code, 0);
    assert!(stdout.contains("rank: 1"));
    assert!(stdout.contains("doc_id:"));
    assert!(stdout.contains("file_name:"));
    assert!(stdout.contains("snippet:"));
    assert!(stderr.is_empty());
}

#[test]
fn import_status_and_search_use_persisted_snapshot() {
    let root = fixture_path("tests/fixtures/resumes");
    let state_dir = unique_state_dir("snapshot");

    let (import_code, import_stdout, import_stderr) =
        run_cli_with_state(&["resume-cli", "import", "--root", &root], &state_dir);
    assert_eq!(import_code, 0);
    assert!(import_stdout.contains("searchable_documents:"));
    assert!(import_stderr.is_empty());

    let (status_code, status_stdout, status_stderr) =
        run_cli_with_state(&["resume-cli", "status"], &state_dir);
    assert_eq!(status_code, 0);
    assert!(status_stdout.contains("indexed_documents:"));
    assert!(status_stdout.contains("searchable_documents:"));
    assert!(status_stderr.is_empty());

    let (search_code, search_stdout, search_stderr) =
        run_cli_with_state(&["resume-cli", "search", "Java"], &state_dir);
    assert_eq!(search_code, 0);
    assert!(search_stdout.contains("rank: 1"));
    assert!(search_stdout.contains("doc_id:"));
    assert!(search_stdout.contains("snippet:"));
    assert!(search_stderr.is_empty());
}

#[test]
fn search_filters_persisted_snapshot_by_degree() {
    let root = fixture_path("tests/fixtures/resumes");
    let state_dir = unique_state_dir("degree_filter");

    let (import_code, _import_stdout, import_stderr) =
        run_cli_with_state(&["resume-cli", "import", "--root", &root], &state_dir);
    assert_eq!(import_code, 0);
    assert!(import_stderr.is_empty());

    let (search_code, search_stdout, search_stderr) = run_cli_with_state(
        &[
            "resume-cli",
            "search",
            "Java",
            "--degree",
            "bachelor",
            "--top-k",
            "20",
        ],
        &state_dir,
    );

    assert_eq!(search_code, 0);
    assert!(search_stdout.contains("query: Java"));
    assert!(search_stdout.contains("results: 1"));
    assert!(search_stdout.contains("file_name: java_payment_text.pdf"));
    assert!(!search_stdout.contains("file_name: java_backend.docx"));
    assert!(search_stderr.is_empty());
}

#[test]
fn doctor_reports_query_smoke_and_fault_simulation() {
    let state_dir = unique_state_dir("doctor");

    let (code, stdout, stderr) = run_cli_with_state(&["resume-cli", "doctor"], &state_dir);

    assert_eq!(code, 0);
    assert!(stdout.contains("doctor: ok"));
    assert!(stdout.contains("snapshot: missing"));
    assert!(stdout.contains("query_smoke: ok"));
    assert!(stdout.contains("daemon_recovery_smoke: simulated_not_running"));
    assert!(stdout.contains("disk_space_low_simulation: available"));
    assert!(stderr.is_empty());
}

#[test]
fn doctor_reports_corrupt_snapshot_without_panic() {
    let state_dir = unique_state_dir("doctor_corrupt");
    std::fs::write(state_dir.join("cli-index.tsv"), "broken\tline\n").expect("write corrupt");

    let (code, stdout, stderr) = run_cli_with_state(&["resume-cli", "doctor"], &state_dir);

    assert_eq!(code, 0);
    assert!(stdout.contains("snapshot: corrupt"));
    assert!(stderr.is_empty());
}

#[test]
fn export_diagnostics_requires_redaction_and_hides_resume_content() {
    let root = fixture_path("tests/fixtures/resumes");
    let state_dir = unique_state_dir("diagnostics");
    let (import_code, _import_stdout, import_stderr) =
        run_cli_with_state(&["resume-cli", "import", "--root", &root], &state_dir);
    assert_eq!(import_code, 0);
    assert!(import_stderr.is_empty());

    let (code, stdout, stderr) = run_cli_with_state(
        &["resume-cli", "export-diagnostics", "--redact"],
        &state_dir,
    );

    assert_eq!(code, 0);
    assert!(stdout.contains("diagnostics: redacted"));
    assert!(stdout.contains("indexed_documents: 2"));
    assert!(stdout.contains("paths: [redacted]"));
    assert!(stdout.contains("resume_text: [redacted]"));
    assert!(!stdout.contains("Java payment gateway"));
    assert!(!stdout.contains("tests/fixtures"));
    assert!(!stdout.contains("Zhejiang University"));
    assert!(stderr.is_empty());
}
