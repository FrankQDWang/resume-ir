//! Contract tests for filesystem crawling behavior.

use fs_crawler::{
    normalize_path, CrawlErrorKind, Crawler, DirEntryInfo, FileMetadata, FileSource, SourceError,
    SourceErrorKind, SupportedExtension,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn scan_discovers_supported_files_with_chinese_paths_and_same_names() -> TestResult {
    let temp = tempdir()?;
    let root = temp.path();

    write_file(
        root.join("目录甲").join("样例一").join("文档.pdf"),
        b"pdf-one",
    )?;
    write_file(
        root.join("目录甲").join("样例二").join("文档.pdf"),
        b"pdf-two",
    )?;
    write_file(
        root.join("目录甲").join("样例二").join("photo.JPG"),
        b"image",
    )?;
    write_file(root.join("notes.md"), b"unsupported")?;

    let report = Crawler::new().scan(root);

    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.files.len(), 3);

    let paths: Vec<_> = report
        .files
        .iter()
        .map(|file| file.normalized_path.as_str().to_owned())
        .collect();
    assert!(paths
        .iter()
        .any(|path| path.contains("目录甲/样例一/文档.pdf")));
    assert!(paths
        .iter()
        .any(|path| path.contains("目录甲/样例二/文档.pdf")));

    let resume_files: Vec<_> = report
        .files
        .iter()
        .filter(|file| file.file_name == "文档.pdf")
        .collect();
    assert_eq!(resume_files.len(), 2);
    assert_ne!(
        resume_files[0].fingerprint.path_key,
        resume_files[1].fingerprint.path_key
    );
    assert!(report
        .files
        .iter()
        .any(|file| file.extension == SupportedExtension::Image));
    Ok(())
}

#[test]
fn scan_filters_temporary_and_unsupported_files() -> TestResult {
    let temp = tempdir()?;
    let root = temp.path();

    write_file(root.join("resume.docx"), b"ok")?;
    write_file(root.join("~$resume.docx"), b"office temp")?;
    write_file(root.join("resume.tmp"), b"tmp")?;
    write_file(root.join("download.pdf.crdownload"), b"partial")?;
    write_file(root.join(".DS_Store"), b"finder")?;
    write_file(root.join(".resume.swp"), b"swap")?;
    write_file(root.join("archive.zip"), b"zip")?;

    let report = Crawler::new().scan(root);

    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].file_name, "resume.docx");
    assert_eq!(report.files[0].extension, SupportedExtension::Docx);
    Ok(())
}

#[test]
fn normalize_path_is_unicode_preserving_and_separator_stable() {
    let normalized = normalize_path(r"X:\synthetic\目录甲\.\样例一\..\样例二\文档.PDF");

    assert_eq!(normalized.as_str(), "X:/synthetic/目录甲/样例二/文档.PDF");
}

#[test]
fn scan_filters_windows_style_temp_paths_before_extension_detection() {
    let source = FakeSource::new()
        .with_dir(
            "X:/synthetic/root",
            vec![
                DirEntryInfo::file(r"X:\synthetic\root\~$draft.docx"),
                DirEntryInfo::file(r"X:\synthetic\root\resume.docx"),
            ],
        )
        .with_file(r"X:\synthetic\root\~$draft.docx", b"temp")
        .with_file(r"X:\synthetic\root\resume.docx", b"ok");

    let report = Crawler::with_source(source).scan(Path::new("X:/synthetic/root"));

    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].file_name, "resume.docx");
}

#[test]
fn fingerprints_include_path_size_mtime_and_content_sample() -> TestResult {
    let temp = tempdir()?;
    let root = temp.path();
    write_file(root.join("resume.txt"), b"abcdefghijklmnopqrstuvwxyz")?;

    let first = Crawler::new().scan(root);
    let first_file = first
        .files
        .first()
        .ok_or("expected first scan to discover resume.txt")?;

    assert_eq!(first_file.fingerprint.size_bytes, 26);
    assert!(first_file.fingerprint.mtime_millis > 0);
    assert!(first_file.fingerprint.sample_hash.len() >= 32);
    assert!(first_file.fingerprint.path_key.ends_with("/resume.txt"));
    let debug = format!("{:?}", first_file.fingerprint);
    assert!(!debug.contains(&first_file.fingerprint.sample_hash));

    write_file(root.join("resume.txt"), b"abc")?;
    let second = Crawler::new().scan(root);
    let second_file = second
        .files
        .first()
        .ok_or("expected second scan to discover resume.txt")?;

    assert_ne!(
        first_file.fingerprint.sample_hash,
        second_file.fingerprint.sample_hash
    );
    assert_ne!(
        first_file.fingerprint.size_bytes,
        second_file.fingerprint.size_bytes
    );
    Ok(())
}

