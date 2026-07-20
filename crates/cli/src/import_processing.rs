use std::path::Path;

use import_pipeline::{
    current_import_processing_contract, finalize_migration_rebuild, DataDirectoryOwnerAcquireError,
    DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, ImportOptions,
    ImportProcessingOrphanNormalizationError, SearchPublicationVectorization,
};
use meta_store::{
    ImportProcessingContract, ImportRootTaskHeadOutcome, ImportRootTaskHeadRequest,
    ImportScanScope, ImportTask, ImportTaskPurpose, MigrationRebuildContractActivation,
    OwnedMetaStore, SearchProjectionServiceState, SearchRepairReason, UnixTimestamp,
};

use super::{CliError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OfflineImportProcessingMutation {
    DirectImport,
    DirectDelete,
    PurgeDeleted,
    DirectCancel,
    TaskControl,
    OcrWorker,
    DoctorRecovery,
    SyntheticFaultProbe,
    PrivateWitness,
}

/// Acquires the single data-directory owner before any offline command that
/// mutates import-task lifecycle or its publication/deletion boundary.
///
pub(super) fn acquire_owner_for_mutation(
    data_dir: &Path,
    _mutation: OfflineImportProcessingMutation,
) -> Result<DataDirectoryOwnerLease> {
    acquire_owner(data_dir)
}

pub(super) fn current_contract(options: &ImportOptions) -> Result<ImportProcessingContract> {
    current_import_processing_contract(options).map_err(CliError::import)
}

pub(super) fn activate_contract(
    store: &OwnedMetaStore,
    contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<()> {
    match store
        .activate_migration_rebuild_contract(contract, now)
        .map_err(CliError::store)?
    {
        MigrationRebuildContractActivation::Activated
        | MigrationRebuildContractActivation::AlreadyActive
        | MigrationRebuildContractActivation::Superseded => Ok(()),
        MigrationRebuildContractActivation::RunningTaskConflict => Err(CliError::user(
            "import processing contract activation is blocked by a running task",
        )),
    }
}

pub(super) fn acquire_owner(data_dir: &Path) -> Result<DataDirectoryOwnerLease> {
    match DataDirectoryOwnerLease::try_acquire(data_dir) {
        Ok(DataDirectoryOwnerAcquisition::Acquired(lease)) => Ok(lease),
        Ok(DataDirectoryOwnerAcquisition::Contended) => {
            Err(CliError::user("offline import processing is already owned"))
        }
        Err(DataDirectoryOwnerAcquireError::Storage) => Err(CliError::user(
            "data-directory owner storage is unavailable",
        )),
        Err(DataDirectoryOwnerAcquireError::RuntimeIntegrity) => {
            Err(CliError::user("data-directory owner lock integrity failed"))
        }
    }
}

pub(super) fn normalize_orphaned_running_tasks(
    store: &OwnedMetaStore,
    now: UnixTimestamp,
) -> Result<usize> {
    store
        .normalize_orphaned_running_tasks(now)
        .map_err(|error| match error {
            ImportProcessingOrphanNormalizationError::Store(error) => CliError::store(error),
            ImportProcessingOrphanNormalizationError::TaskOwnerLockStorage => {
                CliError::user("unable to inspect import task owner lock")
            }
            ImportProcessingOrphanNormalizationError::TaskOwnerLockContended => {
                CliError::user("offline import processing conflicts with a legacy task owner")
            }
        })
}

pub(super) fn ensure_local_import_ready(
    store: &OwnedMetaStore,
    contract: &ImportProcessingContract,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
) -> Result<()> {
    finalize_migration_rebuild(
        store,
        now,
        contract,
        vectorization,
        &import_pipeline::PipelineRunControl::default(),
    )
    .map_err(CliError::import)?;
    let state = store.search_projection_state().map_err(CliError::store)?;
    if state.service_state == SearchProjectionServiceState::Repairing
        && state.repair_reason == Some(SearchRepairReason::MigrationRebuild)
        && state.generation.is_none()
    {
        return Err(CliError::user(
            "offline import is blocked until migration rebuild completes",
        ));
    }
    if state.service_state == SearchProjectionServiceState::RepairBlocked {
        return Err(CliError::user(
            "offline import is blocked by migration repair failure",
        ));
    }
    Ok(())
}

pub(super) fn claim_task_for_local_execution(
    store: &OwnedMetaStore,
    observed: &ImportTask,
    now: UnixTimestamp,
) -> Result<ImportTask> {
    store
        .claim_observed_import_task_for_worker(observed, now)
        .map_err(CliError::store)?
        .ok_or_else(|| CliError::user("import task is no longer claimable"))
}

pub(super) fn insert_new_configured_task_head(
    store: &OwnedMetaStore,
    task: &ImportTask,
    scope: &ImportScanScope,
    contract: &ImportProcessingContract,
) -> Result<()> {
    let outcome = store
        .coordinate_import_root_task_head(ImportRootTaskHeadRequest::Configured {
            task,
            scope,
            processing_contract: contract,
        })
        .map_err(CliError::store)?;
    if matches!(
        outcome,
        ImportRootTaskHeadOutcome::HeadInserted {
            task: persisted_task,
            scope: persisted_scope,
            purpose: ImportTaskPurpose::ConfiguredCatchUp,
            ..
        } if persisted_task.id == task.id && persisted_scope.import_task_id == task.id
    ) {
        return Ok(());
    }
    Err(CliError::user(
        "configured import task head was not inserted",
    ))
}
