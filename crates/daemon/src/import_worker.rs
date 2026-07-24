use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use import_pipeline::{
    import_root_with_options_and_control, ImportOptions, ImportPipelineErrorClass,
    ImportTaskOwnerLock, PipelineRunControl, ScanProfile,
};
use meta_store::{
    ImportProcessingContract, ImportScanBudgetKind, ImportScanProfile, ImportScanScope,
    ImportTaskFailure, ImportTaskId, ImportTaskStatus, OwnedMetaStore, SearchRepairReason,
    UnixTimestamp,
};

use crate::daemon_error::{DaemonError, Result};
use crate::run_options::RunOptions;
use crate::search_artifact_worker::{
    mark_migration_rebuild_blocked, search_repair_is_blocked, try_finalize_migration_rebuild,
};
use crate::worker_output::ImportWorkerSummary;
use crate::worker_time::{
    current_timestamp, timestamp_at_or_after, timestamp_minus_seconds, u64_to_usize,
};
use crate::{import_processing, migration_repair};

const IMPORT_TASK_HEARTBEAT_SECONDS: u64 = 30;

pub(crate) fn run_import_worker_once(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    processing_contract: &ImportProcessingContract,
) -> Result<ImportWorkerSummary> {
    let retryable_due_at = current_timestamp()?;
    let mut summary = ImportWorkerSummary {
        repair_requeued: migration_repair::reconcile_authorized_roots(
            store,
            processing_contract,
            retryable_due_at,
        )?,
        ..ImportWorkerSummary::default()
    };
    summary.extend(run_import_worker_once_with_retry_due(
        data_dir,
        store,
        options,
        processing_contract,
        retryable_due_at,
        PipelineRunControl::default(),
        || true,
    )?);
    Ok(summary)
}

pub(crate) fn run_import_worker_once_with_retry_due(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    processing_contract: &ImportProcessingContract,
    retryable_due_at: UnixTimestamp,
    run_control: PipelineRunControl,
    claim_allowed: impl Fn() -> bool,
) -> Result<ImportWorkerSummary> {
    let mut worker_summary = ImportWorkerSummary::default();
    let mut attempted = Vec::<ImportTaskId>::new();

    while !run_control.shutdown_requested() {
        if search_repair_is_blocked(store)? {
            break;
        }
        if !claim_allowed() {
            break;
        }
        let Some(candidate) = store
            .import_task_claim_candidate_for_worker_excluding_due_at(retryable_due_at, &attempted)
            .map_err(DaemonError::store)?
        else {
            break;
        };
        attempted.push(candidate.id.clone());
        if !import_processing::task_matches_contract(store, &candidate.id, processing_contract)? {
            store
                .cancel_import_task(
                    &candidate.id,
                    timestamp_at_or_after(current_timestamp()?, candidate.updated_at),
                )
                .map_err(DaemonError::store)?;
            worker_summary.failed += 1;
            continue;
        }
        let owner_lock = match ImportTaskOwnerLock::try_acquire(data_dir, &candidate.id) {
            Ok(Some(owner_lock)) => owner_lock,
            Ok(None) => continue,
            Err(_) => {
                mark_migration_rebuild_blocked(
                    store,
                    SearchRepairReason::RuntimeInvariant,
                    current_timestamp()?,
                )?;
                worker_summary.failed += 1;
                continue;
            }
        };
        if search_repair_is_blocked(store)? {
            drop(owner_lock);
            break;
        }
        if !claim_allowed() {
            drop(owner_lock);
            break;
        }
        let Some(task) = store
            .claim_observed_import_task_for_worker(&candidate, current_timestamp()?)
            .map_err(DaemonError::store)?
        else {
            drop(owner_lock);
            continue;
        };
        let now = task.updated_at;
        let Some(scope) = store
            .import_scan_scope_by_task_id(&task.id)
            .map_err(DaemonError::store)?
        else {
            let _ = store
                .fail_observed_import_task(
                    &task,
                    ImportTaskFailure::Permanent,
                    Some(SearchRepairReason::RuntimeInvariant),
                    now,
                )
                .map_err(DaemonError::store)?;
            worker_summary.failed += 1;
            continue;
        };

        let import_options = match import_options_from_scope(&scope, options) {
            Ok(import_options) => import_options,
            Err(_) => {
                let _ = store
                    .fail_observed_import_task(
                        &task,
                        ImportTaskFailure::Permanent,
                        Some(SearchRepairReason::RuntimeInvariant),
                        now,
                    )
                    .map_err(DaemonError::store)?;
                worker_summary.failed += 1;
                continue;
            }
        };
        let heartbeat = ImportTaskHeartbeat::start(store, task.id.clone())?;
        let import_result = import_root_with_options_and_control(
            data_dir,
            store,
            &task,
            Path::new(&scope.canonical_root_path),
            now,
            import_options,
            run_control.clone(),
        );
        drop(heartbeat);
        let import_summary = match import_result {
            Ok(import_summary) => import_summary,
            Err(error) => {
                worker_summary.failure_class = Some(error.class());
                worker_summary.metadata_failure_class = error.metadata_class_label();
                let user_cancelled = store
                    .is_import_task_cancelled(&task.id)
                    .map_err(DaemonError::store)?;
                if should_requeue_interrupted_import(
                    error.class(),
                    run_control.shutdown_requested(),
                    user_cancelled,
                ) {
                    let interrupted = store
                        .import_task_by_id(&task.id)
                        .map_err(DaemonError::store)?
                        .ok_or_else(|| DaemonError::control_plane("import task disappeared"))?;
                    if interrupted.status == ImportTaskStatus::FailedRetryable {
                        store
                            .requeue_interrupted_import_task(
                                &task.id,
                                interrupted.updated_at,
                                current_timestamp()?,
                            )
                            .map_err(DaemonError::store)?;
                    }
                }
                if user_cancelled {
                    worker_summary.cancelled += 1;
                } else {
                    worker_summary.failed += 1;
                }
                continue;
            }
        };

        worker_summary.processed += 1;
        worker_summary.searchable_documents += import_summary.searchable_documents;
        worker_summary.ocr_jobs_queued += import_summary.ocr_jobs_queued;
    }

    if claim_allowed() {
        let _ = try_finalize_migration_rebuild(store, options, processing_contract, &run_control)?;
    }
    Ok(worker_summary)
}

