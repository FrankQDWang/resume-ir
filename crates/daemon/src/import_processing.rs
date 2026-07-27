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

#[allow(dead_code)] // production bootstrap uses upgrade_coordinator; retained for tests/CLI parity
pub(super) fn activate_contract(
    store: &OwnedMetaStore,
    contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<()> {
    match store
        .commit_online_writer_contract(contract, now)
        .map_err(DaemonError::store)?
    {
        meta_store::WriterContractTransitionOutcome::AlreadyActive
        | meta_store::WriterContractTransitionOutcome::TargetCommitted => Ok(()),
        meta_store::WriterContractTransitionOutcome::BlockedByRunningOwner => {
            Err(DaemonError::ownership_conflict())
        }
        meta_store::WriterContractTransitionOutcome::PersistedStateInvalid => {
            match store
                .activate_migration_rebuild_contract(contract, now)
                .map_err(DaemonError::store)?
            {
                MigrationRebuildContractActivation::Activated
                | MigrationRebuildContractActivation::AlreadyActive => Ok(()),
                MigrationRebuildContractActivation::Superseded => {
                    Err(DaemonError::recoverable_dependency(
                        "processing-contract online transition required",
                    ))
                }
                MigrationRebuildContractActivation::RunningTaskConflict => {
                    Err(DaemonError::ownership_conflict())
                }
            }
        }
        meta_store::WriterContractTransitionOutcome::TransitionRequired
        | meta_store::WriterContractTransitionOutcome::TransitionInProgress
        | meta_store::WriterContractTransitionOutcome::UnsupportedTransition
        | meta_store::WriterContractTransitionOutcome::RuntimeUnavailable => {
            Err(DaemonError::recoverable_dependency(
                "processing-contract writer transition unavailable",
            ))
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

/// Startup orphan reconciliation that projects owner contention onto a
/// writer-only barrier instead of failing the whole daemon as a core block.
pub(super) fn normalize_orphaned_running_tasks_for_writer_bootstrap(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
) -> Result<usize> {
    match store.normalize_orphaned_running_tasks(now) {
        Ok(recovered) => {
            store
                .clear_writer_blocked_by_running_owner(now)
                .map_err(DaemonError::store)?;
            Ok(recovered)
        }
        Err(ImportProcessingOrphanNormalizationError::TaskOwnerLockContended) => {
            store
                .mark_writer_blocked_by_running_owner(now)
                .map_err(DaemonError::store)?;
            Ok(0)
        }
        Err(ImportProcessingOrphanNormalizationError::Store(error)) => {
            Err(DaemonError::store(error))
        }
        Err(ImportProcessingOrphanNormalizationError::TaskOwnerLockStorage) => Err(
            DaemonError::recoverable_dependency("import task owner lock unavailable"),
        ),
    }
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
