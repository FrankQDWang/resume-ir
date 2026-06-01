use std::fmt;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use core_domain::{DocumentId, FileExtension, UnixTimestamp};

const FNV_OFFSET_A: u64 = 0xcbf29ce484222325;
const FNV_OFFSET_B: u64 = 0x6c62272e07bb0142;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const SAMPLE_BYTES_PER_EDGE: u64 = 4 * 1024;

pub const MAX_TOTAL_SAMPLE_BYTES: u64 = SAMPLE_BYTES_PER_EDGE * 2;

pub fn crate_name() -> &'static str {
    "fs-crawler"
}

pub fn crawl_directory(root: impl AsRef<Path>) -> Result<ScanReport> {
    let fs = StdFileSystem;
    crawl_with_fs(&fs, root.as_ref())
}

pub fn crawl_directory_with_profile(
    root: impl AsRef<Path>,
    profile: ScanProfile,
) -> Result<ScanReport> {
    let fs = StdFileSystem;
    crawl_with_fs_profile(&fs, root.as_ref(), profile)
}

pub fn crawl_with_fs(file_system: &impl FileSystem, root: &Path) -> Result<ScanReport> {
    crawl_with_fs_profile(file_system, root, ScanProfile::Explicit)
}

pub fn crawl_with_fs_profile(
    file_system: &impl FileSystem,
    root: &Path,
    profile: ScanProfile,
) -> Result<ScanReport> {
    let root_metadata = file_system
        .metadata(root)
        .map_err(|error| CrawlError::from_io(root, FsOperation::ReadMetadata, error))?;

    if root_metadata.kind != FsEntryKind::Directory {
        return Err(CrawlError::new(
            CrawlErrorKind::SourceUnavailable,
            FsOperation::ReadMetadata,
            root,
        ));
    }

    let mut report = ScanReport::default();
    let mut directories = vec![root.to_path_buf()];

    while let Some(directory) = directories.pop() {
        let mut entries = match file_system.read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                report.errors.push(CrawlError::from_io(
                    &directory,
                    FsOperation::ReadDirectory,
                    error,
                ));
                continue;
            }
        };
        if let Ok(normalized_directory) = normalize_path(&directory) {
            report.scanned_directories.push(normalized_directory);
        }

        entries.sort_by_key(|entry| path_sort_key(&entry.path));

        for entry in entries {
            match entry.kind {
                FsEntryKind::Directory => {
                    if ignored_directory(&entry.path, profile, &mut report) {
                        report.ignored_count += 1;
                    } else {
                        directories.push(entry.path);
                    }
                }
                FsEntryKind::File => process_file(file_system, &entry.path, &mut report),
                FsEntryKind::Other => report.ignored_count += 1,
            }
        }
    }

    report
        .files
        .sort_by(|left, right| left.normalized_path.cmp(&right.normalized_path));
    report.scanned_directories.sort();
    report.scanned_directories.dedup();
    report.skipped_directories.sort();
    report.skipped_directories.dedup();
    report.errors.sort_by_key(|error| error.sort_key());

    Ok(report)
}

pub fn normalize_path(
    path: impl AsRef<Path>,
) -> std::result::Result<NormalizedPath, NormalizePathError> {
    let raw = path.as_ref().to_str().ok_or(NormalizePathError)?;
    Ok(NormalizedPath::new(normalize_path_string(raw)))
}

fn process_file(file_system: &impl FileSystem, path: &Path, report: &mut ScanReport) {
    let normalized_path = match normalize_path(path) {
        Ok(normalized_path) => normalized_path,
        Err(error) => {
            report.errors.push(CrawlError::from_normalize_error(
                path,
                FsOperation::NormalizePath,
                error,
            ));
            return;
        }
    };

    if ignored_path_name(&normalized_path) {
        report.ignored_count += 1;
        return;
    }

    let Some(extension) = supported_extension(&normalized_path) else {
        report.ignored_count += 1;
        return;
    };

    let metadata = match file_system.metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            report
                .errors
                .push(CrawlError::from_io(path, FsOperation::ReadMetadata, error));
            return;
        }
    };

    if metadata.kind != FsEntryKind::File {
        report.ignored_count += 1;
        return;
    }

    let fingerprint = match quick_fingerprint(file_system, path, &normalized_path, &metadata) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            report.errors.push(error);
            return;
        }
    };

    let mtime = unix_timestamp(metadata.modified);
    let byte_size = metadata.len;
    let document_id = DocumentId::from_non_secret_parts(&[
        fingerprint.value.as_str(),
        byte_size.to_string().as_str(),
        mtime.as_unix_seconds().to_string().as_str(),
    ]);

    report.files.push(DiscoveredFile {
        document_id,
        normalized_path,
        file_name: path_file_name(path),
        extension,
        byte_size,
        mtime,
        permissions: FilePermissions {
            readonly: metadata.readonly,
        },
        fingerprint,
    });
}

