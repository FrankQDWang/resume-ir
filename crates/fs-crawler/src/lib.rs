//! Local filesystem crawling for resume source discovery.

use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use thiserror::Error;

const SAMPLE_CHUNK_BYTES: usize = 4096;

/// Crawls local filesystem roots for supported source documents.
#[derive(Clone, Debug)]
pub struct Crawler<S = StdFileSource> {
    source: S,
}

impl Crawler<StdFileSource> {
    /// Creates a crawler backed by the host filesystem.
    #[must_use]
    pub fn new() -> Self {
        Self {
            source: StdFileSource,
        }
    }
}

impl Default for Crawler<StdFileSource> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Crawler<S>
where
    S: FileSource,
{
    /// Creates a crawler backed by an injected source.
    #[must_use]
    pub fn with_source(source: S) -> Self {
        Self { source }
    }

    /// Recursively scans a directory and returns discovered files plus recoverable errors.
    #[must_use]
    pub fn scan(&self, root: &Path) -> ScanReport {
        let mut report = ScanReport::default();
        self.scan_path(root, &mut report);
        report
            .files
            .sort_by(|left, right| left.normalized_path.cmp(&right.normalized_path));
        report
    }

    fn scan_path(&self, path: &Path, report: &mut ScanReport) {
        let entries = match self.source.read_dir(path) {
            Ok(entries) => entries,
            Err(error) => {
                report.errors.push(CrawlError::from_source(path, error));
                return;
            }
        };

        for entry in entries {
            match entry.kind {
                FileKind::Directory => self.scan_path(&entry.path, report),
                FileKind::File => self.scan_file(&entry.path, report),
            }
        }
    }

    fn scan_file(&self, path: &Path, report: &mut ScanReport) {
        let normalized_path = normalize_path(path.to_string_lossy());
        let Some(file_name) = file_name(normalized_path.as_str()) else {
            return;
        };

        if is_temporary_file(&file_name) {
            return;
        }

        let Some(extension) = SupportedExtension::from_normalized_path(normalized_path.as_str())
        else {
            return;
        };

        let metadata = match self.source.metadata(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                report.errors.push(CrawlError::from_source(path, error));
                return;
            }
        };

        let sample = match self.source.read_sample(path, SAMPLE_CHUNK_BYTES) {
            Ok(sample) => sample,
            Err(error) => {
                report.errors.push(CrawlError::from_source(path, error));
                return;
            }
        };

        let fingerprint = FastFingerprint::new(&normalized_path, &metadata, &sample);

        report.files.push(DiscoveredFile {
            normalized_path,
            file_name,
            extension,
            fingerprint,
        });
    }
}

/// Host filesystem adapter used by the default crawler.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdFileSource;

impl FileSource for StdFileSource {
    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, SourceError> {
        let mut entries = Vec::new();

        for entry in fs::read_dir(path).map_err(SourceError::from_io)? {
            let entry = entry.map_err(SourceError::from_io)?;
            let file_type = entry.file_type().map_err(SourceError::from_io)?;
            let kind = if file_type.is_dir() {
                FileKind::Directory
            } else if file_type.is_file() {
                FileKind::File
            } else {
                continue;
            };
            entries.push(DirEntryInfo {
                path: entry.path(),
                kind,
            });
        }

        Ok(entries)
    }

    fn metadata(&self, path: &Path) -> Result<FileMetadata, SourceError> {
        let metadata = fs::metadata(path).map_err(SourceError::from_io)?;
        let modified = metadata.modified().map_err(SourceError::from_io)?;

        Ok(FileMetadata {
            len: metadata.len(),
            modified,
        })
    }

    fn read_sample(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, SourceError> {
        let mut file = fs::File::open(path).map_err(SourceError::from_io)?;
        let len = file.metadata().map_err(SourceError::from_io)?.len();
        let max_bytes_u64 = max_bytes as u64;

        if len <= max_bytes_u64 * 2 {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(SourceError::from_io)?;
            return Ok(bytes);
        }

        let mut head = vec![0; max_bytes];
        file.read_exact(&mut head).map_err(SourceError::from_io)?;

        file.seek(SeekFrom::End(-(max_bytes as i64)))
            .map_err(SourceError::from_io)?;
        let mut tail = vec![0; max_bytes];
        file.read_exact(&mut tail).map_err(SourceError::from_io)?;

        head.extend_from_slice(&tail);
        Ok(head)
    }
}

/// Minimal filesystem source interface used for deterministic tests and failure simulation.
pub trait FileSource {
    /// Lists direct children of a directory.
    fn read_dir(&self, path: &Path) -> Result<Vec<DirEntryInfo>, SourceError>;

