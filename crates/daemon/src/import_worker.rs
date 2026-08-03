use std::cell::Cell;
use std::path::Path;
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use import_pipeline::{
    finish_source_scan_failure, finish_source_scan_success, import_root_with_options_and_control,
    import_root_with_options_control_and_handoff, ImportOptions, ImportPipelineErrorClass,
    ImportTaskOwnerLock, PipelineRunControl, ScanProfile, SearchGenerationHandoff,
    SearchGenerationPublicationDisposition,
};
use meta_store::{
    ImportProcessingContract, ImportScanBudgetKind, ImportScanProfile, ImportScanScope,
    ImportTaskFailure, ImportTaskId, ImportTaskStatus, OwnedMetaStore, ScanTrigger,
    SearchRepairReason, UnixTimestamp,
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
    crate::source_root_deletion::resume_pending(data_dir, store, processing_contract)
        .map_err(|_| DaemonError::recoverable_dependency("source root deletion recovery failed"))?;
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
    run_import_worker_once_with_retry_due_and_handoff(
        data_dir,
        store,
        options,
        processing_contract,
        retryable_due_at,
        run_control,
        claim_allowed,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_import_worker_once_with_retry_due_and_handoff(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    processing_contract: &ImportProcessingContract,
    retryable_due_at: UnixTimestamp,
    run_control: PipelineRunControl,
    claim_allowed: impl Fn() -> bool,
    generation_handoff: Option<&crate::ipc::search_service::GenerationHandoff>,
) -> Result<ImportWorkerSummary> {
    let mut worker_summary = ImportWorkerSummary::default();
    let mut attempted = Vec::<ImportTaskId>::new();
    let mut reprocess_schedule_checked = false;

    while !run_control.shutdown_requested() {
        if search_repair_is_blocked(store)? {
            break;
        }
        if !claim_allowed() {
            break;
        }
        let candidate = store
            .import_task_claim_candidate_for_worker_excluding_due_at(retryable_due_at, &attempted)
            .map_err(DaemonError::store)?;
        let Some(candidate) = candidate else {
            if !reprocess_schedule_checked
                && options.pdf_import == import_pipeline::PdfImportPolicy::Enabled
            {
                reprocess_schedule_checked = true;
                if schedule_next_pdf_reprocess(store, processing_contract, retryable_due_at)? {
                    continue;
                }
            }
            break;
        };
        attempted.push(candidate.id.clone());
        let source_root = store
            .source_root_by_canonical_path(&candidate.root_path)
            .map_err(DaemonError::store)?;
        let source_root_is_deleting = match source_root {
            Some(root) => store
                .source_root_deletion_in_progress(&root.id)
                .map_err(DaemonError::store)?,
            None => false,
        };
        if source_root_is_deleting {
            store
                .cancel_import_task(
                    &candidate.id,
                    timestamp_at_or_after(current_timestamp()?, candidate.updated_at),
                )
                .map_err(DaemonError::store)?;
            worker_summary.cancelled += 1;
            continue;
        }
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
        let generation_adapter = generation_handoff.map(|handoff| MainImportGenerationHandoff {
            data_dir,
            handoff,
            control_unresponsive: Cell::new(false),
            activation_failed: Cell::new(false),
        });
        let import_result = if let Some(generation_adapter) = generation_adapter.as_ref() {
            import_root_with_options_control_and_handoff(
                data_dir,
                store,
                &task,
                Path::new(&scope.canonical_root_path),
                now,
                import_options,
                run_control.clone(),
                Some(generation_adapter),
            )
        } else {
            import_root_with_options_and_control(
                data_dir,
                store,
                &task,
                Path::new(&scope.canonical_root_path),
                now,
                import_options,
                run_control.clone(),
            )
        };
        drop(heartbeat);
        if generation_adapter
            .as_ref()
            .is_some_and(|adapter| adapter.control_unresponsive.get())
        {
            return Err(DaemonError::control_plane(
                "query generation control became unresponsive during import publication",
            ));
        }
        if generation_adapter
            .as_ref()
            .is_some_and(|adapter| adapter.activation_failed.get())
        {
            return Err(DaemonError::control_plane(
                "prepared main-import query generation could not be activated",
            ));
        }
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
                finish_source_scan_failure(
                    store,
                    &scope.canonical_root_path,
                    &task.id,
                    processing_contract,
                    current_timestamp()?,
                )
                .map_err(DaemonError::store)?;
                continue;
            }
        };

        crate::ipc::routes::status::record_latest_import_attribution(&task.id, &import_summary);
        finish_source_scan_success(
            store,
            &scope.canonical_root_path,
            &task.id,
            processing_contract,
            &import_summary,
            current_timestamp()?,
        )
        .map_err(DaemonError::store)?;
        worker_summary.processed += 1;
        worker_summary.searchable_documents += import_summary.searchable_documents;
        worker_summary.ocr_jobs_queued += import_summary.ocr_jobs_queued;
    }

    if claim_allowed() {
        let _ = try_finalize_migration_rebuild(store, options, processing_contract, &run_control)?;
    }
    Ok(worker_summary)
}