fn quick_fingerprint(
    file_system: &impl FileSystem,
    path: &Path,
    normalized_path: &NormalizedPath,
    metadata: &FsMetadata,
) -> Result<QuickFingerprint> {
    let mut hash = StableHash::new();
    hash.update_str("fs-crawler-v1");
    hash.update_str(normalized_path.as_str());
    hash.update_u64(metadata.len);

    let (mtime_seconds, mtime_nanos) = system_time_parts(metadata.modified);
    hash.update_i64(mtime_seconds);
    hash.update_u32(mtime_nanos);

    let mut reader = file_system
        .open(path)
        .map_err(|error| CrawlError::from_io(path, FsOperation::Fingerprint, error))?;

    let mut sampled_bytes = 0_u64;
    if metadata.len <= MAX_TOTAL_SAMPLE_BYTES {
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|error| CrawlError::from_io(path, FsOperation::Fingerprint, error))?;
        let sample = read_up_to(&mut *reader, metadata.len as usize)
            .map_err(|error| CrawlError::from_io(path, FsOperation::Fingerprint, error))?;
        sampled_bytes += sample.len() as u64;
        hash.update_str("all");
        hash.update_bytes(&sample);
    } else {
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|error| CrawlError::from_io(path, FsOperation::Fingerprint, error))?;
        let head = read_up_to(&mut *reader, SAMPLE_BYTES_PER_EDGE as usize)
            .map_err(|error| CrawlError::from_io(path, FsOperation::Fingerprint, error))?;
        sampled_bytes += head.len() as u64;
        hash.update_str("head");
        hash.update_bytes(&head);

        reader
            .seek(SeekFrom::Start(metadata.len - SAMPLE_BYTES_PER_EDGE))
            .map_err(|error| CrawlError::from_io(path, FsOperation::Fingerprint, error))?;
        let tail = read_up_to(&mut *reader, SAMPLE_BYTES_PER_EDGE as usize)
            .map_err(|error| CrawlError::from_io(path, FsOperation::Fingerprint, error))?;
        sampled_bytes += tail.len() as u64;
        hash.update_str("tail");
        hash.update_bytes(&tail);
    }

    Ok(QuickFingerprint {
        value: format!("qfp_{:016x}{:016x}", hash.first, hash.second),
        sampled_bytes,
    })
}

fn read_up_to(reader: &mut dyn Read, max_bytes: usize) -> io::Result<Vec<u8>> {
    let mut output = vec![0; max_bytes];
    let mut total_read = 0;

    while total_read < max_bytes {
        let read = reader.read(&mut output[total_read..])?;
        if read == 0 {
            break;
        }
        total_read += read;
    }

    output.truncate(total_read);
    Ok(output)
}

fn ignored_directory(path: &Path, profile: ScanProfile, report: &mut ScanReport) -> bool {
    match normalize_path(path) {
        Ok(normalized_path) => {
            let ignored = ignored_path_name(&normalized_path)
                || profile_ignored_directory(&normalized_path, profile);
            if ignored {
                report.skipped_directories.push(normalized_path);
            }
            ignored
        }
        Err(error) => {
            report.errors.push(CrawlError::from_normalize_error(
                path,
                FsOperation::NormalizePath,
                error,
            ));
            true
        }
    }
}

fn profile_ignored_directory(path: &NormalizedPath, profile: ScanProfile) -> bool {
    profile == ScanProfile::Discovery
        && (discovery_system_directory(path) || discovery_dependency_directory(path))
}

