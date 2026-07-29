use std::fs;
use std::io;
use std::path::Path;

use super::{platform_stable_file_id, system_time_parts, StableFileId};

/// A high-resolution, path-independent observation used only to decide whether
/// an already imported file is eligible for the metadata fast path.
///
/// This is not a content digest. Callers must fail closed when the observation
/// is unavailable and must periodically refresh the strong content digest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileObservation {
    pub stable_file_id: StableFileId,
    pub byte_size: u64,
    pub modified: FileObservationTime,
    pub changed: FileObservationTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileObservationTime {
    pub seconds: i64,
    pub nanoseconds: u32,
}

impl FileObservationTime {
    fn from_system_time(value: std::time::SystemTime) -> Self {
        let (seconds, nanoseconds) = system_time_parts(value);
        Self {
            seconds,
            nanoseconds,
        }
    }
}

/// Observes an opened file handle. The handle keeps identity stable while its
/// metadata is inspected and is therefore preferred over a path-only stat.
pub fn observe_open_file(file: &fs::File) -> io::Result<Option<FileObservation>> {
    observation_from_metadata(Path::new(""), &file.metadata()?)
}

/// Observes the current target of a path without following any cached crawler
/// state. This is used for the final replacement/rename revalidation.
pub fn observe_path(path: &Path) -> io::Result<Option<FileObservation>> {
    let metadata = fs::metadata(path)?;
    observation_from_metadata(path, &metadata)
}

pub(crate) fn observation_from_metadata(
    path: &Path,
    metadata: &fs::Metadata,
) -> io::Result<Option<FileObservation>> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;

        let Some(stable_file_id) = platform_stable_file_id(path, metadata) else {
            return Ok(None);
        };
        let changed_seconds = metadata.ctime();
        let changed_nanoseconds = u32::try_from(metadata.ctime_nsec()).ok();
        let Some(changed_nanoseconds) = changed_nanoseconds.filter(|value| *value < 1_000_000_000)
        else {
            return Ok(None);
        };
        Ok(Some(FileObservation {
            stable_file_id,
            byte_size: metadata.len(),
            modified: FileObservationTime::from_system_time(metadata.modified()?),
            changed: FileObservationTime {
                seconds: changed_seconds,
                nanoseconds: changed_nanoseconds,
            },
        }))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, metadata);
        Ok(None)
    }
}
