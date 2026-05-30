use fs_crawler::{ScanErrorKind, scan_directory, should_skip_file, supported_extension};
use std::fs;
use std::path::Path;

fn unique_fixture_dir(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "resume_ir_fs_crawler_{name}_{}",
        std::process::id()
    ));
    if root.exists() {
        fs::remove_dir_all(&root).expect("clean old fixture");
    }
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

#[test]
fn scans_supported_files_with_chinese_paths_and_duplicate_names() {
    let root = unique_fixture_dir("scan");
    let nested_a = root.join("候选人A");
    let nested_b = root.join("候选人B");
    fs::create_dir_all(&nested_a).expect("create nested a");
    fs::create_dir_all(&nested_b).expect("create nested b");
    fs::write(nested_a.join("简历.docx"), "java backend").expect("write docx");
    fs::write(nested_b.join("简历.docx"), "rust backend").expect("write second docx");
    fs::write(root.join("notes.txt"), "plain text").expect("write txt");

    let entries = scan_directory(&root).expect("scan directory");
    let paths: Vec<&str> = entries
        .iter()
        .map(|entry| entry.normalized_path.as_str())
        .collect();

    assert_eq!(entries.len(), 3);
    assert!(paths.iter().any(|path| path.contains("候选人A/简历.docx")));
    assert!(paths.iter().any(|path| path.contains("候选人B/简历.docx")));
    assert!(entries[0].fingerprint.byte_size > 0);
}

#[test]
fn filters_temp_files_and_unsupported_extensions() {
    assert!(should_skip_file(Path::new("~$resume.docx")));
    assert!(should_skip_file(Path::new(".DS_Store")));
    assert!(supported_extension(Path::new("resume.pdf")));
    assert!(supported_extension(Path::new("resume.DOCX")));
    assert!(!supported_extension(Path::new("resume.tmp")));

    let root = unique_fixture_dir("filter");
    fs::write(root.join("~$resume.docx"), "temp").expect("write temp");
    fs::write(root.join("resume.tmp"), "tmp").expect("write unsupported");
    fs::write(root.join("resume.pdf"), "pdf text").expect("write pdf");

    let entries = scan_directory(&root).expect("scan directory");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].file_name, "resume.pdf");
}

#[test]
fn missing_root_returns_unreachable_error() {
    let root = unique_fixture_dir("missing");
    fs::remove_dir_all(&root).expect("remove root");

    let error = scan_directory(&root).expect_err("missing root should fail");

    assert_eq!(error.kind, ScanErrorKind::Unreachable);
    assert!(error.retryable);
}

#[test]
fn normalizes_windows_separators() {
    let normalized = fs_crawler::normalize_path_str(r"C:\resumes\候选人\resume.pdf");

    assert_eq!(normalized, "C:/resumes/候选人/resume.pdf");
}