fn discovery_system_directory(path: &NormalizedPath) -> bool {
    let lower_path = path.as_str().to_ascii_lowercase();
    if matches!(
        lower_path.as_str(),
        "/applications"
            | "/bin"
            | "/boot"
            | "/cores"
            | "/dev"
            | "/etc"
            | "/library"
            | "/network"
            | "/opt"
            | "/private"
            | "/proc"
            | "/run"
            | "/sbin"
            | "/system"
            | "/tmp"
            | "/usr"
            | "/var"
            | "/volumes"
    ) {
        return true;
    }

    let parts = lower_path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.len() == 3 && parts[0] == "users" && parts[2] == "library" {
        return true;
    }

    parts.len() == 2
        && parts[0].ends_with(':')
        && matches!(
            parts[1],
            "$recycle.bin"
                | "program files"
                | "program files (x86)"
                | "system volume information"
                | "windows"
        )
}

fn discovery_dependency_directory(path: &NormalizedPath) -> bool {
    let Some(file_name) = path.file_name() else {
        return true;
    };

    matches!(
        file_name,
        "__pycache__"
            | "build"
            | "cache"
            | "caches"
            | "dist"
            | "env"
            | "node_modules"
            | "target"
            | "temp"
            | "tmp"
            | "vendor"
            | "venv"
    )
}

fn ignored_path_name(path: &NormalizedPath) -> bool {
    let Some(file_name) = path.file_name() else {
        return true;
    };

    if file_name == ".DS_Store"
        || file_name.eq_ignore_ascii_case("Thumbs.db")
        || file_name.starts_with("~$")
        || file_name.starts_with('.')
        || file_name.ends_with('~')
    {
        return true;
    }

    matches!(
        path.extension().as_deref(),
        Some("tmp" | "temp" | "swp" | "swo" | "part" | "crdownload")
    )
}

fn supported_extension(path: &NormalizedPath) -> Option<FileExtension> {
    match path.extension()?.as_str() {
        "docx" => Some(FileExtension::Docx),
        "pdf" => Some(FileExtension::Pdf),
        "doc" => Some(FileExtension::Doc),
        "txt" => Some(FileExtension::Txt),
        _ => None,
    }
}

fn normalize_path_string(raw: &str) -> String {
    let replaced = raw.replace('\\', "/");
    let (drive_prefix, drive_absolute, without_drive) = split_windows_drive(&replaced);
    let unc_prefix = drive_prefix.is_none() && without_drive.starts_with("//");
    let absolute = drive_prefix.is_none() && without_drive.starts_with('/') && !unc_prefix;
    let anchored = drive_absolute || absolute || unc_prefix;
    let minimum_parts = if unc_prefix { 2 } else { 0 };
    let mut parts = Vec::<&str>::new();

    for part in without_drive.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.len() > minimum_parts && parts.last().is_some_and(|last| *last != "..") {
                    parts.pop();
                } else if !anchored {
                    parts.push(part);
                }
            }
            _ => parts.push(part),
        }
    }

    match (
        drive_prefix,
        drive_absolute,
        unc_prefix,
        absolute,
        parts.is_empty(),
    ) {
        (Some(prefix), true, _, _, true) => format!("{prefix}:/"),
        (Some(prefix), true, _, _, false) => format!("{prefix}:/{}", parts.join("/")),
        (Some(prefix), false, _, _, true) => format!("{prefix}:"),
        (Some(prefix), false, _, _, false) => format!("{prefix}:{}", parts.join("/")),
        (None, _, true, _, true) => "//".to_string(),
        (None, _, true, _, false) => format!("//{}", parts.join("/")),
        (None, _, false, true, true) => "/".to_string(),
        (None, _, false, true, false) => format!("/{}", parts.join("/")),
        (None, _, false, false, true) => ".".to_string(),
        (None, _, false, false, false) => parts.join("/"),
    }
}

fn split_windows_drive(path: &str) -> (Option<char>, bool, &str) {
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = &path[2..];
        return (Some(drive), rest.starts_with('/'), rest);
    }

    (None, false, path)
}

fn path_file_name(path: &Path) -> String {
    normalize_path(path)
        .ok()
        .and_then(|normalized| normalized.file_name().map(str::to_string))
        .unwrap_or_else(|| "<unknown>".to_string())
}

fn path_sort_key(path: &Path) -> String {
    normalize_path(path)
        .map(|normalized| normalized.into_inner())
        .unwrap_or_else(|_| path.as_os_str().to_string_lossy().replace('\\', "/"))
}

