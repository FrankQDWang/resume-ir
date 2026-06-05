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

#[test]
fn filtered_search_prefilters_fields_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-filter-prefilter-data");
    let resume_root = temp_dir("search-filter-prefilter-resumes");
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    for index in 0..5 {
        fs::write(
            resume_root.join(format!("decoy-{index}.txt")),
            format!("Candidate Decoy {index}\nSkills: Java\n{noisy_query_text}\n"),
        )
        .unwrap();
    }
    fs::write(
        resume_root.join("target-rust-candidate.txt"),
        "Candidate Target\nSkills: Rust\nneedle\n",
    )
    .unwrap();

    import_root(&data_dir, &resume_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "needle",
            "--skills-any",
            "rust",
            "--top-k",
            "1",
        ])
        .output()
        .expect("run prefiltered skill search");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains("target-rust-candidate.txt"));
    assert!(!stdout.contains("decoy-"));
    assert!(!stdout.contains("query:"));

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn filtered_search_prefilters_unknown_school_tier_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-filter-unknown-tier-data");
    let resume_root = temp_dir("search-filter-unknown-tier-resumes");
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    for index in 0..5 {
        fs::write(
            resume_root.join(format!("known-tier-decoy-{index}.txt")),
            format!(
                "\
Candidate Known Tier Decoy {index}
Education
School: Synthetic 985 University (985/211/dual first class)
Skills: Java
{noisy_query_text}
"
            ),
        )
        .unwrap();
    }
    fs::write(
        resume_root.join("unknown-tier-target.txt"),
        "\
Candidate Unknown Tier Target
Education
School: Synthetic Regional College
Skills: Java
needle
",
    )
    .unwrap();

    import_root(&data_dir, &resume_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "needle",
            "--school-tier",
            "unknown",
            "--top-k",
            "1",
        ])
        .output()
        .expect("run unknown school-tier filtered search");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains("unknown-tier-target.txt"));
    assert!(!stdout.contains("known-tier-decoy-"));
    assert!(!stdout.contains("query:"));

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn filtered_search_prefilters_certificates_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-filter-certificate-data");
    let resume_root = temp_dir("search-filter-certificate-resumes");
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    for index in 0..5 {
        fs::write(
            resume_root.join(format!("certificate-decoy-{index}.txt")),
            format!(
                "\
Candidate Certificate Decoy {index}
Skills: Java
{noisy_query_text}
"
            ),
        )
        .unwrap();
    }
    fs::write(
        resume_root.join("certificate-target.txt"),
        "\
Candidate Certificate Target
Certifications
PMP
Skills: Java
needle
",
    )
    .unwrap();

    import_root(&data_dir, &resume_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "needle",
            "--certificate",
            "PMP",
            "--top-k",
            "1",
        ])
        .output()
        .expect("run certificate filtered search");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains("certificate-target.txt"));
    assert!(!stdout.contains("certificate-decoy-"));
    assert!(!stdout.contains("query:"));

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn filtered_search_prefilters_company_and_title_before_fulltext_top_k_cutoff() {
    let data_dir = temp_dir("search-filter-company-title-data");
    let resume_root = temp_dir("search-filter-company-title-resumes");
    let noisy_query_text = std::iter::repeat_n("needle", 100)
        .collect::<Vec<_>>()
        .join(" ");
    for index in 0..5 {
        fs::write(
            resume_root.join(format!("role-decoy-{index}.txt")),
            format!(
                "\
Candidate Role Decoy {index}
Experience
Synthetic Search Inc.
Product Manager
Skills: Java
{noisy_query_text}
"
            ),
        )
        .unwrap();
    }
    fs::write(
        resume_root.join("role-target.txt"),
        "\
Candidate Role Target
Experience
Synthetic Payments Inc.
Senior Backend Engineer
Skills: Java
needle
",
    )
    .unwrap();

    import_root(&data_dir, &resume_root);

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "needle",
            "--company",
            "Synthetic Payments Inc.",
            "--title",
            "Backend Engineer",
            "--top-k",
            "1",
        ])
        .output()
        .expect("run company-title filtered search");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(stdout.contains("role-target.txt"));
    assert!(!stdout.contains("role-decoy-"));
    assert!(!stdout.contains("query:"));

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

fn import_fixtures(data_dir: &Path) {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes");
    import_root(data_dir, &fixture_root);
}

fn import_root(data_dir: &Path, root: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(root),
        ])
        .output()
        .expect("import root");
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
