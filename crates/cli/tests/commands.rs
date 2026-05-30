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

fn fixture_path(path: &str) -> String {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .join("../..")
        .join(path)
        .to_string_lossy()
        .into_owned()
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
fn import_root_queues_a_skeleton_job() {
    let root = fixture_path("tests/fixtures/empty");
    let (code, stdout, stderr) = run_cli(&["resume-cli", "import", "--root", &root]);

    assert_eq!(code, 0);
    assert!(stdout.contains("import_job: queued"));
    assert!(stdout.contains("job_"));
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
fn search_returns_clear_empty_result_before_index_exists() {
    let (code, stdout, stderr) = run_cli(&["resume-cli", "search", "Java"]);

    assert_eq!(code, 0);
    assert!(stdout.contains("results: 0"));
    assert!(stdout.contains("full-text index is not available"));
    assert!(stderr.is_empty());
}
