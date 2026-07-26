use std::fmt;
use std::fs::{self, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use meta_store::{
    DocumentId, FileExtension, OwnedMetaStore, ReadMetaStore, SearchMetadataReadError,
    SearchSelection, SearchSourceFileReference, SearchSourceFileResolution, SourceRevisionId,
};
use sha2::{Digest, Sha256};

const MAX_SOURCE_FILE_BYTES: u64 = 256 * 1024 * 1024;
const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceFileError {
    StaleSelection,
    NotFound,
    SourceMissing,
    SourceChanged,
    UnsafePath,
    UnsupportedFormat,
    MetadataUnavailable,
    Cancelled,
    Io,
}

pub(crate) struct VerifiedSourceFile {
    file: File,
    path: PathBuf,
    byte_size: u64,
    content_hash: String,
    chunk_hashes: Vec<[u8; 32]>,
    extension: FileExtension,
}

impl fmt::Debug for VerifiedSourceFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VerifiedSourceFile")
            .field("path", &"<redacted>")
            .field("byte_size", &self.byte_size)
            .field("extension", &self.extension)
            .finish()
    }
}

impl VerifiedSourceFile {
    pub(crate) fn byte_size(&self) -> u64 {
        self.byte_size
    }

    pub(crate) fn is_pdf(&self) -> bool {
        self.extension == FileExtension::Pdf
    }

    pub(crate) fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub(crate) fn into_parts(self) -> (File, PathBuf, Vec<[u8; 32]>) {
        (self.file, self.path, self.chunk_hashes)
    }
}

pub(crate) fn open_verified_with_cancellation(
    store: &ReadMetaStore,
    selection: &SearchSelection,
    cancellation: &AtomicBool,
) -> Result<VerifiedSourceFile, SourceFileError> {
    open_verified_with(store, selection, || cancellation.load(Ordering::Acquire))
}

fn open_verified_with(
    store: &ReadMetaStore,
    selection: &SearchSelection,
    is_cancelled: impl Fn() -> bool,
) -> Result<VerifiedSourceFile, SourceFileError> {
    let reference = match store
        .search_source_file(selection)
        .map_err(map_store_error)?
    {
        SearchSourceFileResolution::Current(reference) => reference,
        SearchSourceFileResolution::Stale => return Err(SourceFileError::StaleSelection),
        SearchSourceFileResolution::NotFound => return Err(SourceFileError::NotFound),
    };
    open_reference(reference, is_cancelled)
}

pub(crate) fn open_verified_revision_with_cancellation(
    store: &OwnedMetaStore,
    document_id: &DocumentId,
    source_revision_id: &SourceRevisionId,
    is_cancelled: impl Fn() -> bool,
) -> Result<VerifiedSourceFile, SourceFileError> {
    let reference = store
        .active_source_file_for_revision(document_id, source_revision_id)
        .map_err(|_| SourceFileError::MetadataUnavailable)?
        .ok_or(SourceFileError::NotFound)?;
    open_reference(reference, is_cancelled)
}

fn open_reference(
    reference: SearchSourceFileReference,
    is_cancelled: impl Fn() -> bool,
) -> Result<VerifiedSourceFile, SourceFileError> {
    if reference.byte_size == 0
        || reference.byte_size > MAX_SOURCE_FILE_BYTES
        || !safe_relative_path(&reference.relative_path)
    {
        return Err(SourceFileError::UnsafePath);
    }
    let root_path = Path::new(&reference.root_path);
    let Some(root) = crate::source_root_path::canonicalize_authorized_root(root_path) else {
        return Err(SourceFileError::UnsafePath);
    };
    if root != root_path {
        return Err(SourceFileError::UnsafePath);
    }
    let path = validate_source_path(&root, &reference.relative_path)?;
    let mut file = File::open(&path).map_err(classify_io)?;
    let opened_metadata = file.metadata().map_err(classify_io)?;
    if !opened_metadata.is_file() || opened_metadata.len() != reference.byte_size {
        return Err(SourceFileError::SourceChanged);
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; HASH_BUFFER_BYTES];
    let mut observed = 0_u64;
    let mut chunk_hashes = Vec::new();
    loop {
        if is_cancelled() {
            return Err(SourceFileError::Cancelled);
        }
        let read = file.read(&mut buffer).map_err(classify_io)?;
        if read == 0 {
            break;
        }
        observed = observed
            .checked_add(u64::try_from(read).map_err(|_| SourceFileError::Io)?)
            .ok_or(SourceFileError::Io)?;
        if observed > reference.byte_size {
            return Err(SourceFileError::SourceChanged);
        }
        hasher.update(&buffer[..read]);
        chunk_hashes.push(Sha256::digest(&buffer[..read]).into());
    }
    let digest = format!("sha256:{:x}", hasher.finalize());
    if observed != reference.byte_size || digest != reference.content_hash.as_str() {
        return Err(SourceFileError::SourceChanged);
    }
    file.seek(SeekFrom::Start(0)).map_err(classify_io)?;
    Ok(VerifiedSourceFile {
        file,
        path,
        byte_size: reference.byte_size,
        content_hash: reference.content_hash.as_str().to_string(),
        chunk_hashes,
        extension: reference.extension,
    })
}

fn validate_source_path(root: &Path, relative_path: &Path) -> Result<PathBuf, SourceFileError> {
    let mut path = root.to_path_buf();
    let mut components = relative_path.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(value) = component else {
            return Err(SourceFileError::UnsafePath);
        };
        path.push(value);
        let metadata = fs::symlink_metadata(&path).map_err(classify_io)?;
        if is_link_or_reparse_point(&metadata) {
            return Err(SourceFileError::UnsafePath);
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return Err(SourceFileError::UnsafePath);
            }
        } else if !metadata.is_file() {
            return Err(SourceFileError::UnsafePath);
        }
    }
    Ok(path)
}

#[cfg(not(windows))]
fn is_link_or_reparse_point(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
fn is_link_or_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|component| match component {
            Component::Normal(value) => !value.is_empty(),
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => false,
        })
}

fn map_store_error(_: SearchMetadataReadError) -> SourceFileError {
    SourceFileError::MetadataUnavailable
}

fn classify_io(error: std::io::Error) -> SourceFileError {
    match error.kind() {
        std::io::ErrorKind::NotFound => SourceFileError::SourceMissing,
        std::io::ErrorKind::PermissionDenied => SourceFileError::UnsafePath,
        _ => SourceFileError::Io,
    }
}
