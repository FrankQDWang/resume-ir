use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::str::FromStr;

use meta_store::ContentDigest;
use sha2::{Digest, Sha256};

use super::{ImportPipelineError, Result};

pub(super) fn stream_content_digest(
    path: &Path,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
) -> Result<Option<(ContentDigest, u64)>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut byte_size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        ensure_not_cancelled()?;
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(_) => return Ok(None),
        };
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        byte_size = byte_size
            .checked_add(u64::try_from(read).map_err(|_| ImportPipelineError::store_invariant())?)
            .ok_or_else(ImportPipelineError::store_invariant)?;
    }
    let digest = ContentDigest::from_str(&format!("sha256:{:x}", hasher.finalize()))
        .map_err(|_| ImportPipelineError::store_invariant())?;
    Ok(Some((digest, byte_size)))
}
