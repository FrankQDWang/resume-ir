use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use tempfile::Builder;

use crate::{
    active_store_manifest::{
        sync_parent_directory, validate_owner_regular_metadata, ActiveStoreManifest,
    },
    schema_v29, schema_v30, schema_v31, schema_v32, schema_v33, schema_v34, schema_v35,
    MetaStoreError, Result,
};

pub(super) const FILE_NAME: &str = "metadata-forward-migration-receipt.v1";
const SCHEMA: &str = "resume-ir.metadata-forward-migration-receipt.v1";
const MAX_BYTES: u64 = 2 * 1024;
const STAGING_PREFIX: &str = ".metadata-forward-stage-";
const LEGACY_V30_STAGING_PREFIX: &str = ".metadata-v30-stage-";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ReceiptPhase {
    Preparing,
    Ready,
    Published,
}

impl ReceiptPhase {
    fn label(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Ready => "ready",
            Self::Published => "published",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct MigrationReceipt {
    pub(super) phase: ReceiptPhase,
    pub(super) migration_id: String,
    pub(super) source: ActiveStoreManifest,
    pub(super) staging_file: String,
    pub(super) target: ActiveStoreManifest,
}

pub(super) fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

pub(super) fn persist(data_dir: &Path, receipt: &MigrationReceipt) -> Result<()> {
    validate(receipt)?;
    crate::active_store_manifest::owner_regular_file_exists(&path(data_dir))?;
    let body = format!(
        "{SCHEMA}\nphase={}\nid={}\nsource_file={}\nsource_schema={}\nsource_digest={}\nstaging_file={}\ntarget_file={}\ntarget_schema={}\ntarget_digest={}\n",
        receipt.phase.label(),
        receipt.migration_id,
        receipt.source.file_name,
        receipt.source.schema_version,
        receipt.source.store_id_digest,
        receipt.staging_file,
        receipt.target.file_name,
        receipt.target.schema_version,
        receipt.target.store_id_digest,
    );
    let mut temporary = Builder::new()
        .prefix(".metadata-forward-migration-receipt-")
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

pub(super) fn read(path: &Path) -> Result<MigrationReceipt> {
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
    let mut fields = BTreeMap::new();
    for line in lines {
        let (key, value) = line
            .split_once('=')
            .ok_or_else(MetaStoreError::storage_invariant)?;
        if fields.insert(key, value).is_some() {
            return Err(MetaStoreError::storage_invariant());
        }
    }
    let required = |key| {
        fields
            .get(key)
            .copied()
            .ok_or_else(MetaStoreError::storage_invariant)
    };
    let schema = |key| {
        required(key)?
            .parse::<u32>()
            .map_err(|_| MetaStoreError::storage_invariant())
    };
    let receipt = MigrationReceipt {
        phase: match required("phase")? {
            "preparing" => ReceiptPhase::Preparing,
            "ready" => ReceiptPhase::Ready,
            "published" => ReceiptPhase::Published,
            _ => return Err(MetaStoreError::storage_invariant()),
        },
        migration_id: required("id")?.to_string(),
        source: ActiveStoreManifest {
            file_name: required("source_file")?.to_string(),
            schema_version: schema("source_schema")?,
            store_id_digest: required("source_digest")?.to_string(),
        },
        staging_file: required("staging_file")?.to_string(),
        target: ActiveStoreManifest {
            file_name: required("target_file")?.to_string(),
            schema_version: schema("target_schema")?,
            store_id_digest: required("target_digest")?.to_string(),
        },
    };
    if fields.len() != 9 {
        return Err(MetaStoreError::storage_invariant());
    }
    validate(&receipt)?;
    Ok(receipt)
}

fn validate(receipt: &MigrationReceipt) -> Result<()> {
    if receipt.migration_id.len() != 64
        || !receipt
            .migration_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(MetaStoreError::storage_invariant());
    }
    let staging_suffix = format!("{}.sqlite3", &receipt.migration_id[..16]);
    let current_staging = format!("{STAGING_PREFIX}{staging_suffix}");
    let legacy_v30_staging = format!("{LEGACY_V30_STAGING_PREFIX}{staging_suffix}");
    let expected_target = format!(
        "metadata-v{}-{}.sqlite3",
        receipt.target.schema_version,
        &receipt.migration_id[..16],
    );
    let legacy_v30_pair = receipt.source.schema_version == schema_v29::VERSION
        && receipt.target.schema_version == schema_v30::VERSION
        && receipt.staging_file == legacy_v30_staging;
    let current_pair = matches!(
        receipt.source.schema_version,
        schema_v29::VERSION
            | schema_v30::VERSION
            | schema_v31::VERSION
            | schema_v32::VERSION
            | schema_v33::VERSION
            | schema_v34::VERSION
    ) && receipt.target.schema_version == schema_v35::VERSION
        && receipt.staging_file == current_staging;
    if (!legacy_v30_pair && !current_pair)
        || receipt.source.store_id_digest != receipt.target.store_id_digest
        || receipt.target.file_name != expected_target
    {
        return Err(MetaStoreError::storage_invariant());
    }
    Ok(())
}