pub(crate) fn should_requeue_interrupted_import(
    error_class: ImportPipelineErrorClass,
    shutdown_requested: bool,
    durable_user_cancelled: bool,
) -> bool {
    shutdown_requested
        && !durable_user_cancelled
        && matches!(
            error_class,
            ImportPipelineErrorClass::Cancelled | ImportPipelineErrorClass::Interrupted
        )
}

pub(crate) fn recover_stale_import_tasks(
    data_dir: &Path,
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    now: UnixTimestamp,
    stale_seconds: i64,
) -> Result<usize> {
    let stale_before = timestamp_minus_seconds(now, stale_seconds);
    let task_ids = store
        .running_import_task_ids()
        .map_err(DaemonError::store)?;
    let mut recovered = 0_usize;
    for task_id in task_ids {
        let Some(observed) = store
            .import_task_by_id(&task_id)
            .map_err(DaemonError::store)?
        else {
            continue;
        };
        if observed.updated_at.as_unix_seconds() > stale_before.as_unix_seconds() {
            continue;
        }
        let Some(owner_probe) = ImportTaskOwnerLock::try_acquire(data_dir, &task_id)
            .map_err(|_| DaemonError::recoverable_dependency("import owner lock unavailable"))?
        else {
            continue;
        };
        let Some(task) = store
            .import_task_by_id(&task_id)
            .map_err(DaemonError::store)?
        else {
            continue;
        };
        if task.status != ImportTaskStatus::Running
            || task.updated_at.as_unix_seconds() > stale_before.as_unix_seconds()
        {
            continue;
        }
        if !import_processing::task_matches_contract(store, &task.id, processing_contract)? {
            store
                .cancel_import_task(&task.id, timestamp_at_or_after(now, task.updated_at))
                .map_err(DaemonError::store)?;
            continue;
        }
        if store
            .requeue_running_import_task(&task_id, task.updated_at, now)
            .map_err(DaemonError::store)?
        {
            recovered += 1;
        }
        drop(owner_probe);
    }
    Ok(recovered)
}

fn import_options_from_scope(
    scope: &ImportScanScope,
    options: &RunOptions,
) -> Result<ImportOptions> {
    Ok(ImportOptions {
        scan_profile: match scope.scan_profile {
            ImportScanProfile::Explicit => ScanProfile::Explicit,
            ImportScanProfile::Discovery => ScanProfile::Discovery,
        },
        max_files: match (scope.scan_budget_kind, scope.scan_budget_limit) {
            (Some(ImportScanBudgetKind::Files), Some(limit)) => Some(u64_to_usize(limit)?),
            (None, None) => None,
            _ => {
                return Err(DaemonError::user(
                    "queued import task has invalid scan budget metadata",
                ));
            }
        },
        linear_promotion: options.linear_promotion.clone(),
        search_vectorization: options.search_vectorization.clone(),
        ..ImportOptions::default()
    })
}

struct ImportTaskHeartbeat {
    stop: Option<mpsc::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ImportTaskHeartbeat {
    fn start(store: &OwnedMetaStore, task_id: ImportTaskId) -> Result<Self> {
        let (stop, stop_receiver) = mpsc::channel();
        let store = store.open_sibling().map_err(DaemonError::store)?;

        let worker = thread::spawn(move || loop {
            match stop_receiver.recv_timeout(Duration::from_secs(IMPORT_TASK_HEARTBEAT_SECONDS)) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let Ok(now) = current_timestamp() else {
                        continue;
                    };
                    let _ = store.heartbeat_running_import_task(&task_id, now);
                }
            }
        });

        Ok(Self {
            stop: Some(stop),
            worker: Some(worker),
        })
    }
}

impl Drop for ImportTaskHeartbeat {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
