use std::path::Path;

use sha2::{Digest, Sha256};

use crate::ipc::OptionalRuntimeReason;

use super::macho_payload::read_canonical_payload;

pub(super) struct PayloadIdentity {
    pub(super) architecture: &'static str,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}

pub(super) fn payload_identity(path: &Path) -> Result<PayloadIdentity, OptionalRuntimeReason> {
    let payload = read_canonical_payload(path).map_err(|_| OptionalRuntimeReason::Invalid)?;
    let mut digest = Sha256::new();
    digest.update(&payload.bytes);
    Ok(PayloadIdentity {
        architecture: payload.architecture,
        bytes: payload.bytes.len() as u64,
        sha256: format!("{:x}", digest.finalize()),
    })
}
