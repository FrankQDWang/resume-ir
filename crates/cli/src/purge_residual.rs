//! Bounded fail-closed residual scan for physical deleted-data purges.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::Path;

use import_pipeline::{DataDirectoryOwnerLease, PurgeArtifactClass, PurgeArtifactClassifier};
use meta_store::{DocumentId, OwnedMetaStore};

use crate::{CliError, Result};

const MARKER_MIN_BYTES: usize = 8;
const SCAN_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub(crate) struct PurgeResidualProbe {
    markers: BTreeSet<Vec<u8>>,
}

impl PurgeResidualProbe {
    pub(crate) fn collect(
        store: &OwnedMetaStore,
        document_ids: &[DocumentId],
        ocr_cache_hashes: &[String],
    ) -> Result<Self> {
        let mut probe = Self::default();

        for document_id in document_ids {
            probe.add_marker(document_id.as_str());
            let Some(document) = store.document_by_id(document_id).map_err(CliError::store)? else {
                continue;
            };
            probe.add_marker(&document.source_uri);
            probe.add_marker(&document.normalized_path);
            probe.add_marker(&document.file_name);

            for version in store
                .resume_versions_for_document(&document.id)
                .map_err(CliError::store)?
            {
                probe.add_marker(version.id.as_str());
                if let Some(raw_text) = &version.raw_text {
                    probe.add_marker(raw_text);
                }
                if let Some(clean_text) = &version.clean_text {
                    probe.add_marker(clean_text);
                }
                for mention in store
                    .entity_mentions_for_version(&version.id)
                    .map_err(CliError::store)?
                {
                    probe.add_marker(&mention.raw_value);
                    if let Some(normalized_value) = &mention.normalized_value {
                        probe.add_marker(normalized_value);
                    }
                }
            }
        }

        for marker in store
            .import_task_markers_for_deleted_documents(document_ids)
            .map_err(CliError::store)?
        {
            probe.add_marker(&marker);
        }
        for entry in store
            .ocr_page_cache_entries_for_content_hashes(ocr_cache_hashes)
            .map_err(CliError::store)?
        {
            if let Some(text) = entry.text() {
                probe.add_marker(text);
            }
            for word_box in entry.word_boxes() {
                probe.add_marker(word_box.text());
            }
        }

        Ok(probe)
    }

    fn add_marker(&mut self, value: &str) {
        let marker = value.trim().as_bytes();
        if marker.len() >= MARKER_MIN_BYTES {
            self.markers.insert(marker.to_vec());
        }
    }

    pub(crate) fn scan_data_dir(
        &self,
        owner: &DataDirectoryOwnerLease,
    ) -> Result<PurgeResidualScan> {
        let markers_checked = self.markers.len();
        let Some(max_marker_len) = self.markers.iter().map(Vec::len).max() else {
            return Ok(PurgeResidualScan {
                markers_checked,
                ..PurgeResidualScan::default()
            });
        };

        let classifier = PurgeArtifactClassifier::new(owner);
        let mut scan = PurgeResidualScan {
            markers_checked,
            ..PurgeResidualScan::default()
        };
        let mut pending_dirs = vec![classifier.data_dir().to_path_buf()];
        while let Some(dir) = pending_dirs.pop() {
            let entries = fs::read_dir(&dir).map_err(|_| unreadable_artifact())?;
            for entry in entries {
                let entry = entry.map_err(|_| unreadable_artifact())?;
                let path = entry.path();
                let class = classifier
                    .classify(&path)
                    .map_err(|_| invalid_control_artifact())?;
                let file_type = entry.file_type().map_err(|_| unreadable_artifact())?;
                match class {
                    PurgeArtifactClass::ControlPlaneFile if file_type.is_file() => continue,
                    PurgeArtifactClass::ControlPlaneDirectory if file_type.is_dir() => {
                        pending_dirs.push(path);
                    }
                    PurgeArtifactClass::ControlPlaneFile
                    | PurgeArtifactClass::ControlPlaneDirectory => {
                        return Err(invalid_control_artifact());
                    }
                    PurgeArtifactClass::Data if file_type.is_dir() => pending_dirs.push(path),
                    PurgeArtifactClass::Data if file_type.is_file() => {
                        scan.files_scanned += 1;
                        let file_scan =
                            scan_file_for_markers(&path, &self.markers, max_marker_len)?;
                        scan.bytes_scanned = scan
                            .bytes_scanned
                            .checked_add(file_scan.bytes_scanned)
                            .ok_or_else(byte_count_overflow)?;
                        if file_scan.retained_marker {
                            scan.retained_markers += 1;
                        }
                    }
                    PurgeArtifactClass::Data => {
                        return Err(CliError::user(
                            "purge residual scan blocked by unsupported local artifact",
                        ));
                    }
                }
            }
        }

        Ok(scan)
    }
}

