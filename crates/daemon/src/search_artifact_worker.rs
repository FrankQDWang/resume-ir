use import_pipeline::{
    finalize_migration_rebuild, reconcile_search_artifacts, PipelineRunControl,
    SearchArtifactRecoverySummary,
};
use meta_store::{
    ImportProcessingContract, OwnedMetaStore, SearchProjectionServiceState, SearchRepairReason,
    UnixTimestamp,
};

use crate::daemon_error::{DaemonError, Result};
use crate::run_options::RunOptions;
use crate::worker_time::current_timestamp;

pub(crate) fn run_search_artifact_worker_once(
    store: &OwnedMetaStore,
    options: &RunOptions,
    processing_contract: &ImportProcessingContract,
    control: &PipelineRunControl,
) -> Result<SearchArtifactRecoverySummary> {
    let migration = try_finalize_migration_rebuild(store, options, processing_contract, control)?;
    if migration.active_generation_rebuilt {
        return Ok(migration);
    }
    reconcile_search_artifacts(
        store,
        current_timestamp()?,
        &options.search_vectorization,
        control,
    )
    .map_err(DaemonError::import)
}

pub(crate) fn try_finalize_migration_rebuild(
    store: &OwnedMetaStore,
    options: &RunOptions,
    processing_contract: &ImportProcessingContract,
    control: &PipelineRunControl,
) -> Result<SearchArtifactRecoverySummary> {
    match finalize_migration_rebuild(
        store,
        current_timestamp()?,
        processing_contract,
        &options.search_vectorization,
        control,
    ) {
        Ok(summary) => Ok(summary),
        Err(error) => {
            if !error.is_retryable() {
                mark_migration_rebuild_blocked(
                    store,
                    SearchRepairReason::RuntimeInvariant,
                    current_timestamp()?,
                )?;
            }
            Ok(SearchArtifactRecoverySummary::default())
        }
    }
}

pub(crate) fn search_repair_is_blocked(store: &OwnedMetaStore) -> Result<bool> {
    Ok(store
        .search_projection_state()
        .map_err(DaemonError::store)?
        .service_state
        == SearchProjectionServiceState::RepairBlocked)
}

pub(crate) fn mark_migration_rebuild_blocked(
    store: &OwnedMetaStore,
    reason: SearchRepairReason,
    now: UnixTimestamp,
) -> Result<()> {
    let _ = store
        .block_migration_rebuild(reason, now)
        .map_err(DaemonError::store)?;
    Ok(())
}
