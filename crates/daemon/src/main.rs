use std::path::{Path, PathBuf};
use std::sync::{atomic::Ordering, Arc};
use std::thread;
use std::time::Duration;

use import_pipeline::PipelineRunControl;

mod bootstrap;
mod command_failure;
mod daemon_error;
mod daemon_policy;
mod delete_command;
mod detail_hydrate;
mod detail_ipc;
mod embedding_runtime;
mod import_command;
mod import_processing;
mod import_watcher;
mod import_worker;
mod ipc;
mod migration_repair;
mod ocr_worker;
mod parent_lifecycle;
mod query_runtime;
mod query_timing;
mod rescan_schedule;
mod run_options;
mod runtime_pack;
mod runtime_probe;
mod search_artifact_worker;
mod search_command;
mod search_contract;
mod search_runtime_config;
mod store_access;
mod worker_ipc;
mod worker_output;
mod worker_runtime;
mod worker_time;

#[cfg(test)]
mod daemon_contract_tests;

use daemon_error::{DaemonError, Result};
use daemon_policy::{
    DEFAULT_IMPORT_RESCAN_MIN_AGE_SECONDS, DEFAULT_OCR_JOBS_PER_TICK, FIELD_CONFIDENCE_THRESHOLD,
    IMPORT_PROGRESS_STREAM_EVENTS, IMPORT_PROGRESS_STREAM_INTERVAL_MS,
    IMPORT_RETRY_BACKOFF_SECONDS, IPC_METADATA_READ_ATTEMPTS, IPC_METADATA_READ_RETRY_MS,
    OCR_LANGUAGE_REMEDIATION, OCR_PAGE_BUDGET_REMEDIATION, SEARCH_RESULT_FILE_NAME_MAX_BYTES,
    STALE_IMPORT_TASK_SECONDS,
};
use import_watcher::ImportWatcher;
use import_worker::{
    recover_stale_import_tasks, run_import_worker_once, run_import_worker_once_with_retry_due,
};
use ocr_worker::{run_ocr_worker_batch, run_ocr_worker_once};
use parent_lifecycle::ParentLifecycleMode;
use run_options::RunOptions;
use search_artifact_worker::run_search_artifact_worker_once;
use store_access::{index_health_label, open_owned_store, open_store};
use worker_output::{
    print_import_worker_summary, print_ocr_worker_summary, print_search_artifact_worker_summary,
    print_startup_summary, search_artifact_recovery_has_activity, ImportWorkerSummary, StartupMode,
};
use worker_runtime::{
    prepare_migration_artifacts_for_worker, run_worker_loop, MigrationArtifactPreparation,
    WorkerLoopRuntime, WorkerSummaryOutput,
};
use worker_time::{current_timestamp, timestamp_minus_seconds};

