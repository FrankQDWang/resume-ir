use crate::daemon_error::{DaemonError, Result};
use crate::ipc;
use crate::parent_lifecycle::ParentLifecycleMode;

use super::{usage, RunOptions};

pub(super) fn validate(options: &RunOptions) -> Result<()> {
    if options
        .expected_ipc_protocol
        .as_deref()
        .is_some_and(|protocol| protocol != ipc::IPC_PROTOCOL_VERSION)
    {
        return Err(DaemonError::protocol_mismatch());
    }
    if options.parent_lifecycle == ParentLifecycleMode::Stdin && options.launch_id.is_none() {
        return Err(DaemonError::usage(
            "usage: --parent-lifecycle-stdin requires --launch-id",
        ));
    }
    if options.once && options.launch_id.is_some() {
        return Err(DaemonError::usage(
            "usage: --launch-id cannot be combined with --once",
        ));
    }
    if !options.foreground {
        return Err(DaemonError::usage(usage()));
    }
    if options.once && options.ipc_listen.is_some() {
        return Err(DaemonError::usage(
            "usage: --once cannot be combined with --ipc-listen",
        ));
    }
    if options.work_imports_once && !options.once {
        return Err(DaemonError::usage(
            "usage: --work-imports-once requires --once",
        ));
    }
    if options.work_imports && options.once {
        return Err(DaemonError::usage(
            "usage: --work-imports cannot be combined with --once",
        ));
    }
    if options.work_imports && options.work_imports_once {
        return Err(DaemonError::usage(
            "usage: choose either --work-imports or --work-imports-once",
        ));
    }
    if options.work_ocr_once && !options.once {
        return Err(DaemonError::usage("usage: --work-ocr-once requires --once"));
    }
    if options.work_ocr && options.once {
        return Err(DaemonError::usage(
            "usage: --work-ocr cannot be combined with --once",
        ));
    }
    if options.work_ocr && options.work_ocr_once {
        return Err(DaemonError::usage(
            "usage: choose either --work-ocr or --work-ocr-once",
        ));
    }
    if options.work_index_once && !options.once {
        return Err(DaemonError::usage(
            "usage: --work-index-once requires --once",
        ));
    }
    if options.work_index && options.once {
        return Err(DaemonError::usage(
            "usage: --work-index cannot be combined with --once",
        ));
    }
    if options.work_index && options.work_index_once {
        return Err(DaemonError::usage(
            "usage: choose either --work-index or --work-index-once",
        ));
    }
    if (options.worker_interval_ms.is_some()
        || options.max_worker_ticks.is_some()
        || options.ocr_jobs_per_tick.is_some())
        && !options.has_worker_loop()
    {
        return Err(DaemonError::usage(
            "usage: worker loop options require --work-imports, --work-ocr, or --work-index",
        ));
    }
    if options.ocr_jobs_per_tick.is_some() && !options.work_ocr {
        return Err(DaemonError::usage(
            "usage: --ocr-jobs-per-tick requires --work-ocr",
        ));
    }
    if options.import_rescan_min_age_seconds.is_some() && !options.rescan_completed_imports {
        return Err(DaemonError::usage(
            "usage: --import-rescan-min-age-seconds requires --rescan-completed-imports",
        ));
    }
    if options.stale_import_task_seconds.is_some() && !options.work_imports {
        return Err(DaemonError::usage(
            "usage: --stale-import-task-seconds requires --work-imports",
        ));
    }
    if options.import_retry_backoff_seconds.is_some() && !options.work_imports {
        return Err(DaemonError::usage(
            "usage: --import-retry-backoff-seconds requires --work-imports",
        ));
    }
    if options.rescan_completed_imports && !options.work_imports {
        return Err(DaemonError::usage(
            "usage: import rescan options require --work-imports",
        ));
    }
    if options.watch_import_roots && !options.work_imports {
        return Err(DaemonError::usage(
            "usage: import watcher options require --work-imports",
        ));
    }
    if options.has_worker_loop()
        && options.ipc_listen.is_some()
        && options.max_worker_ticks.is_some()
    {
        return Err(DaemonError::usage(
            "usage: --max-worker-ticks cannot be combined with --ipc-listen",
        ));
    }
    if options.work_ocr_once
        && options.ocr_command.is_none()
        && options.ocr_tesseract_command.is_none()
    {
        return Err(DaemonError::configuration_invalid(
            "ocr worker blocked: local OCR command not configured",
        ));
    }
    Ok(())
}