    /// Reads file metadata used for fast fingerprinting.
    fn metadata(&self, path: &Path) -> Result<FileMetadata, SourceError>;

    /// Reads a bounded content sample from a file.
    fn read_sample(&self, path: &Path, max_bytes: usize) -> Result<Vec<u8>, SourceError>;
}

/// Directory entry returned by a [`FileSource`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirEntryInfo {
    /// Entry path.
    pub path: PathBuf,
    /// Entry kind.
    pub kind: FileKind,
}

impl DirEntryInfo {
    /// Creates a file entry.
    #[must_use]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: FileKind::File,
        }
    }

    /// Creates a directory entry.
    #[must_use]
    pub fn dir(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            kind: FileKind::Directory,
        }
    }
}

/// File source entry kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    /// Regular file.
    File,
    /// Directory.
    Directory,
}

/// File metadata used by fingerprinting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileMetadata {
    /// File size in bytes.
    pub len: u64,
    /// Last modified time.
    pub modified: SystemTime,
}

/// Complete scan output.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ScanReport {
    /// Supported files discovered during the scan.
    pub files: Vec<DiscoveredFile>,
    /// Recoverable scan errors for inaccessible paths.
    pub errors: Vec<CrawlError>,
}

impl fmt::Debug for ScanReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScanReport")
            .field("files", &self.files)
            .field("errors", &self.errors)
            .finish()
    }
}

/// Supported local source file discovered by the crawler.
#[derive(Clone, Eq, PartialEq)]
pub struct DiscoveredFile {
    /// Normalized path used for dedupe and change detection.
    pub normalized_path: NormalizedPath,
    /// File name only.
    pub file_name: String,
    /// Supported extension class.
    pub extension: SupportedExtension,
    /// Fast non-cryptographic identity data for change detection.
    pub fingerprint: FastFingerprint,
}

impl fmt::Debug for DiscoveredFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveredFile")
            .field("normalized_path", &"[redacted local path]")
            .field("file_name", &"[redacted file name]")
            .field("extension", &self.extension)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

/// Fast file fingerprint suitable as a coarse change-discovery prefilter.
///
/// Large files are sampled from the head and tail for speed. Later ingest stages
/// must verify full content before relying on this as a cryptographic identity.
#[derive(Clone, Eq, PartialEq)]
pub struct FastFingerprint {
    /// Normalized path key.
    pub path_key: String,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Last modified time as milliseconds since Unix epoch.
    pub mtime_millis: u128,
    /// SHA-256 of the bounded content sample.
    pub sample_hash: String,
}

impl FastFingerprint {
    fn new(path: &NormalizedPath, metadata: &FileMetadata, sample: &[u8]) -> Self {
        let mtime_millis = metadata
            .modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis();
        let sample_hash = hex_sha256(sample);

        Self {
            path_key: path.as_str().to_owned(),
            size_bytes: metadata.len,
            mtime_millis,
            sample_hash,
        }
    }
}

impl fmt::Debug for FastFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FastFingerprint")
            .field("path_key", &"[redacted local path]")
            .field("size_bytes", &self.size_bytes)
            .field("mtime_millis", &self.mtime_millis)
            .field("sample_hash", &"[redacted content hash]")
            .finish()
    }
}

/// Normalized local path string.
#[derive(Clone, Eq, Ord, PartialEq, PartialOrd)]
pub struct NormalizedPath(String);

impl NormalizedPath {
    /// Returns the normalized path as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NormalizedPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted local path]")
    }
}

/// Supported source extension classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportedExtension {
    /// Microsoft Word `.docx`.
    Docx,
    /// Portable Document Format `.pdf`.
    Pdf,
    /// Legacy Microsoft Word `.doc`.
    Doc,
    /// Plain text `.txt`.
    Txt,
    /// Common image extension for later OCR routing.
    Image,
}

