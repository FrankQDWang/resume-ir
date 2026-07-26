use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::time::{Duration, Instant};

use base64::Engine;
use meta_store::{ReadMetaStore, SearchSelection, SearchSourceFileResolution};
use sha2::Digest;

use crate::source_file_authority::{SourceFileError, VerifiedSourceFile};

const LEASE_TTL: Duration = Duration::from_secs(120);
const MAX_LEASES: usize = 8;
pub(crate) const MAX_RANGE_BYTES: usize = 64 * 1024;
const CHUNK_BYTES: u64 = MAX_RANGE_BYTES as u64;

struct PreviewLease {
    file: File,
    selection: SearchSelection,
    byte_size: u64,
    content_hash: String,
    chunk_hashes: Vec<[u8; 32]>,
    expires_at: Instant,
}

pub(crate) struct PreviewLeaseStore {
    leases: HashMap<String, PreviewLease>,
}

pub(crate) struct CreatedPreview {
    pub(crate) lease_id: String,
    pub(crate) byte_size: u64,
    pub(crate) expires_in_ms: u64,
}

pub(crate) struct PreviewRange {
    pub(crate) offset: u64,
    pub(crate) bytes_read: usize,
    pub(crate) total_bytes: u64,
    pub(crate) base64_data: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PreviewError {
    Source(SourceFileError),
    LeaseInvalid,
    RangeInvalid,
    Capacity,
    Io,
}

impl PreviewLeaseStore {
    pub(crate) fn new() -> Self {
        Self {
            leases: HashMap::new(),
        }
    }

    pub(crate) fn ensure_capacity(&mut self) -> Result<(), PreviewError> {
        self.remove_expired();
        if self.leases.len() >= MAX_LEASES {
            return Err(PreviewError::Capacity);
        }
        Ok(())
    }

    pub(crate) fn create_verified(
        &mut self,
        selection: SearchSelection,
        verified: VerifiedSourceFile,
    ) -> Result<CreatedPreview, PreviewError> {
        self.ensure_capacity()?;
        if !verified.is_pdf() {
            return Err(PreviewError::Source(SourceFileError::UnsupportedFormat));
        }
        let byte_size = verified.byte_size();
        let content_hash = verified.content_hash().to_string();
        let (file, _, chunk_hashes) = verified.into_parts();
        let lease_id = random_lease_id().map_err(|_| PreviewError::Io)?;
        self.leases.insert(
            lease_id.clone(),
            PreviewLease {
                file,
                selection,
                byte_size,
                content_hash,
                chunk_hashes,
                expires_at: Instant::now() + LEASE_TTL,
            },
        );
        Ok(CreatedPreview {
            lease_id,
            byte_size,
            expires_in_ms: LEASE_TTL.as_millis() as u64,
        })
    }

    pub(crate) fn read_range(
        &mut self,
        store: &ReadMetaStore,
        lease_id: &str,
        offset: u64,
        length: usize,
    ) -> Result<PreviewRange, PreviewError> {
        self.remove_expired();
        if length == 0 || length > MAX_RANGE_BYTES {
            return Err(PreviewError::RangeInvalid);
        }
        let lease = self
            .leases
            .get_mut(lease_id)
            .ok_or(PreviewError::LeaseInvalid)?;
        match store
            .search_source_file(&lease.selection)
            .map_err(|_| PreviewError::Source(SourceFileError::MetadataUnavailable))?
        {
            SearchSourceFileResolution::Current(reference)
                if reference.byte_size == lease.byte_size
                    && reference.content_hash.as_str() == lease.content_hash => {}
            SearchSourceFileResolution::Current(_) => {
                return Err(PreviewError::Source(SourceFileError::SourceChanged));
            }
            SearchSourceFileResolution::Stale => {
                return Err(PreviewError::Source(SourceFileError::StaleSelection));
            }
            SearchSourceFileResolution::NotFound => {
                return Err(PreviewError::Source(SourceFileError::SourceMissing));
            }
        }
        if offset >= lease.byte_size {
            return Err(PreviewError::RangeInvalid);
        }
        let remaining = lease.byte_size - offset;
        let bounded = usize::try_from(remaining.min(length as u64))
            .map_err(|_| PreviewError::RangeInvalid)?;
        let bytes = read_verified_range(lease, offset, bounded)?;
        // The lease is short-lived when idle, but an actively viewed document
        // must not expire halfway through a deliberate page turn.
        lease.expires_at = Instant::now() + LEASE_TTL;
        Ok(PreviewRange {
            offset,
            bytes_read: bytes.len(),
            total_bytes: lease.byte_size,
            base64_data: base64::engine::general_purpose::STANDARD.encode(bytes),
        })
    }

    pub(crate) fn close(&mut self, lease_id: &str) -> bool {
        self.remove_expired();
        self.leases.remove(lease_id).is_some()
    }

    fn remove_expired(&mut self) {
        let now = Instant::now();
        self.leases.retain(|_, lease| lease.expires_at > now);
    }
}

fn read_verified_range(
    lease: &mut PreviewLease,
    offset: u64,
    length: usize,
) -> Result<Vec<u8>, PreviewError> {
    let end = offset
        .checked_add(u64::try_from(length).map_err(|_| PreviewError::RangeInvalid)?)
        .ok_or(PreviewError::RangeInvalid)?;
    let first_chunk = offset / CHUNK_BYTES;
    let last_chunk = (end - 1) / CHUNK_BYTES;
    let mut requested = Vec::with_capacity(length);
    for chunk_index in first_chunk..=last_chunk {
        let chunk_start = chunk_index * CHUNK_BYTES;
        let chunk_end = lease.byte_size.min(chunk_start + CHUNK_BYTES);
        let chunk_length =
            usize::try_from(chunk_end - chunk_start).map_err(|_| PreviewError::RangeInvalid)?;
        lease
            .file
            .seek(SeekFrom::Start(chunk_start))
            .map_err(|_| PreviewError::Io)?;
        let mut chunk = vec![0_u8; chunk_length];
        lease
            .file
            .read_exact(&mut chunk)
            .map_err(|_| PreviewError::Source(SourceFileError::SourceChanged))?;
        let expected = lease
            .chunk_hashes
            .get(usize::try_from(chunk_index).map_err(|_| PreviewError::RangeInvalid)?)
            .ok_or(PreviewError::Source(SourceFileError::SourceChanged))?;
        let observed: [u8; 32] = sha2::Sha256::digest(&chunk).into();
        if &observed != expected {
            return Err(PreviewError::Source(SourceFileError::SourceChanged));
        }
        let copy_start = usize::try_from(offset.max(chunk_start) - chunk_start)
            .map_err(|_| PreviewError::RangeInvalid)?;
        let copy_end = usize::try_from(end.min(chunk_end) - chunk_start)
            .map_err(|_| PreviewError::RangeInvalid)?;
        requested.extend_from_slice(&chunk[copy_start..copy_end]);
    }
    if requested.len() != length {
        return Err(PreviewError::Source(SourceFileError::SourceChanged));
    }
    Ok(requested)
}

fn random_lease_id() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