#[derive(Default)]
pub(crate) struct PurgeResidualScan {
    pub(crate) markers_checked: usize,
    pub(crate) files_scanned: usize,
    pub(crate) bytes_scanned: u64,
    pub(crate) retained_markers: usize,
}

#[derive(Default)]
struct PurgeResidualFileScan {
    bytes_scanned: u64,
    retained_marker: bool,
}

fn scan_file_for_markers(
    path: &Path,
    markers: &BTreeSet<Vec<u8>>,
    max_marker_len: usize,
) -> Result<PurgeResidualFileScan> {
    let mut file = fs::File::open(path).map_err(|_| unreadable_artifact())?;
    let mut scan = PurgeResidualFileScan::default();
    let mut buffer = vec![0_u8; SCAN_CHUNK_BYTES];
    let mut carry = Vec::new();
    let carry_len = max_marker_len.saturating_sub(1);

    loop {
        let bytes_read = file.read(&mut buffer).map_err(|_| unreadable_artifact())?;
        if bytes_read == 0 {
            return Ok(scan);
        }
        scan.bytes_scanned = scan
            .bytes_scanned
            .checked_add(u64::try_from(bytes_read).map_err(|_| byte_count_overflow())?)
            .ok_or_else(byte_count_overflow)?;
        let mut window = Vec::with_capacity(carry.len() + bytes_read);
        window.extend_from_slice(&carry);
        window.extend_from_slice(&buffer[..bytes_read]);
        if window_contains_marker(&window, markers) {
            scan.retained_marker = true;
            return Ok(scan);
        }
        carry.clear();
        if carry_len > 0 {
            let start = window.len().saturating_sub(carry_len);
            carry.extend_from_slice(&window[start..]);
        }
    }
}

fn window_contains_marker(buffer: &[u8], markers: &BTreeSet<Vec<u8>>) -> bool {
    markers.iter().any(|marker| {
        marker.len() <= buffer.len() && buffer.windows(marker.len()).any(|window| window == marker)
    })
}

fn unreadable_artifact() -> CliError {
    CliError::user("purge residual scan could not read local artifact")
}

fn invalid_control_artifact() -> CliError {
    CliError::user("purge residual scan found an invalid control artifact")
}

fn byte_count_overflow() -> CliError {
    CliError::user("purge residual scan byte count overflowed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use import_pipeline::{DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn held_owner_controls_are_excluded_while_marker_data_is_detected() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("resume-ir-purge-residual-{unique}"));
        let owner = match DataDirectoryOwnerLease::try_acquire(&root).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("synthetic owner contended"),
        };
        let marker = b"synthetic-retained-marker".to_vec();
        fs::write(owner.canonical_data_dir().join("marker.bin"), &marker).unwrap();
        let probe = PurgeResidualProbe {
            markers: BTreeSet::from([marker]),
        };

        let scan = probe.scan_data_dir(&owner).unwrap();
        assert_eq!(scan.retained_markers, 1);
        assert_eq!(scan.files_scanned, 1);
        drop(owner);
        fs::remove_dir_all(root).unwrap();
    }
}
