use std::path::Path;

use import_pipeline::DataDirectoryOwnerLease;
use meta_store::{IndexStateStatus, OwnedMetaStore, ReadMetaStore};

use crate::daemon_error::{DaemonError, Result};

pub(crate) fn open_store(data_dir: &Path) -> Result<ReadMetaStore> {
    ReadMetaStore::open_data_dir(data_dir).map_err(DaemonError::store)
}

pub(crate) fn open_owned_store(owner: &DataDirectoryOwnerLease) -> Result<OwnedMetaStore> {
    owner.open_store().map_err(DaemonError::store)
}

pub(crate) fn index_health_label(status: IndexStateStatus) -> &'static str {
    match status {
        IndexStateStatus::Empty => "empty",
        IndexStateStatus::Building => "building",
        IndexStateStatus::Ready => "ready",
        IndexStateStatus::Stale => "stale",
    }
}
