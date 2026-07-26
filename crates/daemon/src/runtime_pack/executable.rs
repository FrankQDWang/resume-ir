use std::path::Path;

use sha2::{Digest, Sha256};

use crate::ipc::OptionalRuntimeReason;

pub(super) struct PayloadIdentity {
    pub(super) architecture: &'static str,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}

pub(super) fn payload_identity(path: &Path) -> Result<PayloadIdentity, OptionalRuntimeReason> {
    let payload = canonical_payload(path).map_err(|_| OptionalRuntimeReason::Invalid)?;
    let mut digest = Sha256::new();
    digest.update(&payload.bytes);
    Ok(PayloadIdentity {
        architecture: payload.architecture,
        bytes: payload.bytes.len() as u64,
        sha256: format!("{:x}", digest.finalize()),
    })
}

#[cfg(target_os = "macos")]
fn canonical_payload(path: &Path) -> Result<CanonicalPayload, ()> {
    let payload = super::macho_payload::read_canonical_payload(path)?;
    Ok(CanonicalPayload {
        architecture: payload.architecture,
        bytes: payload.bytes,
    })
}

#[cfg(target_os = "windows")]
fn canonical_payload(path: &Path) -> Result<CanonicalPayload, ()> {
    let payload = super::pe_payload::read_canonical_payload(path)?;
    Ok(CanonicalPayload {
        architecture: payload.architecture,
        bytes: payload.bytes,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn canonical_payload(_path: &Path) -> Result<CanonicalPayload, ()> {
    Err(())
}

struct CanonicalPayload {
    architecture: &'static str,
    bytes: Vec<u8>,
}
