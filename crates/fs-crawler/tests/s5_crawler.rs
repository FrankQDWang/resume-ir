use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use core_domain::FileExtension;
use fs_crawler::{
    crawl_directory, crawl_with_fs, crawl_with_fs_profile, normalize_path, CrawlErrorKind,
    FileSystem, FsEntry, FsEntryKind, FsMetadata, FsOperation, ScanBudgetKind, ScanControl,
    ScanOptions, ScanProfile, MAX_TOTAL_SAMPLE_BYTES,
};

#[test]
fn normalizes_chinese_and_mixed_separator_paths_without_display_leakage() {
    let normalized = normalize_path(r"C:\\候选人//张三/./简历.docx").unwrap();

    assert_eq!(normalized.as_str(), "c:/候选人/张三/简历.docx");
    assert_eq!(normalized.file_name(), Some("简历.docx"));
    assert_eq!(normalized.extension().as_deref(), Some("docx"));
    assert_eq!(
        normalize_path(r"\\server\share//候选人/../简历.pdf")
            .unwrap()
            .as_str(),
        "//server/share/简历.pdf"
    );
    assert_eq!(
        normalize_path(r"\\server\share\..\x.pdf").unwrap().as_str(),
        "//server/share/x.pdf"
    );
    assert_eq!(
        normalize_path(r"C:relative\简历.pdf").unwrap().as_str(),
        "c:relative/简历.pdf"
    );
    assert_eq!(
        normalize_path(r"C:\absolute\简历.pdf").unwrap().as_str(),
        "c:/absolute/简历.pdf"
    );
    assert!(!normalized.to_string().contains("张三"));
    assert!(!format!("{normalized:?}").contains("简历"));

    let entry_debug = format!(
        "{:?}",
        FsEntry::new(PathBuf::from("/fixture/候选人/张三.pdf"), FsEntryKind::File)
    );
    assert!(!entry_debug.contains("张三"));
    assert!(!entry_debug.contains("/fixture"));
}

#[cfg(unix)]
#[test]
fn rejects_non_utf8_paths_without_lossy_replacement() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let path = PathBuf::from(OsStr::from_bytes(b"/fixture/\xff/resume.pdf"));

    assert!(normalize_path(path).is_err());
}

#[test]
fn scans_chinese_paths_and_distinguishes_same_file_names_by_path_and_fingerprint() {
    let root = TestDir::new("fs-crawler-chinese");
    root.write("候选人一/resume.pdf", b"synthetic pdf one");
    root.write("候选人二/resume.pdf", b"synthetic pdf two");
    root.write("候选人三/张三_简历.DOCX", b"synthetic docx");

    let report = crawl_directory(root.path()).unwrap();

    assert_eq!(report.errors.len(), 0);
    assert_eq!(report.files.len(), 3);

    let same_name_files = report
        .files
        .iter()
        .filter(|file| file.file_name == "resume.pdf")
        .collect::<Vec<_>>();
    assert_eq!(same_name_files.len(), 2);
    assert_ne!(
        same_name_files[0].normalized_path,
        same_name_files[1].normalized_path
    );
    assert_ne!(
        same_name_files[0].document_id,
        same_name_files[1].document_id
    );

    let docx = report
        .files
        .iter()
        .find(|file| file.file_name == "张三_简历.DOCX")
        .unwrap();
    assert_eq!(docx.extension, FileExtension::Docx);
    assert!(docx
        .normalized_path
        .as_str()
        .contains("候选人三/张三_简历.DOCX"));
    assert!(!format!("{docx:?}").contains("张三"));
}