struct MainImportGenerationHandoff<'a> {
    data_dir: &'a Path,
    handoff: &'a crate::ipc::search_service::GenerationHandoff,
    control_unresponsive: Cell<bool>,
    activation_failed: Cell<bool>,
}

impl SearchGenerationHandoff for MainImportGenerationHandoff<'_> {
    fn stage(
        &self,
        publication: &meta_store::SearchPublicationRecord,
        projections: &[meta_store::ActiveSearchProjection],
    ) -> import_pipeline::Result<()> {
        let prepared =
            search_runtime::PreparedQueryGeneration::open(self.data_dir, publication, projections)
                .map_err(|_| {
                    import_pipeline::ImportPipelineError::query_generation_preparation()
                })?;
        match self.handoff.stage(prepared) {
            Ok(true) => Ok(()),
            Ok(false) => Err(import_pipeline::ImportPipelineError::query_generation_preparation()),
            Err(_) => {
                self.control_unresponsive.set(true);
                Err(import_pipeline::ImportPipelineError::query_generation_preparation())
            }
        }
    }

    fn finish(
        &self,
        disposition: SearchGenerationPublicationDisposition,
    ) -> import_pipeline::Result<()> {
        let daemon_disposition = match disposition {
            SearchGenerationPublicationDisposition::Committed => {
                crate::ipc::search_service::PublicationDisposition::Committed
            }
            SearchGenerationPublicationDisposition::Aborted => {
                crate::ipc::search_service::PublicationDisposition::Aborted
            }
        };
        match self.handoff.finish_publication(daemon_disposition) {
            Ok(true) => Ok(()),
            Ok(false) if disposition == SearchGenerationPublicationDisposition::Aborted => Ok(()),
            Ok(false) => {
                self.activation_failed.set(true);
                Err(import_pipeline::ImportPipelineError::query_generation_preparation())
            }
            Err(_) => {
                self.control_unresponsive.set(true);
                Err(import_pipeline::ImportPipelineError::query_generation_preparation())
            }
        }
    }
}

fn schedule_next_pdf_reprocess(
    store: &OwnedMetaStore,
    processing_contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<bool> {
    let parser_contract = processing_contract.primary_parse_version();
    let Some(root) = store
        .next_pdf_reprocess_root(parser_contract)
        .map_err(DaemonError::store)?
    else {
        return Ok(false);
    };
    let snapshot = crate::source_scan_coordinator::enqueue(
        store,
        processing_contract,
        &root,
        ScanTrigger::Recovery,
        now,
    )
    .map_err(|_| DaemonError::recoverable_dependency("PDF reprocess scan enqueue failed"))?;
    let task_id = ImportTaskId::from_str(&snapshot.id)
        .map_err(|_| DaemonError::control_plane("PDF reprocess scan id was invalid"))?;
    let scheduled = store
        .mark_pdf_reprocess_root_scheduled(&root.id, parser_contract, &task_id, now)
        .map_err(DaemonError::store)?;
    Ok(scheduled > 0)
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
        pdf_import: options.pdf_import,
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