#[test]
fn scan_represents_permission_locked_and_unreachable_source_errors() {
    let source = FakeSource::new()
        .with_dir(
            "/synthetic-crawl-root",
            vec![
                DirEntryInfo::file("/synthetic-crawl-root/ok.pdf"),
                DirEntryInfo::file("/synthetic-crawl-root/locked.pdf"),
                DirEntryInfo::file("/synthetic-crawl-root/private.pdf"),
                DirEntryInfo::dir("/synthetic-crawl-root/external"),
            ],
        )
        .with_file("/synthetic-crawl-root/ok.pdf", b"ok")
        .with_error("/synthetic-crawl-root/locked.pdf", SourceErrorKind::Locked)
        .with_error(
            "/synthetic-crawl-root/private.pdf",
            SourceErrorKind::PermissionDenied,
        )
        .with_error(
            "/synthetic-crawl-root/external",
            SourceErrorKind::Unreachable,
        );

    let report = Crawler::with_source(source).scan(Path::new("/synthetic-crawl-root"));

    assert_eq!(report.files.len(), 1);
    assert_eq!(report.errors.len(), 3);
    assert!(report
        .errors
        .iter()
        .any(|error| error.kind == CrawlErrorKind::Locked));
    assert!(report
        .errors
        .iter()
        .any(|error| error.kind == CrawlErrorKind::PermissionDenied));
    assert!(report
        .errors
        .iter()
        .any(|error| error.kind == CrawlErrorKind::Unreachable));

    let debug = format!("{:?}", report.errors[0]);
    assert!(debug.contains("[redacted local path]"));
    assert!(!debug.contains("/synthetic-crawl-root"));
}

fn write_file(path: PathBuf, contents: &[u8]) -> TestResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, contents)?;
    Ok(())
}

#[derive(Clone, Default)]
struct FakeSource {
    dirs: BTreeMap<PathBuf, Vec<DirEntryInfo>>,
    files: BTreeMap<PathBuf, Vec<u8>>,
    errors: BTreeMap<PathBuf, SourceErrorKind>,
}

impl FakeSource {
    fn new() -> Self {
        Self::default()
    }

    fn with_dir(mut self, path: &str, entries: Vec<DirEntryInfo>) -> Self {
        self.dirs.insert(PathBuf::from(path), entries);
        self
    }

    fn with_file(mut self, path: &str, contents: &[u8]) -> Self {
        self.files.insert(PathBuf::from(path), contents.to_vec());
        self
    }

    fn with_error(mut self, path: &str, kind: SourceErrorKind) -> Self {
        self.errors.insert(PathBuf::from(path), kind);
        self
    }

    fn maybe_error(&self, path: &Path) -> Result<(), SourceError> {
        match self.errors.get(path) {
            Some(kind) => Err(SourceError::new(*kind)),
            None => Ok(()),
        }
    }
}

impl FileSource for FakeSource {
    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, SourceError> {
        self.maybe_error(path)?;
        Ok(self.dirs.get(path).cloned().unwrap_or_default())
    }

    fn metadata(&self, path: &Path) -> Result<FileMetadata, SourceError> {
        self.maybe_error(path)?;
        let len = self
            .files
            .get(path)
            .map_or(0, |contents| contents.len() as u64);
        Ok(FileMetadata {
            len,
            modified: SystemTime::UNIX_EPOCH + Duration::from_secs(42),
        })
    }

    fn read_sample(&self, path: &Path, _max_bytes: usize) -> Result<Vec<u8>, SourceError> {
        self.maybe_error(path)?;
        Ok(self.files.get(path).cloned().unwrap_or_default())
    }
}
