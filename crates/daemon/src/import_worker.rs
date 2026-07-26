use std::path::Path;
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use import_pipeline::{
    import_root_with_options_and_control, ImportOptions, ImportPipelineErrorClass,
    ImportTaskOwnerLock, PipelineRunControl, ScanProfile,
};
use meta_store::{
    ImportProcessingContract, ImportScanBudgetKind, ImportScanProfile, ImportScanScope,
    ImportTaskFailure, ImportTaskId, ImportTaskStatus, OwnedMetaStore, ScanCounts, ScanPhase,
    ScanTrigger, SearchRepairReason, UnixTimestamp,
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
                finish_source_scan_failure(
                    store,
                    &scope,
                    &task.id,
                    processing_contract,
                    current_timestamp()?,
                )?;
                continue;
            }
        };

        finish_source_scan_success(
            store,
            &scope,
            &task.id,
            processing_contract,
            &import_summary,
        )?;
        worker_summary.processed += 1;
        worker_summary.searchable_documents += import_summary.searchable_documents;
        worker_summary.ocr_jobs_queued += import_summary.ocr_jobs_queued;
    }

    if claim_allowed() {
        let _ = try_finalize_migration_rebuild(store, options, processing_contract, &run_control)?;
    }
    Ok(worker_summary)
}

fn finish_source_scan_success(
    store: &OwnedMetaStore,
    scope: &ImportScanScope,
    task_id: &ImportTaskId,
    processing_contract: &ImportProcessingContract,
    summary: &import_pipeline::ImportSummary,
) -> Result<()> {
    let Some(root) = store
        .source_root_by_canonical_path(&scope.canonical_root_path)
        .map_err(DaemonError::store)?
    else {
        return Ok(());
    };
    let Some(snapshot) = store
        .latest_scan_snapshot(&root.id)
        .map_err(DaemonError::store)?
        .filter(|snapshot| snapshot.id == task_id.as_str() && snapshot.phase.is_active())
    else {
        return Ok(());
    };
    let finished_at = current_timestamp()?;
    let elapsed = finished_at
        .as_unix_seconds()
        .saturating_sub(snapshot.started_at.as_unix_seconds());
    let processed = summary.processed_documents;
    let rate = (elapsed > 0 && processed > 0).then_some(processed as f64 / elapsed as f64);
    let classifications = store
        .source_root_classification_counts(&root.id, processing_contract.classifier_epoch())
        .map_err(DaemonError::store)?;
    let counts = ScanCounts {
        discovered: summary.files_discovered as u64,
        searchable: summary.searchable_documents as u64,
        non_resume: classifications.non_resume,
        needs_review: classifications.needs_review,
        ocr: summary.ocr_required_documents as u64,
        failed: summary.failed_documents as u64,
        ignored: summary.ignored_entries as u64,
        processed: processed as u64,
        total: Some(summary.files_discovered as u64),
        errors: summary.scan_errors as u64,
    };
    // Per-file parse/classification failures are part of a complete source
    // snapshot. Only an incomplete directory enumeration or an exhausted scan
    // budget makes absence unsafe to interpret as deletion.
    let scan_is_complete = summary.source_truth_complete
        && summary.scan_errors == 0
        && !summary.scan_budget.is_some_and(|budget| budget.exhausted);
    if scan_is_complete {
        store
            .reconcile_complete_source_scan(&root.id, task_id.as_str(), counts, rate, finished_at)
            .map_err(DaemonError::store)?;
        if summary.deferred_pdf_documents == 0 {
            store
                .complete_pdf_reprocess_root(
                    &root.id,
                    task_id,
                    processing_contract.primary_parse_version(),
                    finished_at,
                )
                .map_err(DaemonError::store)?;
        } else {
            store
                .requeue_pdf_reprocess_root(
                    &root.id,
                    task_id,
                    processing_contract.primary_parse_version(),
                    finished_at,
                )
                .map_err(DaemonError::store)?;
        }
    } else {
        store
            .fail_or_partial_scan(
                &root.id,
                task_id.as_str(),
                counts,
                ScanPhase::Partial,
                finished_at,
            )
            .map_err(DaemonError::store)?;
        store
            .requeue_pdf_reprocess_root(
                &root.id,
                task_id,
                processing_contract.primary_parse_version(),
                finished_at,
            )
            .map_err(DaemonError::store)?;
    }
    Ok(())
}

fn finish_source_scan_failure(
    store: &OwnedMetaStore,
    scope: &ImportScanScope,
    task_id: &ImportTaskId,
    processing_contract: &ImportProcessingContract,
    now: UnixTimestamp,
) -> Result<()> {
    let Some(root) = store
        .source_root_by_canonical_path(&scope.canonical_root_path)
        .map_err(DaemonError::store)?
    else {
        return Ok(());
    };
    let Some(snapshot) = store
        .latest_scan_snapshot(&root.id)
        .map_err(DaemonError::store)?
        .filter(|snapshot| snapshot.id == task_id.as_str() && snapshot.phase.is_active())
    else {
        return Ok(());
    };
    store
        .fail_or_partial_scan(
            &root.id,
            task_id.as_str(),
            snapshot.counts,
            ScanPhase::Failed,
            now,
        )
        .map_err(DaemonError::store)?;
    store
        .requeue_pdf_reprocess_root(
            &root.id,
            task_id,
            processing_contract.primary_parse_version(),
            now,
        )
        .map_err(DaemonError::store)?;
    Ok(())
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