#[test]
fn filters_temporary_hidden_and_unsupported_files() {
    let root = TestDir::new("fs-crawler-filter");
    root.write("keep/notes.txt", b"synthetic notes");
    root.write("keep/report.pdf", b"%PDF synthetic");
    root.write("~$draft.docx", b"temporary office lock");
    root.write(".DS_Store", b"mac metadata");
    root.write("scratch.tmp", b"temporary file");
    root.write(".hidden.pdf", b"hidden garbage");
    root.write(".cache/resume.pdf", b"hidden directory");
    root.write("keep/.git/report.docx", b"nested hidden directory");
    root.write("image.png", b"unsupported");

    let report = crawl_directory(root.path()).unwrap();
    let names = report
        .files
        .iter()
        .map(|file| file.file_name.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(names, BTreeSet::from(["notes.txt", "report.pdf"]));
    assert_eq!(report.ignored_count, 7);
}

#[test]
fn discovery_profile_skips_system_cache_and_dependency_directories() {
    let fs = FakeFileSystem::new()
        .dir("/")
        .dir("/System")
        .file("/System/system-resume.pdf", b"%PDF system noise")
        .dir("/usr")
        .file("/usr/share-resume.pdf", b"%PDF usr noise")
        .dir("/Users")
        .dir("/Users/frank")
        .dir("/Users/frank/Library")
        .dir("/Users/frank/Library/Caches")
        .file(
            "/Users/frank/Library/Caches/cached-resume.pdf",
            b"%PDF cache noise",
        )
        .dir("/Users/frank/Documents")
        .file(
            "/Users/frank/Documents/resume.pdf",
            b"%PDF synthetic resume",
        )
        .dir("/Users/frank/Documents/Target")
        .file(
            "/Users/frank/Documents/Target/candidate-resume.pdf",
            b"%PDF candidate target",
        )
        .dir("/Users/frank/project")
        .dir("/Users/frank/project/node_modules")
        .file(
            "/Users/frank/project/node_modules/dependency-resume.pdf",
            b"%PDF dependency noise",
        )
        .dir("/Users/frank/project/target")
        .file(
            "/Users/frank/project/target/build-resume.pdf",
            b"%PDF build noise",
        );

    let explicit = crawl_with_fs(&fs, Path::new("/")).unwrap();
    assert_eq!(explicit.files.len(), 7);

    let discovery = crawl_with_fs_profile(&fs, Path::new("/"), ScanProfile::Discovery).unwrap();
    let discovered_paths = discovery
        .files
        .iter()
        .map(|file| file.normalized_path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        discovered_paths,
        vec![
            "/Users/frank/Documents/Target/candidate-resume.pdf",
            "/Users/frank/Documents/resume.pdf",
        ]
    );
    assert!(discovery.errors.is_empty());
    assert!(discovery.ignored_count >= 5);
}

#[test]
fn scan_options_stop_after_file_budget_without_path_leakage() {
    let fs = FakeFileSystem::new()
        .dir("/fixture")
        .file("/fixture/a.docx", b"synthetic a")
        .file("/fixture/b.pdf", b"synthetic b")
        .file("/fixture/c.pdf", b"synthetic c");

    let report = fs_crawler::crawl_with_fs_options(
        &fs,
        Path::new("/fixture"),
        ScanOptions {
            profile: ScanProfile::Explicit,
            max_files: Some(2),
        },
    )
    .unwrap();

    assert_eq!(report.files.len(), 2);
    assert!(report.errors.is_empty());
    let budget = report.budget_exhausted.expect("file budget exhausted");
    assert_eq!(budget.kind, ScanBudgetKind::Files);
    assert_eq!(budget.limit, 2);
    assert_eq!(budget.observed, 2);
    assert!(!format!("{budget:?}").contains("/fixture"));
}

#[test]
fn scan_control_cancels_directory_walk_without_path_leakage() {
    let fs = FakeFileSystem::new()
        .dir("/fixture")
        .dir("/fixture/a")
        .file("/fixture/a/resume-a.pdf", b"%PDF synthetic a")
        .dir("/fixture/b")
        .file("/fixture/b/resume-b.pdf", b"%PDF synthetic b");
    let checks = Cell::new(0);
    let should_cancel = || {
        let previous = checks.get();
        checks.set(previous + 1);
        previous >= 2
    };

    let error = fs_crawler::crawl_with_fs_options_and_control(
        &fs,
        Path::new("/fixture"),
        ScanOptions {
            profile: ScanProfile::Explicit,
            max_files: None,
        },
        ScanControl::from_cancel_check(&should_cancel),
    )
    .unwrap_err();

    assert_eq!(error.kind, CrawlErrorKind::Cancelled);
    assert_eq!(error.operation, FsOperation::CheckCancellation);
    assert!(error.normalized_path().is_none());
    assert!(!format!("{error:?}").contains("/fixture"));
}

#[test]
fn scan_control_cancels_during_fingerprint_without_path_leakage() {
    let fs = FakeFileSystem::new()
        .dir("/fixture")
        .file("/fixture/resume.pdf", b"%PDF synthetic resume");
    let checks = Cell::new(0);
    let should_cancel = || {
        let previous = checks.get();
        checks.set(previous + 1);
        previous >= 5
    };

    let error = fs_crawler::crawl_with_fs_options_and_control(
        &fs,
        Path::new("/fixture"),
        ScanOptions {
            profile: ScanProfile::Explicit,
            max_files: None,
        },
        ScanControl::from_cancel_check(&should_cancel),
    )
    .unwrap_err();

    assert_eq!(error.kind, CrawlErrorKind::Cancelled);
    assert_eq!(error.operation, FsOperation::CheckCancellation);
    assert!(error.normalized_path().is_none());
    assert!(!format!("{error:?}").contains("resume.pdf"));
}

#[test]
fn quick_fingerprint_samples_head_and_tail_without_reading_entire_large_file() {
    let root = TestDir::new("fs-crawler-fingerprint");
    let mut content = vec![b'a'; (MAX_TOTAL_SAMPLE_BYTES as usize) * 4];
    let last_index = content.len() - 1;
    content[last_index] = b'z';
    root.write("large.txt", &content);

    let report = crawl_directory(root.path()).unwrap();
    let file = report
        .files
        .iter()
        .find(|file| file.file_name == "large.txt")
        .unwrap();

    assert!(file.fingerprint.sampled_bytes <= MAX_TOTAL_SAMPLE_BYTES);
    assert!(file.byte_size > file.fingerprint.sampled_bytes);
    assert!(file.fingerprint.as_str().starts_with("qfp_"));
    assert_eq!(file.fingerprint.to_string(), "<redacted-fingerprint>");
    assert!(!format!("{:?}", file.fingerprint).contains(file.fingerprint.as_str()));
    assert!(!format!("{file:?}").contains(file.fingerprint.as_str()));
}

#[test]
fn classifies_permission_unavailable_and_locked_errors_with_fake_filesystem() {
    let fs = FakeFileSystem::new()
        .dir("/fixture")
        .file("/fixture/readable.txt", b"readable synthetic text")
        .file_with_open_error(
            "/fixture/locked.pdf",
            12,
            io::ErrorKind::WouldBlock,
            "locked by another process",
        )
        .entry("/fixture/denied", FsEntryKind::Directory)
        .read_dir_error(
            "/fixture/denied",
            io::ErrorKind::PermissionDenied,
            "permission denied",
        )
        .entry("/fixture/missing.docx", FsEntryKind::File)
        .metadata_error(
            "/fixture/missing.docx",
            io::ErrorKind::NotFound,
            "source unavailable",
        );

    let report = crawl_with_fs(&fs, Path::new("/fixture")).unwrap();
    let error_kinds = report
        .errors
        .iter()
        .map(|error| error.kind)
        .collect::<BTreeSet<_>>();

    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].file_name, "readable.txt");
    assert_eq!(
        error_kinds,
        BTreeSet::from([
            CrawlErrorKind::LockedOrUnreadable,
            CrawlErrorKind::PermissionDenied,
            CrawlErrorKind::SourceUnavailable,
        ])
    );

    let debug = format!("{:?}", report.errors);
    assert!(!debug.contains("/fixture"));
    assert!(!debug.contains("missing.docx"));
}

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let unique = format!(
            "{}-{}-{}",
            label,
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn write(&self, relative: &str, bytes: &[u8]) {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Default)]
struct FakeFileSystem {
    entries: BTreeMap<PathBuf, FakeNode>,
    read_dir_errors: BTreeMap<PathBuf, FakeIoError>,
    metadata_errors: BTreeMap<PathBuf, FakeIoError>,
    open_errors: BTreeMap<PathBuf, FakeIoError>,
}