impl SupportedExtension {
    fn from_normalized_path(path: &str) -> Option<Self> {
        let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match extension.as_str() {
            "docx" => Some(Self::Docx),
            "pdf" => Some(Self::Pdf),
            "doc" => Some(Self::Doc),
            "txt" => Some(Self::Txt),
            "png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp" | "gif" | "webp" => Some(Self::Image),
            _ => None,
        }
    }
}

/// Recoverable crawler error.
#[derive(Clone, Eq, PartialEq)]
pub struct CrawlError {
    /// Path where the error occurred.
    pub path: NormalizedPath,
    /// Crawler-level error kind.
    pub kind: CrawlErrorKind,
}

impl CrawlError {
    fn from_source(path: &Path, error: SourceError) -> Self {
        Self {
            path: normalize_path(path.to_string_lossy()),
            kind: error.kind.into(),
        }
    }
}

impl fmt::Debug for CrawlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CrawlError")
            .field("path", &"[redacted local path]")
            .field("kind", &self.kind)
            .finish()
    }
}

/// Crawler-level inaccessible path reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CrawlErrorKind {
    /// File is locked or temporarily unavailable.
    Locked,
    /// Permission denied.
    PermissionDenied,
    /// Path is unreachable, such as a missing external disk mount.
    Unreachable,
    /// Other source error.
    Other,
}

/// Lower-level source error.
#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
#[error("{kind:?}")]
pub struct SourceError {
    /// Source error kind.
    pub kind: SourceErrorKind,
}

impl SourceError {
    /// Creates a source error.
    #[must_use]
    pub fn new(kind: SourceErrorKind) -> Self {
        Self { kind }
    }

    fn from_io(error: io::Error) -> Self {
        let kind = match error.kind() {
            io::ErrorKind::PermissionDenied => SourceErrorKind::PermissionDenied,
            io::ErrorKind::WouldBlock => SourceErrorKind::Locked,
            io::ErrorKind::NotFound => SourceErrorKind::Unreachable,
            _ => SourceErrorKind::Other,
        };

        Self { kind }
    }
}

/// Lower-level source error kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceErrorKind {
    /// File is locked or temporarily unavailable.
    Locked,
    /// Permission denied.
    PermissionDenied,
    /// Path is unreachable, such as a missing external disk mount.
    Unreachable,
    /// Other source error.
    Other,
}

impl From<SourceErrorKind> for CrawlErrorKind {
    fn from(kind: SourceErrorKind) -> Self {
        match kind {
            SourceErrorKind::Locked => Self::Locked,
            SourceErrorKind::PermissionDenied => Self::PermissionDenied,
            SourceErrorKind::Unreachable => Self::Unreachable,
            SourceErrorKind::Other => Self::Other,
        }
    }
}

/// Normalizes separators and lexical path components without touching the filesystem.
#[must_use]
pub fn normalize_path(path: impl AsRef<str>) -> NormalizedPath {
    let path = path.as_ref().replace('\\', "/");
    let is_absolute = path.starts_with('/');
    let mut parts = Vec::new();

    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }

        if part == ".." {
            let can_pop = parts
                .last()
                .is_some_and(|last: &&str| *last != ".." && !last.ends_with(':'));
            if can_pop {
                parts.pop();
            } else if !is_absolute {
                parts.push(part);
            }
            continue;
        }

        parts.push(part);
    }

    let mut normalized = parts.join("/");
    if is_absolute {
        normalized.insert(0, '/');
    }

    NormalizedPath(normalized)
}

fn file_name(normalized_path: &str) -> Option<String> {
    normalized_path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn is_temporary_file(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();

    file_name.starts_with("~$")
        || file_name == ".DS_Store"
        || lower.ends_with(".tmp")
        || lower.ends_with(".temp")
        || lower.ends_with(".crdownload")
        || lower.ends_with(".part")
        || lower.ends_with(".download")
        || lower.ends_with(".swp")
        || lower.ends_with(".swo")
        || (file_name.starts_with('.') && (lower.contains(".swp") || lower.contains(".swo")))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);

    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut encoded, "{byte:02x}");
    }

    encoded
}
