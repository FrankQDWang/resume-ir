use std::path::Path;
use std::time::Instant;

mod orchestrator;
mod parallel;
mod scan;
mod scheduler;

use meta_store::{
    ImportTask, ImportTaskFailure, ImportTaskId, ImportTaskStatus, OwnedMetaStore,
    SearchRepairReason, UnixTimestamp,
};

use crate::{
    current_import_processing_contract, index_recovery, ImportOptions, ImportPipelineError,
    ImportPipelineErrorClass, ImportPipelineErrorKind, ImportSummary, PipelineRunControl, Result,
};

use orchestrator::{import_scan_scope_from_summary, run_import};

#[cfg(test)]
pub(crate) use crate::file_processing::ParseWorkerClock;
pub(crate) use orchestrator::current_timestamp_or;
#[cfg(test)]
pub(crate) use orchestrator::{CancelCheckMetrics, ImportCancelPoller};
#[cfg(test)]
pub(crate) use scan::document_path_is_deletion_candidate;
#[cfg(test)]
pub(crate) use scheduler::{finish_import_file, should_flush_searchable_documents};

pub fn import_root(
    data_dir: &Path,
    store: &OwnedMetaStore,
    task: &ImportTask,
    root: &Path,
    now: UnixTimestamp,
) -> Result<ImportSummary> {
    import_root_with_options(data_dir, store, task, root, now, ImportOptions::default())
}

pub fn import_root_with_options(
    data_dir: &Path,
    store: &OwnedMetaStore,
    task: &ImportTask,
    root: &Path,
    now: UnixTimestamp,
    options: ImportOptions,
) -> Result<ImportSummary> {
    import_root_with_options_and_control(
        data_dir,
        store,
        task,
        root,
        now,
        options,
        PipelineRunControl::default(),
    )
}

pub fn import_root_with_options_and_control(
    data_dir: &Path,
    store: &OwnedMetaStore,
    task: &ImportTask,
    root: &Path,
    now: UnixTimestamp,
    options: ImportOptions,
    control: PipelineRunControl,
) -> Result<ImportSummary> {
    let import_started = Instant::now();
    let processing_contract = current_import_processing_contract(&options)?;
    let bound_contract_id = store
        .import_task_processing_contract_id(&task.id)
        .map_err(ImportPipelineError::store)?;
    if bound_contract_id.as_ref() != Some(processing_contract.id()) {
        return Err(ImportPipelineError::store_invariant());
    }
    let search_vectorization = options.search_vectorization.clone();
    let persisted_task = store
        .import_task_by_id(&task.id)
        .map_err(ImportPipelineError::store)?;
    if task.status != ImportTaskStatus::Running
        || task.finished_at.is_some()
        || !matches!(
            persisted_task.as_ref(),
            Some(persisted)
                if persisted.id == task.id
                    && persisted.root_path == task.root_path
                    && persisted.status == task.status
                    && persisted.queued_at == task.queued_at
                    && persisted.started_at == task.started_at
                    && persisted.finished_at.is_none()
        )
    {
        return Err(ImportPipelineError::store_invariant());
    }

    let import_result = run_import(
        data_dir,
        store,
        task,
        root,
        now,
        options,
        &processing_contract,
        &control,
    )
    .map_err(|error| normalize_shutdown_interruption(store, &task.id, &control, error));
    let finished_at = current_timestamp_or(now);
    let result = import_result.and_then(|summary| {
        let final_scope = import_scan_scope_from_summary(store, &task.id, &summary, finished_at)?
            .ok_or_else(ImportPipelineError::store_invariant)?;
        store
            .complete_import_task(
                &task.id,
                processing_contract.id(),
                &final_scope,
                finished_at,
            )
            .map_err(ImportPipelineError::store)?;
        Ok(summary)
    });
    match result {
        Ok(mut summary) => {
            let migration = index_recovery::finalize_migration_rebuild(
                store,
                finished_at,
                &processing_contract,
                &search_vectorization,
                &control,
            )?;
            if migration.active_generation_rebuilt {
                let ready_elapsed = import_started.elapsed();
                if summary.searchable_documents > 0 {
                    summary
                        .milestone_timings
                        .first_searchable
                        .get_or_insert(ready_elapsed);
                }
                summary
                    .milestone_timings
                    .full_index_ready
                    .get_or_insert(ready_elapsed);
                summary
                    .milestone_timings
                    .full_import_ready
                    .get_or_insert(ready_elapsed);
            }
            Ok(summary)
        }
        Err(error) => {
            let failure = if error.is_retryable() {
                ImportTaskFailure::Retryable
            } else {
                ImportTaskFailure::Permanent
            };
            let migration_block_reason = match error.class() {
                ImportPipelineErrorClass::SourceUnavailable => {
                    Some(SearchRepairReason::SourceUnavailable)
                }
                _ if !error.is_retryable() => Some(SearchRepairReason::RuntimeInvariant),
                _ => None,
            };
            store
                .fail_observed_import_task(task, failure, migration_block_reason, finished_at)
                .map_err(ImportPipelineError::store)?;
            Err(error)
        }
    }
}

fn normalize_shutdown_interruption(
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    control: &PipelineRunControl,
    error: ImportPipelineError,
) -> ImportPipelineError {
    if error.kind != ImportPipelineErrorKind::Cancelled || !control.shutdown_requested() {
        return error;
    }
    match store.is_import_task_cancelled(task_id) {
        Ok(false) => ImportPipelineError::interrupted(),
        Ok(true) | Err(_) => error,
    }
}