impl FakeFileSystem {
    fn new() -> Self {
        Self::default()
    }

    fn dir(mut self, path: &str) -> Self {
        self.entries.insert(
            PathBuf::from(path),
            FakeNode {
                kind: FsEntryKind::Directory,
                bytes: Vec::new(),
            },
        );
        self
    }

    fn entry(mut self, path: &str, kind: FsEntryKind) -> Self {
        self.entries.insert(
            PathBuf::from(path),
            FakeNode {
                kind,
                bytes: Vec::new(),
            },
        );
        self
    }

    fn file(mut self, path: &str, bytes: &[u8]) -> Self {
        self.entries.insert(
            PathBuf::from(path),
            FakeNode {
                kind: FsEntryKind::File,
                bytes: bytes.to_vec(),
            },
        );
        self
    }

    fn file_with_open_error(
        mut self,
        path: &str,
        len: u64,
        kind: io::ErrorKind,
        message: &str,
    ) -> Self {
        self.entries.insert(
            PathBuf::from(path),
            FakeNode {
                kind: FsEntryKind::File,
                bytes: vec![0; len as usize],
            },
        );
        self.open_errors
            .insert(PathBuf::from(path), FakeIoError::new(kind, message));
        self
    }

    fn read_dir_error(mut self, path: &str, kind: io::ErrorKind, message: &str) -> Self {
        self.read_dir_errors
            .insert(PathBuf::from(path), FakeIoError::new(kind, message));
        self
    }

