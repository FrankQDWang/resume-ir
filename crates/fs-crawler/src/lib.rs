use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const SAMPLE_BYTES: usize = 2048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub normalized_path: String,
    pub file_name: String,
    pub extension: String,
    pub fingerprint: FileFingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileFingerprint {
    pub normalized_path: String,
    pub byte_size: u64,
    pub mtime_unix_ms: i64,
    pub sample_hash: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanErrorKind {
    Unreachable,
    NotDirectory,
    PermissionDenied,
    Locked,
    Io,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanError {
    pub kind: ScanErrorKind,
    pub retryable: bool,
    pub path: PathBuf,
    pub message: String,
}

pub type ScanResult<T> = Result<T, ScanError>;

pub fn scan_directory(root: &Path) -> ScanResult<Vec<FileEntry>> {
    if !root.exists() {
        return Err(ScanError::new(
            ScanErrorKind::Unreachable,
            true,
            root,
            "root is not reachable",
        ));
    }
    if !root.is_dir() {
        return Err(ScanError::new(
            ScanErrorKind::NotDirectory,
            false,
            root,
            "root is not a directory",
        ));
    }

    let mut entries = Vec::new();
    visit_directory(root, &mut entries)?;
    entries.sort_by(|left, right| left.normalized_path.cmp(&right.normalized_path));
    Ok(entries)
}

pub fn normalize_path(path: &Path) -> String {
    normalize_path_str(&path.to_string_lossy())
}

pub fn normalize_path_str(path: &str) -> String {
    path.replace('\\', "/")
}

pub fn should_skip_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };

    file_name == ".DS_Store" || file_name.starts_with("~$") || file_name.ends_with(".tmp")
}

pub fn supported_extension(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "docx" | "pdf" | "doc" | "txt" | "png" | "jpg" | "jpeg"
    )
}

fn visit_directory(directory: &Path, entries: &mut Vec<FileEntry>) -> ScanResult<()> {
    let children = fs::read_dir(directory).map_err(|error| map_io_error(directory, error))?;

    for child in children {
        let child = child.map_err(|error| map_io_error(directory, error))?;
        let path = child.path();
        let file_type = child
            .file_type()
            .map_err(|error| map_io_error(&path, error))?;

        if file_type.is_dir() {
            visit_directory(&path, entries)?;
        } else if file_type.is_file() && !should_skip_file(&path) && supported_extension(&path) {
            entries.push(build_entry(&path)?);
        }
    }

    Ok(())
}

fn build_entry(path: &Path) -> ScanResult<FileEntry> {
    let metadata = fs::metadata(path).map_err(|error| map_io_error(path, error))?;
    let normalized_path = normalize_path(path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    Ok(FileEntry {
        path: path.to_path_buf(),
        fingerprint: FileFingerprint {
            normalized_path: normalized_path.clone(),
            byte_size: metadata.len(),
            mtime_unix_ms: modified_unix_ms(&metadata),
            sample_hash: sample_hash(path, metadata.len())?,
        },
        normalized_path,
        file_name,
        extension,
    })
}

fn modified_unix_ms(metadata: &fs::Metadata) -> i64 {
    let Ok(modified) = metadata.modified() else {
        return 0;
    };
    let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) else {
        return 0;
    };
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn sample_hash(path: &Path, byte_size: u64) -> ScanResult<u64> {
    let mut file = fs::File::open(path).map_err(|error| map_io_error(path, error))?;
    let mut sample = Vec::new();

    if byte_size <= (SAMPLE_BYTES * 2) as u64 {
        file.read_to_end(&mut sample)
            .map_err(|error| map_io_error(path, error))?;
    } else {
        let mut head = vec![0; SAMPLE_BYTES];
        let head_len = file
            .read(&mut head)
            .map_err(|error| map_io_error(path, error))?;
        sample.extend_from_slice(&head[..head_len]);

        file.seek(SeekFrom::End(-(SAMPLE_BYTES as i64)))
            .map_err(|error| map_io_error(path, error))?;
        let mut tail = vec![0; SAMPLE_BYTES];
        let tail_len = file
            .read(&mut tail)
            .map_err(|error| map_io_error(path, error))?;
        sample.extend_from_slice(&tail[..tail_len]);
    }

    Ok(fnv1a64(&sample))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn map_io_error(path: &Path, error: std::io::Error) -> ScanError {
    let kind = match error.kind() {
        std::io::ErrorKind::PermissionDenied => ScanErrorKind::PermissionDenied,
        std::io::ErrorKind::NotFound => ScanErrorKind::Unreachable,
        _ => ScanErrorKind::Io,
    };
    let retryable = matches!(
        kind,
        ScanErrorKind::Unreachable | ScanErrorKind::PermissionDenied | ScanErrorKind::Locked
    );
    ScanError::new(kind, retryable, path, error.to_string())
}

impl ScanError {
    #[must_use]
    pub fn new(
        kind: ScanErrorKind,
        retryable: bool,
        path: &Path,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            retryable,
            path: path.to_path_buf(),
            message: message.into(),
        }
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "fs-crawler"
}
