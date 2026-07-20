use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use import_pipeline::{
    begin_reported_artifact_repair, prepare_migration_rebuild_artifacts, ImportPipelineErrorClass,
    PipelineRunControl, ReportedArtifactRepairOutcome,
};
use meta_store::{ImportProcessingContract, OwnedMetaStore};

use crate::daemon_error::{DaemonError, WorkerErrorDisposition, WorkerRetryClass};
use crate::ipc::{self, DaemonFatalError};
use crate::rescan_schedule::CompletedRootRescanSchedule;
use crate::{
    current_timestamp, import_processing, migration_repair, print_import_worker_summary,
    print_ocr_worker_summary, print_search_artifact_worker_summary, recover_stale_import_tasks,
    run_import_worker_once_with_retry_due, run_ocr_worker_batch, run_search_artifact_worker_once,
    search_artifact_recovery_has_activity, timestamp_minus_seconds, ImportWatcher,
    ImportWorkerSummary, RunOptions, DEFAULT_IMPORT_RESCAN_MIN_AGE_SECONDS,
    DEFAULT_OCR_JOBS_PER_TICK, IMPORT_RETRY_BACKOFF_SECONDS, STALE_IMPORT_TASK_SECONDS,
};

const RETRY_BACKOFF_MS: [u64; 5] = [250, 1_000, 4_000, 15_000, 30_000];

