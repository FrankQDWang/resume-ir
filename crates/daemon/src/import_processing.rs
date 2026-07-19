use std::path::Path;

use import_pipeline::{
    current_import_processing_contract, DataDirectoryOwnerAcquireError,
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportOptions,
    ImportProcessingOrphanNormalizationError,
};
use meta_store::{
    ImportProcessingContract, ImportTaskId, MigrationRebuildContractActivation, OwnedMetaStore,
    UnixTimestamp,
};

use super::{DaemonError, Result, RunOptions};

pub(super) fn current_contract(options: &RunOptions) -> Result<ImportProcessingContract> {
    current_import_processing_contract(&ImportOptions {
        linear_promotion: options.linear_promotion.clone(),
        search_vectorization: options.search_vectorization.clone(),
        ..ImportOptions::default()
    })
    .map_err(DaemonError::import)
}

pub(super) fn activate_contract(
    store: &OwnedMetaStore,
    contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<()> {
    match store
        .activate_migration_rebuild_contract(contract, now)
        .map_err(DaemonError::store)?
    {
        MigrationRebuildContractActivation::Activated
        | MigrationRebuildContractActivation::AlreadyActive
        | MigrationRebuildContractActivation::Superseded => Ok(()),
        MigrationRebuildContractActivation::RunningTaskConflict => {
            Err(DaemonError::ownership_conflict())
        }
    }
}

pub(super) fn acquire_owner(data_dir: &Path) -> Result<DataDirectoryOwnerLease> {
    match DataDirectoryOwnerLease::try_acquire(data_dir) {
        Ok(DataDirectoryOwnerAcquisition::Acquired(lease)) => Ok(lease),
        Ok(DataDirectoryOwnerAcquisition::Contended) => Err(DaemonError::ownership_conflict()),
        Err(DataDirectoryOwnerAcquireError::RuntimeIntegrity) => {
            Err(DaemonError::runtime_integrity())
        }
        Err(DataDirectoryOwnerAcquireError::Storage) => Err(DaemonError::recoverable_dependency(
            "data-directory owner storage unavailable",
        )),
    }
}

pub(super) fn normalize_orphaned_running_tasks(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
) -> Result<usize> {
    store
        .normalize_orphaned_running_tasks(now)
        .map_err(|error| match error {
            ImportProcessingOrphanNormalizationError::Store(error) => DaemonError::store(error),
            ImportProcessingOrphanNormalizationError::TaskOwnerLockStorage => {
                DaemonError::recoverable_dependency("import task owner lock unavailable")
            }
            ImportProcessingOrphanNormalizationError::TaskOwnerLockContended => {
                DaemonError::ownership_conflict()
            }
        })
}

pub(super) fn task_matches_contract(
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    contract: &ImportProcessingContract,
) -> Result<bool> {
    store
        .import_task_processing_contract_id(task_id)
        .map(|bound| bound.as_ref() == Some(contract.id()))
        .map_err(DaemonError::store)
}