fn main() {
    if let Err(error) = run() {
        eprintln!("{}", error.fatal_event_json());
        std::process::exit(error.exit_code());
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();

    if args == ["--identity"] {
        println!("resume-daemon");
        return Ok(());
    }

    let data_dir = take_data_dir(&mut args)?;
    if args.first().map(String::as_str) != Some("run") {
        return Err(DaemonError::usage(
            "expected command: resume-daemon run --foreground [--once]",
        ));
    }

    run_command(&data_dir, &args[1..])
}

fn take_data_dir(args: &mut Vec<String>) -> Result<PathBuf> {
    if args.first().map(String::as_str) != Some("--data-dir") {
        return Ok(PathBuf::from("local-data"));
    }

    if args.len() < 2 {
        return Err(DaemonError::usage(
            "usage: resume-daemon --data-dir <path> run --foreground [--once]",
        ));
    }

    let path = PathBuf::from(args.remove(1));
    args.remove(0);
    Ok(path)
}

fn run_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let mut options = run_options::parse(args)?;
    let data_directory_owner = Arc::new(import_processing::acquire_owner(data_dir)?);
    let mut daemon_owner = if options.once {
        None
    } else {
        let owner_mode = match options.parent_lifecycle {
            ParentLifecycleMode::Unmanaged => ipc::OwnerMode::Standalone,
            ParentLifecycleMode::Stdin => ipc::OwnerMode::DesktopSupervised,
        };
        let launch_id = match options.launch_id.clone() {
            Some(launch_id) => launch_id,
            None => ipc::DaemonGenerationOwner::generate_launch_id().map_err(|_| {
                DaemonError::recoverable_dependency(
                    "daemon launch identifier generation unavailable",
                )
            })?,
        };
        Some(
            match ipc::DaemonGenerationOwner::acquire(
                Arc::clone(&data_directory_owner),
                owner_mode,
                launch_id,
            ) {
                Ok(owner) => owner,
                Err(ipc::GenerationError::RuntimeIntegrity) => {
                    return Err(DaemonError::runtime_integrity());
                }
                Err(ipc::GenerationError::Storage) => {
                    return Err(DaemonError::recoverable_dependency(
                        "daemon generation storage unavailable",
                    ));
                }
            },
        )
    };
    let parent_shutdown = parent_lifecycle::start(options.parent_lifecycle)?;
    if !options.once && options.ipc_listen.is_some() {
        return bootstrap::run_persistent_ipc(
            data_dir,
            options,
            data_directory_owner,
            parent_shutdown,
            daemon_owner
                .take()
                .expect("persistent daemon owns its data directory"),
        );
    }
    drop(daemon_owner);

    let startup_control = parent_shutdown
        .as_ref()
        .map(|shutdown| PipelineRunControl::from_shutdown_signal(Arc::clone(shutdown)))
        .unwrap_or_default();
    let (standalone_runtimes, _resident_embedding_owner) =
        bootstrap::resolve_standalone_runtimes(&mut options)?;
    let standalone_capabilities = ipc::CapabilityMatrix::derive(
        ipc::CoreHealth {
            state: ipc::CoreState::Ready,
            reason: None,
        },
        standalone_runtimes,
    );

    let store = open_owned_store(&data_directory_owner)?;
    let processing_contract = import_processing::current_contract(&options)?;
    let mutation_worker_enabled = options.has_worker_loop()
        || options.work_imports_once
        || options.work_ocr_once
        || options.work_index_once;
    let startup_orphaned_recovered = if mutation_worker_enabled {
        let startup_now = current_timestamp()?;
        let recovered = import_processing::normalize_orphaned_running_tasks(&store, startup_now)?;
        import_processing::activate_contract(&store, &processing_contract, startup_now)?;
        recovered
    } else {
        0
    };
    let standalone_startup_artifacts = if !options.once
        && !options.has_worker_loop()
        && options.ipc_listen.is_none()
        && standalone_capabilities.index_publication.state == ipc::CapabilityState::Available
    {
        let preparation = prepare_migration_artifacts_for_worker(&store, &startup_control)?;
        if preparation == MigrationArtifactPreparation::Ready {
            run_search_artifact_worker_once(
                &store,
                &options,
                &processing_contract,
                &startup_control,
            )?;
        }
        Some(preparation)
    } else if !options.once && !options.has_worker_loop() && options.ipc_listen.is_none() {
        Some(MigrationArtifactPreparation::RepairBlocked)
    } else {
        None
    };

    let summary = store.status_summary().map_err(DaemonError::store)?;
    print_startup_summary(
        if options.once {
            StartupMode::Once
        } else {
            StartupMode::Foreground
        },
        &summary,
    )?;

    let migration_artifacts = match standalone_startup_artifacts {
        Some(preparation) => preparation,
        None if options.work_imports_once || options.work_index_once => {
            prepare_migration_artifacts_for_worker(&store, &startup_control)?
        }
        None => MigrationArtifactPreparation::Ready,
    };

    if options.work_imports_once && migration_artifacts == MigrationArtifactPreparation::Ready {
        let mut import_summary =
            run_import_worker_once(data_dir, &store, &options, &processing_contract)?;
        import_summary.orphaned_recovered += startup_orphaned_recovered;
        print_import_worker_summary(&import_summary)?;
    }
    if options.work_ocr_once {
        let ocr_summary = run_ocr_worker_once(data_dir, &store, &options, || true)?;
        if let Some(reason) = ocr_summary.runtime_unavailable {
            return Err(DaemonError::configuration_invalid(format!(
                "ocr runtime became unavailable before claim: {}",
                reason.label()
            )));
        }
        print_ocr_worker_summary(&ocr_summary)?;
    }
    if options.work_index_once && migration_artifacts == MigrationArtifactPreparation::Ready {
        let recovery = run_search_artifact_worker_once(
            &store,
            &options,
            &processing_contract,
            &startup_control,
        )?;
        print_search_artifact_worker_summary(&recovery)?;
    }

    if options.once {
        return Ok(());
    }
    if options.has_worker_loop() {
        run_worker_loop(
            data_dir,
            &store,
            &options,
            &processing_contract,
            WorkerLoopRuntime {
                startup_orphaned_recovered,
                stop_signal: parent_shutdown,
                artifact_fault_receiver: None,
                summary_output: WorkerSummaryOutput::Stdout,
                capability_state: None,
                runtime_health_reporter: None,
            },
        )?;
        return Ok(());
    }

    loop {
        if parent_shutdown
            .as_ref()
            .is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
        {
            return Ok(());
        }
        thread::sleep(if parent_shutdown.is_some() {
            Duration::from_millis(25)
        } else {
            Duration::from_secs(3_600)
        });
    }
}