    fn metadata_error(mut self, path: &str, kind: io::ErrorKind, message: &str) -> Self {
        self.metadata_errors
            .insert(PathBuf::from(path), FakeIoError::new(kind, message));
        self
    }
}

impl FileSystem for FakeFileSystem {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<FsEntry>> {
        if let Some(error) = self.read_dir_errors.get(path) {
            return Err(error.to_io_error());
        }

        let mut children = Vec::new();
        for (candidate, node) in &self.entries {
            if candidate.parent() == Some(path) {
                children.push(FsEntry::new(candidate.clone(), node.kind));
            }
        }
        Ok(children)
    }

    fn metadata(&self, path: &Path) -> io::Result<FsMetadata> {
        if let Some(error) = self.metadata_errors.get(path) {
            return Err(error.to_io_error());
        }

        let node = self.entries.get(path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "fake filesystem node is missing")
        })?;

        Ok(FsMetadata::new(
            node.kind,
            node.bytes.len() as u64,
            UNIX_EPOCH + Duration::from_secs(1_800_000_000),
        ))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn fs_crawler::ReadSeek>> {
        if let Some(error) = self.open_errors.get(path) {
            return Err(error.to_io_error());
        }

        let node = self.entries.get(path).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "fake filesystem node is missing")
        })?;

        Ok(Box::new(Cursor::new(node.bytes.clone())))
    }
}

#[derive(Clone)]
struct FakeNode {
    kind: FsEntryKind,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct FakeIoError {
    kind: io::ErrorKind,
    message: String,
}

impl FakeIoError {
    fn new(kind: io::ErrorKind, message: &str) -> Self {
        Self {
            kind,
            message: message.to_string(),
        }
    }

    fn to_io_error(&self) -> io::Error {
        io::Error::new(self.kind, self.message.clone())
    }
}
