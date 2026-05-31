use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn search_supports_degree_filter_and_top_k_without_query_echo() {
    let data_dir = temp_dir("search-filter-data");
    import_fixtures(&data_dir);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--degree",
            "bachelor",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run filtered search");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 2"));
    assert!(stdout.contains("synthetic-java-engineer.docx"));
    assert!(stdout.contains("synthetic-java-platform.pdf"));
    assert!(!stdout.contains("query:"));

    let limited = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--degree",
            "bachelor",
            "--top-k",
            "1",
        ])
        .output()
        .expect("run filtered top-k search");
    assert!(limited.status.success());
    let limited_stdout = String::from_utf8_lossy(&limited.stdout);
    assert!(limited_stdout.contains("results: 1"));

    let filtered_out = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--degree",
            "master",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run filtered-out search");
    assert!(filtered_out.status.success());
    let filtered_out_stdout = String::from_utf8_lossy(&filtered_out.stdout);
    assert!(filtered_out_stdout.contains("results: 0"));

    let skill_filtered = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--skills-any",
            "java",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run skill-filtered search");
    assert!(skill_filtered.status.success());
    let skill_filtered_stdout = String::from_utf8_lossy(&skill_filtered.stdout);
    assert!(skill_filtered_stdout.contains("results: 2"));

    let experience_filtered = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--years-experience-min",
            "4",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run experience-filtered search");
    assert!(experience_filtered.status.success());
    let experience_filtered_stdout = String::from_utf8_lossy(&experience_filtered.stdout);
    assert!(experience_filtered_stdout.contains("results: 2"));

    let senior_filtered = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--years-experience-min",
            "5",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run senior experience-filtered search");
    assert!(senior_filtered.status.success());
    let senior_filtered_stdout = String::from_utf8_lossy(&senior_filtered.stdout);
    assert!(senior_filtered_stdout.contains("results: 0"));

    remove_dir(&data_dir);
}

fn import_fixtures(data_dir: &Path) {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes");
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
        ])
        .output()
        .expect("import fixtures");
    assert!(output.status.success());
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s10-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
