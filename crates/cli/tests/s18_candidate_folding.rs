mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use support::{assert_import_succeeded, import_text_resumes};

#[test]
fn search_folds_exact_versions_assigned_to_the_same_candidate() {
    let data_dir = temp_dir("candidate-folding-data");
    let source_root = temp_dir("candidate-folding-source");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[
            (
                "same-candidate-a.txt",
                resume_text(
                    "Same Candidate A",
                    Some("same-candidate@example.test"),
                    Some("155-555-0101"),
                    "Java Java backend",
                ),
            ),
            (
                "same-candidate-b.txt",
                resume_text(
                    "Same Candidate B",
                    Some("same-candidate@example.test"),
                    Some("155-555-0101"),
                    "Java backend search",
                ),
            ),
            (
                "distinct-candidate.txt",
                resume_text(
                    "Distinct Candidate",
                    Some("distinct@example.test"),
                    Some("155-555-0202"),
                    "Java backend observability",
                ),
            ),
        ],
    ));

    let output = search(&data_dir, "Java");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 2"));
    assert_eq!(
        usize::from(stdout.contains("same-candidate-a.txt"))
            + usize::from(stdout.contains("same-candidate-b.txt")),
        1
    );
    assert!(stdout.contains("distinct-candidate.txt"));
    assert!(!stdout.contains("same-candidate@example.test"));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

#[test]
fn candidate_review_list_is_read_only_and_redacted() {
    let data_dir = temp_dir("candidate-review-data");
    let source_root = temp_dir("candidate-review-source");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[
            (
                "review-a.txt",
                resume_text("Private Review Candidate", None, None, "Java payments"),
            ),
            (
                "review-b.txt",
                resume_text("Private Review Candidate", None, None, "Java search"),
            ),
        ],
    ));

    let list = candidate_review(&data_dir, &["list", "--limit", "5"]);
    assert!(
        list.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&list.stdout),
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("candidate review suggestions:"));
    assert!(stdout.contains("paths: <redacted>") || stdout.contains("suggestions: 0"));
    assert!(!stdout.contains("Private Review Candidate"));
    assert!(!stdout.contains(path_str(&source_root)));

    for removed_action in ["merge", "split"] {
        let rejected = candidate_review(&data_dir, &[removed_action]);
        assert!(!rejected.status.success());
        assert!(rejected.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&rejected.stderr).contains("Private Review Candidate"));
    }

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

#[test]
fn candidate_review_conflicts_redacts_contact_values_and_hashes() {
    let data_dir = temp_dir("candidate-conflict-data");
    let source_root = temp_dir("candidate-conflict-source");
    assert_import_succeeded(&import_text_resumes(
        &data_dir,
        &source_root,
        &[
            (
                "01-email-owner.txt",
                resume_text(
                    "Email Owner",
                    Some("shared-conflict@example.test"),
                    None,
                    "Rust",
                ),
            ),
            (
                "02-phone-owner.txt",
                resume_text("Phone Owner", None, Some("155-555-0303"), "Rust"),
            ),
            (
                "03-conflicting-owner.txt",
                resume_text(
                    "Conflicting Owner",
                    Some("shared-conflict@example.test"),
                    Some("155-555-0303"),
                    "Rust",
                ),
            ),
        ],
    ));

    let conflicts = candidate_review(&data_dir, &["conflicts", "--limit", "5"]);
    assert!(conflicts.status.success());
    let stdout = String::from_utf8_lossy(&conflicts.stdout);
    assert!(stdout.contains("candidate contact conflicts:"));
    assert!(!stdout.contains("shared-conflict@example.test"));
    assert!(!stdout.contains("155-555-0303"));
    assert!(!stdout.contains(path_str(&source_root)));

    remove_dir(&data_dir);
    remove_dir(&source_root);
}

fn resume_text(name: &str, email: Option<&str>, phone: Option<&str>, skills: &str) -> String {
    let email = email
        .map(|value| format!("Email: {value}\n"))
        .unwrap_or_default();
    let phone = phone
        .map(|value| format!("Phone: {value}\n"))
        .unwrap_or_default();
    format!("SUMMARY\n{name}\n{email}{phone}EXPERIENCE\nBuilt {skills} systems\nSKILLS\n{skills}")
}

fn search(data_dir: &Path, query: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "search",
            query,
            "--top-k",
            "10",
        ])
        .output()
        .expect("run candidate-folding search")
}

fn candidate_review(data_dir: &Path, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_resume-cli"));
    command.args(["--data-dir", path_str(data_dir), "candidate-review"]);
    command.args(args);
    command.output().expect("run candidate-review command")
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s18-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