fn unix_timestamp(time: SystemTime) -> UnixTimestamp {
    let (seconds, _) = system_time_parts(time);
    UnixTimestamp::from_unix_seconds(seconds)
}

fn system_time_parts(time: SystemTime) -> (i64, u32) {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let seconds = i64::try_from(duration.as_secs()).unwrap_or(i64::MAX);
            (seconds, duration.subsec_nanos())
        }
        Err(error) => {
            let duration = error.duration();
            let seconds = i64::try_from(duration.as_secs()).unwrap_or(i64::MAX);
            (-seconds, duration.subsec_nanos())
        }
    }
}

fn classify_io_error(kind: io::ErrorKind, operation: FsOperation) -> CrawlErrorKind {
    match kind {
        io::ErrorKind::PermissionDenied => CrawlErrorKind::PermissionDenied,
        io::ErrorKind::NotFound => CrawlErrorKind::SourceUnavailable,
        io::ErrorKind::WouldBlock
        | io::ErrorKind::Interrupted
        | io::ErrorKind::TimedOut
        | io::ErrorKind::UnexpectedEof
        | io::ErrorKind::Other
            if operation == FsOperation::Fingerprint =>
        {
            CrawlErrorKind::LockedOrUnreadable
        }
        _ => CrawlErrorKind::Io,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanProfile {
    #[default]
    Explicit,
    Discovery,
}

impl ScanProfile {
    pub fn label(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Discovery => "discovery",
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct ScanReport {
    pub files: Vec<DiscoveredFile>,
    pub errors: Vec<CrawlError>,
    pub ignored_count: usize,
    pub scanned_directories: Vec<NormalizedPath>,
    pub skipped_directories: Vec<NormalizedPath>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DiscoveredFile {
    pub document_id: DocumentId,
    pub normalized_path: NormalizedPath,
    pub file_name: String,
    pub extension: FileExtension,
    pub byte_size: u64,
    pub mtime: UnixTimestamp,
    pub permissions: FilePermissions,
    pub fingerprint: QuickFingerprint,
}

impl fmt::Debug for DiscoveredFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DiscoveredFile")
            .field("document_id", &self.document_id)
            .field("normalized_path", &"<redacted>")
            .field("file_name", &"<redacted>")
            .field("extension", &self.extension)
            .field("byte_size", &self.byte_size)
            .field("mtime", &self.mtime)
            .field("permissions", &self.permissions)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilePermissions {
    pub readonly: bool,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NormalizedPath(String);

impl NormalizedPath {
    fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn file_name(&self) -> Option<&str> {
        self.0.rsplit('/').next().filter(|value| !value.is_empty())
    }

    pub fn extension(&self) -> Option<String> {
        let file_name = self.file_name()?;
        let (_, extension) = file_name.rsplit_once('.')?;
        if extension.is_empty() {
            return None;
        }

        Some(extension.to_ascii_lowercase())
    }

    fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Debug for NormalizedPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted-path>")
    }
}

impl fmt::Display for NormalizedPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted-path>")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct QuickFingerprint {
    value: String,
    pub sampled_bytes: u64,
}

impl QuickFingerprint {
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for QuickFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted-fingerprint>")
    }
}

impl fmt::Debug for QuickFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuickFingerprint")
            .field("value", &"<redacted>")
            .field("sampled_bytes", &self.sampled_bytes)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizePathError;

impl fmt::Display for NormalizePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("path could not be normalized")
    }
}

impl std::error::Error for NormalizePathError {}

pub type Result<T> = std::result::Result<T, CrawlError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CrawlErrorKind {
    PermissionDenied,
    SourceUnavailable,
    LockedOrUnreadable,
    Io,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FsOperation {
    NormalizePath,
    ReadDirectory,
    ReadMetadata,
    Fingerprint,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CrawlError {
    pub kind: CrawlErrorKind,
    pub operation: FsOperation,
    path: Option<NormalizedPath>,
}

impl CrawlError {
    fn new(kind: CrawlErrorKind, operation: FsOperation, path: &Path) -> Self {
        Self {
            kind,
            operation,
            path: normalize_path(path).ok(),
        }
    }

    fn from_io(path: &Path, operation: FsOperation, error: io::Error) -> Self {
        Self::new(classify_io_error(error.kind(), operation), operation, path)
    }

    fn from_normalize_error(
        path: &Path,
        operation: FsOperation,
        _error: NormalizePathError,
    ) -> Self {
        Self::new(CrawlErrorKind::Io, operation, path)
    }

    pub fn normalized_path(&self) -> Option<&NormalizedPath> {
        self.path.as_ref()
    }

    fn sort_key(&self) -> (CrawlErrorKind, FsOperation, Option<String>) {
        (
            self.kind,
            self.operation,
            self.path.as_ref().map(|path| path.as_str().to_string()),
        )
    }
}

impl fmt::Debug for CrawlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CrawlError")
            .field("kind", &self.kind)
            .field("operation", &self.operation)
            .field("path", &self.path.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl fmt::Display for CrawlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "file scan failed [kind={:?}, operation={:?}, path=<redacted>]",
            self.kind, self.operation
        )
    }
}

impl std::error::Error for CrawlError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsEntryKind {
    File,
    Directory,
    Other,
}

#[derive(Clone, PartialEq, Eq)]
pub struct FsEntry {
    pub path: PathBuf,
    pub kind: FsEntryKind,
}

impl FsEntry {
    pub fn new(path: PathBuf, kind: FsEntryKind) -> Self {
        Self { path, kind }
    }
}

impl fmt::Debug for FsEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FsEntry")
            .field("path", &"<redacted>")
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsMetadata {
    pub kind: FsEntryKind,
    pub len: u64,
    pub modified: SystemTime,
    pub readonly: bool,
}

impl FsMetadata {
    pub fn new(kind: FsEntryKind, len: u64, modified: SystemTime) -> Self {
        Self {
            kind,
            len,
            modified,
            readonly: false,
        }
    }

    pub fn with_readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }
}