pub(crate) struct WorkerLoopRuntime {
    pub(crate) startup_orphaned_recovered: usize,
    pub(crate) stop_signal: Option<Arc<AtomicBool>>,
    pub(crate) artifact_fault_receiver: Option<ipc::search_service::ArtifactFaultReceiver>,
    pub(crate) summary_output: WorkerSummaryOutput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerSummaryOutput {
    Stdout,
    /// Persistent IPC stdout is a startup discovery protocol. Background work
    /// is observable through bounded diagnostics, never ad-hoc writes to a
    /// bootstrap pipe whose reader may already have closed.
    Suppressed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkerOutcome {
    Continue,
    StopRequested,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkerFailure {
    Retryable(WorkerRetryClass),
    Fatal(DaemonFatalError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerExit {
    Stopped,
    TickLimitReached,
}

#[derive(Default)]
struct WorkerTickState {
    migration_artifacts: Option<MigrationArtifactPreparation>,
    import_watcher: Option<ImportWatcher>,
    initial_import_tick_pending: bool,
}

struct WorkerTickContext<'a> {
    data_dir: &'a Path,
    store: &'a OwnedMetaStore,
    options: &'a RunOptions,
    processing_contract: &'a ImportProcessingContract,
    runtime: &'a WorkerLoopRuntime,
    pipeline_control: &'a PipelineRunControl,
    completed_root_rescan: Option<CompletedRootRescanSchedule>,
}

impl WorkerTickState {
    fn new() -> Self {
        Self {
            initial_import_tick_pending: true,
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MigrationArtifactPreparation {
    Ready,
    RepairBlocked,
}

pub(crate) fn prepare_migration_artifacts_for_worker(
    store: &OwnedMetaStore,
    control: &PipelineRunControl,
) -> crate::Result<MigrationArtifactPreparation> {
    // A metadata hard cut deliberately invalidates every predecessor derived
    // index. A deterministic retirement failure is persisted as RepairBlocked;
    // do not touch the same invalid publication seam again during startup.
    match prepare_migration_rebuild_artifacts(store, current_timestamp()?, control) {
        Ok(_) => {
            let state = store
                .search_projection_state()
                .map_err(DaemonError::store)?;
            if state.service_state == meta_store::SearchProjectionServiceState::RepairBlocked {
                Ok(MigrationArtifactPreparation::RepairBlocked)
            } else {
                Ok(MigrationArtifactPreparation::Ready)
            }
        }
        Err(error) if error.class() == ImportPipelineErrorClass::ArtifactRetirement => {
            Ok(MigrationArtifactPreparation::RepairBlocked)
        }
        Err(error) => Err(DaemonError::import(error)),
    }
}

pub(crate) fn run_worker_loop(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    processing_contract: &ImportProcessingContract,
    runtime: WorkerLoopRuntime,
) -> Result<WorkerExit, DaemonFatalError> {
    let interval = Duration::from_millis(options.worker_interval_ms.unwrap_or(1_000));
    let completed_root_rescan = if options.rescan_completed_imports {
        Some(
            CompletedRootRescanSchedule::new(
                options
                    .import_rescan_min_age_seconds
                    .unwrap_or(DEFAULT_IMPORT_RESCAN_MIN_AGE_SECONDS),
            )
            .map_err(|_| DaemonFatalError::ConfigurationInvalid)?,
        )
    } else {
        None
    };
    let pipeline_control = runtime
        .stop_signal
        .as_ref()
        .map(|stop| PipelineRunControl::from_shutdown_signal(Arc::clone(stop)))
        .unwrap_or_default();
    let mut state = WorkerTickState::new();
    let tick_context = WorkerTickContext {
        data_dir,
        store,
        options,
        processing_contract,
        runtime: &runtime,
        pipeline_control: &pipeline_control,
        completed_root_rescan,
    };

    drive_resident_worker(
        interval,
        options.max_worker_ticks,
        || stop_requested(runtime.stop_signal.as_ref()),
        |_| {
            classify_tick_result(
                run_worker_tick(&tick_context, &mut state),
                stop_requested(runtime.stop_signal.as_ref()),
            )
        },
        |delay| wait_worker_interval(delay, runtime.stop_signal.as_ref()),
    )
}

fn run_worker_tick(
    context: &WorkerTickContext<'_>,
    state: &mut WorkerTickState,
) -> crate::Result<WorkerOutcome> {
    let WorkerTickContext {
        data_dir,
        store,
        options,
        processing_contract,
        runtime,
        pipeline_control,
        completed_root_rescan,
    } = context;
    if stop_requested(runtime.stop_signal.as_ref()) {
        return Ok(WorkerOutcome::StopRequested);
    }

    let now = current_timestamp()?;
    import_processing::activate_contract(store, processing_contract, now)?;
    if state.migration_artifacts.is_none() {
        state.migration_artifacts = Some(if options.work_imports || options.work_index {
            prepare_migration_artifacts_for_worker(store, pipeline_control)?
        } else {
            MigrationArtifactPreparation::Ready
        });
    }

    run_fault_priority_gate(
        || {
            if options.work_index {
                if let Some(fault) = runtime
                    .artifact_fault_receiver
                    .as_ref()
                    .and_then(ipc::search_service::ArtifactFaultReceiver::try_take)
                {
                    return match begin_reported_artifact_repair(
                        store,
                        fault.generation(),
                        fault.publication_fingerprint(),
                        now,
                    )
                    .map_err(DaemonError::import)?
                    {
                        ReportedArtifactRepairOutcome::Started
                        | ReportedArtifactRepairOutcome::AlreadyRepairing => {
                            let recovery = run_search_artifact_worker_once(
                                store,
                                options,
                                processing_contract,
                                pipeline_control,
                            )?;
                            if runtime.summary_output == WorkerSummaryOutput::Stdout
                                && search_artifact_recovery_has_activity(&recovery)
                            {
                                print_search_artifact_worker_summary(&recovery)?;
                            }
                            Ok(true)
                        }
                        ReportedArtifactRepairOutcome::Superseded => Ok(false),
                    };
                }
            }
            Ok(false)
        },
        |repaired_reported_fault| {
            if state.migration_artifacts == Some(MigrationArtifactPreparation::RepairBlocked) {
                return Ok(());
            }
            if options.work_imports {
                if options.watch_import_roots && state.import_watcher.is_none() {
                    state.import_watcher = Some(ImportWatcher::new()?);
                }
                let initial_import_tick = state.initial_import_tick_pending;
                let mut import_summary = ImportWorkerSummary {
                    orphaned_recovered: usize::from(initial_import_tick)
                        * runtime.startup_orphaned_recovered,
                    stale_recovered: recover_stale_import_tasks(
                        data_dir,
                        store,
                        processing_contract,
                        now,
                        options
                            .stale_import_task_seconds
                            .unwrap_or(STALE_IMPORT_TASK_SECONDS),
                    )?,
                    ..ImportWorkerSummary::default()
                };
                import_summary.repair_requeued = if initial_import_tick {
                    migration_repair::enqueue_authorized_roots(store, processing_contract, now)?
                } else {
                    migration_repair::reconcile_authorized_roots(store, processing_contract, now)?
                };
                if let Some(schedule) = *completed_root_rescan {
                    import_summary.completed_requeued =
                        schedule.requeue_due(store, processing_contract, now)?;
                }
                if let Some(watcher) = state.import_watcher.as_mut() {
                    import_summary.extend_watcher(watcher.sync_and_requeue(
                        store,
                        processing_contract,
                        now,
                    )?);
                }
                import_summary.extend(run_import_worker_once_with_retry_due(
                    data_dir,
                    store,
                    options,
                    processing_contract,
                    timestamp_minus_seconds(
                        now,
                        options
                            .import_retry_backoff_seconds
                            .unwrap_or(IMPORT_RETRY_BACKOFF_SECONDS),
                    ),
                    (*pipeline_control).clone(),
                )?);
                if runtime.summary_output == WorkerSummaryOutput::Stdout
                    && import_summary.has_activity()
                {
                    print_import_worker_summary(&import_summary)?;
                }
                state.initial_import_tick_pending = false;
            }
            if options.work_ocr {
                let ocr_summary = run_ocr_worker_batch(
                    data_dir,
                    store,
                    options,
                    options
                        .ocr_jobs_per_tick
                        .unwrap_or(DEFAULT_OCR_JOBS_PER_TICK),
                )?;
                if runtime.summary_output == WorkerSummaryOutput::Stdout
                    && ocr_summary.has_activity()
                {
                    print_ocr_worker_summary(&ocr_summary)?;
                }
            }
            if options.work_index && !repaired_reported_fault {
                let recovery = run_search_artifact_worker_once(
                    store,
                    options,
                    processing_contract,
                    pipeline_control,
                )?;
                if runtime.summary_output == WorkerSummaryOutput::Stdout
                    && search_artifact_recovery_has_activity(&recovery)
                {
                    print_search_artifact_worker_summary(&recovery)?;
                }
            }
            Ok(())
        },
    )?;

    if stop_requested(runtime.stop_signal.as_ref()) {
        Ok(WorkerOutcome::StopRequested)
    } else {
        Ok(WorkerOutcome::Continue)
    }
}

fn classify_tick_result(
    result: crate::Result<WorkerOutcome>,
    shutdown_requested: bool,
) -> Result<WorkerOutcome, WorkerFailure> {
    match result {
        Ok(outcome) => Ok(outcome),
        Err(error) => match error.worker_disposition() {
            WorkerErrorDisposition::LifecycleCancellation if shutdown_requested => {
                Ok(WorkerOutcome::StopRequested)
            }
            WorkerErrorDisposition::LifecycleCancellation => {
                Err(WorkerFailure::Retryable(WorkerRetryClass::Maintenance))
            }
            WorkerErrorDisposition::Retryable(class) => Err(WorkerFailure::Retryable(class)),
            WorkerErrorDisposition::Fatal(class) => Err(WorkerFailure::Fatal(class.into())),
        },
    }
}

fn drive_resident_worker(
    interval: Duration,
    max_ticks: Option<usize>,
    mut stop_requested: impl FnMut() -> bool,
    mut tick: impl FnMut(usize) -> Result<WorkerOutcome, WorkerFailure>,
    mut wait: impl FnMut(Duration) -> bool,
) -> Result<WorkerExit, DaemonFatalError> {
    let mut ticks = 0_usize;
    let mut consecutive_retryable = 0_usize;

    loop {
        if stop_requested() {
            return Ok(WorkerExit::Stopped);
        }
        ticks += 1;
        let delay = match tick(ticks) {
            Ok(WorkerOutcome::StopRequested) => return Ok(WorkerExit::Stopped),
            Ok(WorkerOutcome::Continue) => {
                consecutive_retryable = 0;
                interval
            }
            Err(WorkerFailure::Retryable(_)) => {
                let index = consecutive_retryable.min(RETRY_BACKOFF_MS.len() - 1);
                consecutive_retryable = consecutive_retryable.saturating_add(1);
                interval.max(Duration::from_millis(RETRY_BACKOFF_MS[index]))
            }
            Err(WorkerFailure::Fatal(fatal)) => return Err(fatal),
        };

        if max_ticks.is_some_and(|max_ticks| ticks >= max_ticks) {
            return Ok(WorkerExit::TickLimitReached);
        }
        if wait(delay) {
            return Ok(WorkerExit::Stopped);
        }
    }
}

/// Enforces the worker-tick priority boundary: an exact reported artifact
/// fault and its synchronous repair complete before import, OCR, or routine
/// index work may enter for the same tick.
pub(crate) fn run_fault_priority_gate<T>(
    reported_artifact_fault: impl FnOnce() -> crate::Result<bool>,
    lower_priority_work: impl FnOnce(bool) -> crate::Result<T>,
) -> crate::Result<T> {
    let repaired_reported_fault = reported_artifact_fault()?;
    lower_priority_work(repaired_reported_fault)
}

fn stop_requested(stop_signal: Option<&Arc<AtomicBool>>) -> bool {
    stop_signal.is_some_and(|stop| stop.load(Ordering::Acquire))
}

fn wait_worker_interval(interval: Duration, stop_signal: Option<&Arc<AtomicBool>>) -> bool {
    let Some(stop_signal) = stop_signal else {
        std::thread::sleep(interval);
        return false;
    };
    let deadline = Instant::now() + interval;
    while Instant::now() < deadline {
        if stop_signal.load(Ordering::Acquire) {
            return true;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(Duration::from_millis(25).min(remaining));
    }
    stop_signal.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    use import_pipeline::ImportPipelineErrorClass;

    use super::{
        classify_tick_result, drive_resident_worker, WorkerExit, WorkerFailure, WorkerOutcome,
        RETRY_BACKOFF_MS,
    };
    use crate::daemon_error::DaemonError;
    use crate::ipc::DaemonFatalError;

    #[test]
    fn retryable_tick_is_backed_off_and_the_next_tick_runs() {
        let attempts = RefCell::new(Vec::new());
        let waits = RefCell::new(Vec::new());

        let outcome = drive_resident_worker(
            Duration::from_millis(1),
            Some(2),
            || false,
            |tick| {
                attempts.borrow_mut().push(tick);
                if tick == 1 {
                    classify_tick_result(
                        Err(DaemonError::test_import_failure(
                            ImportPipelineErrorClass::FullText,
                            true,
                        )),
                        false,
                    )
                } else {
                    Ok(WorkerOutcome::Continue)
                }
            },
            |delay| {
                waits.borrow_mut().push(delay);
                false
            },
        );

        assert_eq!(outcome, Ok(WorkerExit::TickLimitReached));
        assert_eq!(*attempts.borrow(), [1, 2]);
        assert_eq!(*waits.borrow(), [Duration::from_millis(250)]);
    }

    #[test]
    fn retryable_backoff_is_bounded_and_never_busy_loops() {
        let waits = RefCell::new(Vec::new());
        let outcome = drive_resident_worker(
            Duration::from_millis(1),
            Some(8),
            || false,
            |_| {
                Err(WorkerFailure::Retryable(
                    crate::daemon_error::WorkerRetryClass::Storage,
                ))
            },
            |delay| {
                waits.borrow_mut().push(delay);
                false
            },
        );

        assert_eq!(outcome, Ok(WorkerExit::TickLimitReached));
        assert_eq!(
            *waits.borrow(),
            RETRY_BACKOFF_MS
                .into_iter()
                .chain([30_000, 30_000])
                .map(Duration::from_millis)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn lifecycle_cancellation_during_shutdown_is_a_clean_stop() {
        let shutdown = Cell::new(false);
        let outcome = drive_resident_worker(
            Duration::from_millis(1),
            None,
            || shutdown.get(),
            |_| {
                shutdown.set(true);
                classify_tick_result(
                    Err(DaemonError::test_import_failure(
                        ImportPipelineErrorClass::Cancelled,
                        true,
                    )),
                    shutdown.get(),
                )
            },
            |_| panic!("clean lifecycle cancellation must not enter backoff"),
        );

        assert_eq!(outcome, Ok(WorkerExit::Stopped));
    }

    #[test]
    fn metadata_invariant_is_process_fatal_runtime_integrity() {
        let failure = classify_tick_result(
            Err(DaemonError::test_import_failure(
                ImportPipelineErrorClass::MetadataInvariant,
                false,
            )),
            false,
        );

        assert_eq!(
            failure,
            Err(WorkerFailure::Fatal(DaemonFatalError::RuntimeIntegrity))
        );
    }
}
