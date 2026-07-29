use std::fs;
use std::io::{self, Read};
use std::path::Path;

use fs_crawler::{observe_open_file, observe_path, DiscoveredFile};

use crate::ImportIoMetrics;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContentVerification {
    Unavailable,
    Strong,
    MetadataFastPath,
}

pub(crate) fn read_full_content(
    file: &DiscoveredFile,
    io_metrics: &mut ImportIoMetrics,
) -> io::Result<Vec<u8>> {
    io_metrics.full_content_open_count = io_metrics.full_content_open_count.saturating_add(1);
    let path = Path::new(file.normalized_path.as_str());
    let mut handle = fs::File::open(path)?;
    if let Some(expected) = file.observation.as_ref() {
        if observe_open_file(&handle)?.as_ref() != Some(expected) {
            return Err(changed_during_read());
        }
    }

    let mut bytes = Vec::new();
    handle.read_to_end(&mut bytes)?;
    if let Some(expected) = file.observation.as_ref() {
        if observe_open_file(&handle)?.as_ref() != Some(expected)
            || observe_path(path)?.as_ref() != Some(expected)
        {
            return Err(changed_during_read());
        }
    }
    io_metrics.full_content_bytes = io_metrics
        .full_content_bytes
        .saturating_add(bytes.len() as u64);
    Ok(bytes)
}

fn changed_during_read() -> io::Error {
    io::Error::other("source changed during verified content read")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_after_discovery_fails_before_strong_content_is_accepted() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("synthetic.txt");
        fs::write(&path, b"first synthetic content").unwrap();
        let file = fs_crawler::crawl_directory(root.path())
            .unwrap()
            .files
            .remove(0);

        fs::remove_file(&path).unwrap();
        fs::write(&path, b"replacement content").unwrap();
        let mut metrics = ImportIoMetrics::default();
        let error = read_full_content(&file, &mut metrics).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(metrics.full_content_open_count, 1);
        assert_eq!(metrics.full_content_bytes, 0);
    }
}