pub trait ReadSeek: Read + Seek {}

impl<T> ReadSeek for T where T: Read + Seek {}

pub trait FileSystem {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<FsEntry>>;
    fn metadata(&self, path: &Path) -> io::Result<FsMetadata>;
    fn open(&self, path: &Path) -> io::Result<Box<dyn ReadSeek>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StdFileSystem;

impl FileSystem for StdFileSystem {
    fn read_dir(&self, path: &Path) -> io::Result<Vec<FsEntry>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let kind = entry_kind(entry.file_type()?);
            entries.push(FsEntry::new(entry.path(), kind));
        }

        Ok(entries)
    }

    fn metadata(&self, path: &Path) -> io::Result<FsMetadata> {
        let metadata = fs::metadata(path)?;
        Ok(FsMetadata::new(
            entry_kind(metadata.file_type()),
            metadata.len(),
            metadata.modified()?,
        )
        .with_readonly(metadata.permissions().readonly()))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn ReadSeek>> {
        Ok(Box::new(fs::File::open(path)?))
    }
}

fn entry_kind(file_type: fs::FileType) -> FsEntryKind {
    if file_type.is_file() {
        FsEntryKind::File
    } else if file_type.is_dir() {
        FsEntryKind::Directory
    } else {
        FsEntryKind::Other
    }
}

struct StableHash {
    first: u64,
    second: u64,
}

impl StableHash {
    fn new() -> Self {
        Self {
            first: FNV_OFFSET_A,
            second: FNV_OFFSET_B,
        }
    }

    fn update_str(&mut self, value: &str) {
        self.update_bytes(value.as_bytes());
    }

    fn update_i64(&mut self, value: i64) {
        self.update_bytes(&value.to_le_bytes());
    }

    fn update_u32(&mut self, value: u32) {
        self.update_bytes(&value.to_le_bytes());
    }

    fn update_u64(&mut self, value: u64) {
        self.update_bytes(&value.to_le_bytes());
    }

    fn update_bytes(&mut self, bytes: &[u8]) {
        self.update_u64_raw(bytes.len() as u64);
        for byte in bytes {
            self.first ^= u64::from(*byte);
            self.first = self.first.wrapping_mul(FNV_PRIME);
            self.second ^= u64::from(*byte);
            self.second = self.second.wrapping_mul(FNV_PRIME);
        }
    }

    fn update_u64_raw(&mut self, value: u64) {
        for byte in value.to_le_bytes() {
            self.first ^= u64::from(byte);
            self.first = self.first.wrapping_mul(FNV_PRIME);
            self.second ^= u64::from(byte);
            self.second = self.second.wrapping_mul(FNV_PRIME);
        }
    }
}
