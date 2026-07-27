use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use tempfile::Builder;

use crate::{
    active_store_manifest::{
        sync_parent_directory, validate_owner_regular_metadata, ActiveStoreManifest,
    },
    schema_v34, MetaStoreError, Result,
};

pub(super) const FILE_NAME: &str = "metadata-initialization-receipt.v1";
const SCHEMA: &str = "resume-ir.metadata-initialization-receipt.v1";
const MAX_BYTES: u64 = 1024;
const STAGING_PREFIX: &str = ".metadata-v34-init-";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum InitializationPhase {
    Preparing,
    Ready,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct InitializationReceipt {
    phase: InitializationPhase,
    initialization_id: String,
    store_id_digest: Option<String>,
}

impl InitializationReceipt {
    pub(super) fn new(initialization_id: String) -> Self {
        Self {
            phase: InitializationPhase::Preparing,
            initialization_id,
            store_id_digest: None,
        }
    }

    pub(super) fn phase(&self) -> InitializationPhase {
        self.phase
    }

    pub(super) fn staging_file(&self) -> String {
        format!("{STAGING_PREFIX}{}.sqlite3", &self.initialization_id[..16])
    }

    pub(super) fn target_file(&self) -> String {
        format!("metadata-v34-{}.sqlite3", &self.initialization_id[..16])
    }

    pub(super) fn mark_ready(&mut self, store_id_digest: String) {
        self.phase = InitializationPhase::Ready;
        self.store_id_digest = Some(store_id_digest);
    }

    pub(super) fn target_manifest(&self) -> Option<ActiveStoreManifest> {
        self.store_id_digest
            .as_ref()
            .map(|store_id_digest| ActiveStoreManifest {
                file_name: self.target_file(),
                schema_version: schema_v34::VERSION,
                store_id_digest: store_id_digest.clone(),
            })
    }
}

pub(super) fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

pub(super) fn persist(data_dir: &Path, receipt: &InitializationReceipt) -> Result<()> {
    validate(receipt)?;
    let digest = receipt.store_id_digest.as_deref().unwrap_or("-");
    let phase = match receipt.phase {
        InitializationPhase::Preparing => "preparing",
        InitializationPhase::Ready => "ready",
    };
    let body = format!(
        "{SCHEMA}\nphase={phase}\nid={}\ndigest={digest}\n",
        receipt.initialization_id,
    );
    let mut temporary = Builder::new()
        .prefix(".metadata-initialization-receipt-")
        .tempfile_in(data_dir)
        .map_err(MetaStoreError::io_storage)?;
    temporary
        .write_all(body.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(MetaStoreError::io_storage)?;
    crate::restrict_private_file_permissions(temporary.path())?;
    temporary
        .persist(path(data_dir))
        .map_err(|error| MetaStoreError::io_storage(error.error))?;
    sync_parent_directory(data_dir)
}

pub(super) fn read(path: &Path) -> Result<InitializationReceipt> {
    let metadata = fs::symlink_metadata(path).map_err(MetaStoreError::io_storage)?;
    validate_owner_regular_metadata(&metadata)?;
    if metadata.len() > MAX_BYTES {
        return Err(MetaStoreError::storage_invariant());
    }
    let value = fs::read_to_string(path).map_err(MetaStoreError::io_storage)?;
    let mut lines = value.lines();
    if lines.next() != Some(SCHEMA) {
        return Err(MetaStoreError::storage_invariant());
    }
    let phase = lines
        .next()
        .and_then(|line| line.strip_prefix("phase="))
        .ok_or_else(MetaStoreError::storage_invariant)?;
    let initialization_id = lines
        .next()
        .and_then(|line| line.strip_prefix("id="))
        .ok_or_else(MetaStoreError::storage_invariant)?
        .to_string();
    let digest = lines
        .next()
        .and_then(|line| line.strip_prefix("digest="))
        .ok_or_else(MetaStoreError::storage_invariant)?;
    if lines.next().is_some() {
        return Err(MetaStoreError::storage_invariant());
    }
    let receipt = InitializationReceipt {
        phase: match phase {
            "preparing" => InitializationPhase::Preparing,
            "ready" => InitializationPhase::Ready,
            _ => return Err(MetaStoreError::storage_invariant()),
        },
        initialization_id,
        store_id_digest: (digest != "-").then(|| digest.to_string()),
    };
    validate(&receipt)?;
    Ok(receipt)
}

pub(super) fn remove(data_dir: &Path) -> Result<()> {
    match fs::symlink_metadata(path(data_dir)) {
        Ok(metadata) => {
            validate_owner_regular_metadata(&metadata)?;
            fs::remove_file(path(data_dir)).map_err(MetaStoreError::io_storage)?;
            sync_parent_directory(data_dir)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(MetaStoreError::io_storage(error)),
    }
}

fn validate(receipt: &InitializationReceipt) -> Result<()> {
    let valid_id = receipt.initialization_id.len() == 64
        && receipt
            .initialization_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    let valid_digest = receipt.store_id_digest.as_ref().is_none_or(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    let valid_phase = matches!(
        (receipt.phase, receipt.store_id_digest.is_some()),
        (InitializationPhase::Preparing, false) | (InitializationPhase::Ready, true)
    );
    if !valid_id || !valid_digest || !valid_phase {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}
