use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
    Arc,
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use embedder::{
    EmbeddingBudget, EmbeddingError, EmbeddingInput, EmbeddingPriority, LocalEmbeddingCommandSpec,
    ResidentEmbeddingClient, ResidentEmbeddingOwner, ResidentEmbeddingSpec,
};
use import_pipeline::{
    detect_ocr_page_count, finalize_migration_rebuild, import_root_with_options_and_control,
    index_claimed_ocr_text_with_policy, ocr_preclaim_decision, reconcile_search_artifacts,
    ImportOptions, ImportPipelineErrorClass, ImportResourcePolicy, ImportTaskOwnerLock,
    LinearPromotionPolicy, OcrPreclaimDecision, PipelineRunControl, ScanProfile,
    SearchArtifactRecoverySummary, SearchPublicationEmbeddingFailure,
    SearchPublicationEmbeddingInput, SearchPublicationEmbeddingOutput,
    SearchPublicationVectorization, SearchPublicationVectorizer,
};
use meta_store::{
    ImportProcessingContract, ImportScanBudgetKind, ImportScanProfile, ImportScanScope,
    ImportTaskFailure, ImportTaskId, ImportTaskStatus, IndexStateStatus, IngestJobFailureKind,
    OcrAttemptFailure, OcrPageCacheEntry, OcrPageCacheKey, OcrPageCacheStatus, OwnedMetaStore,
    ReadMetaStore, SearchProjectionServiceState, SearchRepairReason, UnixTimestamp, WorkerTaskKind,
};
use notify::{
    event::EventKind as NotifyEventKind, Config as NotifyConfig, Event as NotifyEvent,
    RecommendedWatcher, RecursiveMode, Watcher,
};
use ocr_client::{
    inspect_tesseract_language_availability, CancellationToken, LocalOcrCommandClient,
    LocalOcrCommandSpec, LocalPdfRenderCommandClient, LocalPdfRenderCommandSpec, OcrClient,
    OcrOptions, OcrPageRequest, OcrWorkerBudget, PdftoppmPdfRenderer, PdftoppmRenderSpec,
    RenderedPage, TesseractLanguageAvailability, TesseractOcrClient, TesseractOcrSpec,
};
#[cfg(unix)]
use process_containment::CurrentProcessGroupLeader;

mod command_failure;
mod daemon_error;
mod delete_command;
mod detail_hydrate;
mod detail_ipc;
mod import_command;
mod import_processing;
mod ipc;
mod migration_repair;
mod query_runtime;
mod query_timing;
mod rescan_schedule;
mod search_command;
mod search_contract;
mod search_runtime_config;
mod worker_runtime;

use daemon_error::{DaemonError, Result};
use worker_runtime::{
    prepare_migration_artifacts_for_worker, run_worker_loop, MigrationArtifactPreparation,
    WorkerLoopRuntime, WorkerSummaryOutput,
};

const IMPORT_RETRY_BACKOFF_SECONDS: i64 = 60;
const DEFAULT_IMPORT_RESCAN_MIN_AGE_SECONDS: i64 = 300;
const IMPORT_TASK_HEARTBEAT_SECONDS: u64 = 30;
const STALE_IMPORT_TASK_SECONDS: i64 = 15 * 60;
const STALE_INGEST_JOB_SECONDS: i64 = 15 * 60;
const SEARCH_RESULT_FILE_NAME_MAX_BYTES: usize = 160;
const IPC_METADATA_READ_ATTEMPTS: usize = 40;
const IPC_METADATA_READ_RETRY_MS: u64 = 25;
const IMPORT_PROGRESS_STREAM_EVENTS: usize = 3;
const IMPORT_PROGRESS_STREAM_INTERVAL_MS: u64 = 25;
const DEFAULT_OCR_ENGINE_PROFILE: &str = "local-command";
const DEFAULT_OCR_LANG: &str = "eng";
const DEFAULT_OCR_PROFILE: &str = "balanced";
const DEFAULT_OCR_RENDER_DPI: u32 = 300;
const DEFAULT_OCR_PAGE_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_OCR_MAX_PAGES_PER_DOCUMENT: u32 = 100;
const DEFAULT_OCR_JOBS_PER_TICK: usize = 1;
const OCR_PAGE_BUDGET_REMEDIATION: &str =
    "raise OCR max pages per document or skip oversized scanned PDFs";
const OCR_LANGUAGE_REMEDIATION: &str =
    "install requested OCR language packs or choose an installed OCR language";
const DEFAULT_EMBEDDING_TIMEOUT_MS: u64 = 30_000;
const FIELD_CONFIDENCE_THRESHOLD: f32 = 0.75;
#[cfg(unix)]
const PARENT_LIFECYCLE_SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

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
    let mut options = parse_run_options(args)?;

    if options
        .expected_ipc_protocol
        .as_deref()
        .is_some_and(|protocol| protocol != ipc::IPC_PROTOCOL_VERSION)
    {
        return Err(DaemonError::protocol_mismatch());
    }

    if !options.foreground {
        return Err(DaemonError::usage(run_usage()));
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
    if (options.work_ocr_once || options.work_ocr)
        && options.ocr_command.is_none()
        && options.ocr_tesseract_command.is_none()
    {
        return Err(DaemonError::configuration_invalid(
            "ocr worker blocked: local OCR command not configured",
        ));
    }
    let data_directory_owner = import_processing::acquire_owner(data_dir)?;
    let mut daemon_owner = if options.once {
        None
    } else {
        let owner_mode = match options.parent_lifecycle {
            ParentLifecycleMode::Unmanaged => ipc::OwnerMode::Standalone,
            ParentLifecycleMode::Stdin => ipc::OwnerMode::DesktopSupervised,
        };
        Some(
            match ipc::DaemonGenerationOwner::acquire(&data_directory_owner, owner_mode) {
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
    let parent_shutdown = start_parent_lifecycle_watch(options.parent_lifecycle)?;
    let startup_control = parent_shutdown
        .as_ref()
        .map(|shutdown| PipelineRunControl::from_shutdown_signal(Arc::clone(shutdown)))
        .unwrap_or_default();
    let _resident_embedding_owner = start_resident_embedding(&mut options)?;

    let store = open_owned_store(&data_directory_owner)?;
    let processing_contract = import_processing::current_contract(&options)?;
    let startup_now = current_timestamp()?;
    let startup_orphaned_recovered =
        import_processing::normalize_orphaned_running_tasks(&store, startup_now)?;
    import_processing::activate_contract(&store, &processing_contract, startup_now)?;
    let standalone_startup_artifacts =
        if !options.once && !options.has_worker_loop() && options.ipc_listen.is_none() {
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
        } else {
            None
        };
    let summary = store.status_summary().map_err(DaemonError::store)?;

    println!("resume-daemon foreground ready");
    println!("mode: {}", if options.once { "once" } else { "foreground" });
    println!("index health: {}", index_health_label(summary.index_health));
    println!("import tasks queued: {}", summary.import_tasks_queued);
    println!("import tasks cancelled: {}", summary.import_tasks_cancelled);
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::control_plane("unable to write daemon status"))?;

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
        let ocr_summary = run_ocr_worker_once(data_dir, &store, &options)?;
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
    if options.has_worker_loop() && options.ipc_listen.is_some() {
        run_worker_with_ipc(
            data_dir,
            &store,
            &options,
            &processing_contract,
            startup_orphaned_recovered,
            parent_shutdown.as_ref(),
            daemon_owner
                .take()
                .expect("persistent daemon owns its data directory"),
        )?;
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
            },
        )?;
        return Ok(());
    }
    if let Some(ipc_addr) = options.ipc_listen {
        let ipc_store = open_store(data_dir)?;
        let ipc_owned_store = store.open_sibling().map_err(DaemonError::store)?;
        ipc::server::BoundServer::bind(
            ipc_addr,
            daemon_owner
                .take()
                .expect("persistent daemon owns its data directory"),
        )?
        .serve(ipc::server::Context {
            data_dir,
            store: &ipc_store,
            owned_store: &ipc_owned_store,
            max_requests: options.max_requests,
            search_runtime_config: search_runtime_config(&options),
            processing_contract: &processing_contract,
            shutdown: parent_shutdown.as_ref(),
            worker_result_receiver: None,
            artifact_fault_reporter: None,
        })
        .map_err(DaemonError::from)?;
        if options.max_requests.is_some() || parent_shutdown.is_some() {
            return Ok(());
        }
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

fn start_parent_lifecycle_watch(mode: ParentLifecycleMode) -> Result<Option<Arc<AtomicBool>>> {
    if mode != ParentLifecycleMode::Stdin {
        return Ok(None);
    }
    #[cfg(unix)]
    let process_group_leader = CurrentProcessGroupLeader::acquire().map_err(|_| {
        DaemonError::user("parent lifecycle stdin requires an isolated process group")
    })?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let watcher_shutdown = Arc::clone(&shutdown);
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buffer = [0_u8; 1];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => {
                    watcher_shutdown.store(true, Ordering::Release);
                    #[cfg(unix)]
                    {
                        thread::sleep(PARENT_LIFECYCLE_SHUTDOWN_GRACE);
                        let _ = process_group_leader.kill_process_group();
                    }
                    return;
                }
                Ok(_) => {}
            }
        }
    });
    Ok(Some(shutdown))
}

fn start_resident_embedding(options: &mut RunOptions) -> Result<Option<ResidentEmbeddingOwner>> {
    if options.embedding_command.is_none() {
        return Ok(None);
    }
    let command = options
        .embedding_command
        .clone()
        .ok_or_else(|| DaemonError::usage(run_usage()))?;
    let model_id = options
        .embedding_model_id
        .as_deref()
        .ok_or_else(|| DaemonError::usage(run_usage()))?;
    let dimension = options
        .embedding_dimension
        .ok_or_else(|| DaemonError::usage(run_usage()))?;
    let command =
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), model_id, dimension)
            .map_err(DaemonError::embedding)?
            .with_timeout_ms(options.embedding_timeout_ms)
            .map_err(DaemonError::embedding)?;
    let inference_threads = ImportResourcePolicy::detect().parse_workers.get();
    let owner = ResidentEmbeddingOwner::start(
        ResidentEmbeddingSpec::new(command)
            .with_intra_threads(inference_threads)
            .map_err(DaemonError::embedding)?,
    )
    .map_err(DaemonError::embedding)?;
    let client = owner.client();
    options.search_vectorization =
        SearchPublicationVectorization::enabled(Arc::new(ResidentPublicationVectorizer {
            client: client.clone(),
            timeout_ms: options.embedding_timeout_ms,
        }));
    options.resident_embedding = Some(client);
    Ok(Some(owner))
}

struct ResidentPublicationVectorizer {
    client: ResidentEmbeddingClient,
    timeout_ms: u64,
}

impl SearchPublicationVectorizer for ResidentPublicationVectorizer {
    fn model_id(&self) -> &str {
        self.client.model_id()
    }

    fn dimension(&self) -> usize {
        self.client.dimension()
    }

    fn max_batch_inputs(&self) -> usize {
        embedding_protocol::MAX_INPUTS
    }

    fn max_text_bytes(&self) -> usize {
        embedding_protocol::MAX_TEXT_BYTES
    }

    fn embed_batch(
        &self,
        inputs: &[SearchPublicationEmbeddingInput],
        is_cancelled: &dyn Fn() -> bool,
    ) -> std::result::Result<Vec<SearchPublicationEmbeddingOutput>, SearchPublicationEmbeddingFailure>
    {
        let resident_inputs = inputs
            .iter()
            .map(|input| EmbeddingInput::new(input.id(), input.text()))
            .collect::<Vec<_>>();
        self.client
            .embed_batch_with_cancel(
                EmbeddingPriority::Background,
                &resident_inputs,
                EmbeddingBudget::new(resident_inputs.len(), embedding_protocol::MAX_TEXT_BYTES),
                self.timeout_ms,
                is_cancelled,
            )
            .map(|outputs| {
                outputs
                    .into_iter()
                    .map(|output| {
                        SearchPublicationEmbeddingOutput::new(
                            output.id(),
                            output.model_id(),
                            output.values().to_vec(),
                        )
                    })
                    .collect()
            })
            .map_err(|error| match error {
                EmbeddingError::Cancelled => SearchPublicationEmbeddingFailure::Cancelled,
                EmbeddingError::InvalidDimension
                | EmbeddingError::InvalidRequest
                | EmbeddingError::BudgetExceeded { .. }
                | EmbeddingError::TextBudgetExceeded { .. } => {
                    SearchPublicationEmbeddingFailure::InvalidOutput
                }
                EmbeddingError::WorkerUnavailable
                | EmbeddingError::EngineFailed
                | EmbeddingError::Overloaded
                | EmbeddingError::Timeout => SearchPublicationEmbeddingFailure::RuntimeUnavailable,
            })
    }
}

fn parse_run_options(args: &[String]) -> Result<RunOptions> {
    let mut options = RunOptions::default();
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--foreground" => {
                options.foreground = true;
                index += 1;
            }
            "--parent-lifecycle-stdin" => {
                if options.parent_lifecycle != ParentLifecycleMode::Unmanaged {
                    return Err(DaemonError::usage(run_usage()));
                }
                options.parent_lifecycle = ParentLifecycleMode::Stdin;
                index += 1;
            }
            "--once" => {
                options.once = true;
                index += 1;
            }
            "--ipc-listen" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.ipc_listen = Some(parse_loopback_addr(value)?);
                index += 2;
            }
            "--expected-ipc-protocol" => {
                if options.expected_ipc_protocol.is_some() {
                    return Err(DaemonError::usage(run_usage()));
                }
                options.expected_ipc_protocol =
                    Some(parse_non_empty_run_value(args.get(index + 1))?);
                index += 2;
            }
            "--max-requests" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.max_requests = Some(
                    value
                        .parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| DaemonError::usage(run_usage()))?,
                );
                index += 2;
            }
            "--work-imports-once" => {
                options.work_imports_once = true;
                index += 1;
            }
            "--work-imports" => {
                options.work_imports = true;
                index += 1;
            }
            "--rescan-completed-imports" => {
                options.rescan_completed_imports = true;
                index += 1;
            }
            "--watch-import-roots" => {
                options.watch_import_roots = true;
                index += 1;
            }
            "--import-rescan-min-age-seconds" => {
                options.import_rescan_min_age_seconds =
                    Some(parse_nonnegative_i64_run_value(args.get(index + 1))?);
                index += 2;
            }
            "--stale-import-task-seconds" => {
                options.stale_import_task_seconds =
                    Some(parse_nonnegative_i64_run_value(args.get(index + 1))?);
                index += 2;
            }
            "--import-retry-backoff-seconds" => {
                options.import_retry_backoff_seconds =
                    Some(parse_nonnegative_i64_run_value(args.get(index + 1))?);
                index += 2;
            }
            "--resume-classifier-model" => {
                if options.classifier_model_configured {
                    return Err(DaemonError::usage(run_usage()));
                }
                let path = PathBuf::from(parse_non_empty_run_value(args.get(index + 1))?);
                if !path.is_absolute() {
                    return Err(DaemonError::usage(run_usage()));
                }
                let policy = LinearPromotionPolicy::load_bundled(&path);
                if !policy.enabled() {
                    return Err(DaemonError::user(
                        "resume classifier model is invalid or incompatible",
                    ));
                }
                options.linear_promotion = policy;
                options.classifier_model_configured = true;
                index += 2;
            }
            "--work-ocr-once" => {
                options.work_ocr_once = true;
                index += 1;
            }
            "--work-ocr" => {
                options.work_ocr = true;
                index += 1;
            }
            "--work-index-once" => {
                options.work_index_once = true;
                index += 1;
            }
            "--work-index" => {
                options.work_index = true;
                index += 1;
            }
            "--ocr-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                if options.ocr_command.is_some() {
                    return Err(DaemonError::usage(run_usage()));
                }
                options.ocr_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-tesseract-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                if options.ocr_tesseract_command.is_some() {
                    return Err(DaemonError::usage(run_usage()));
                }
                options.ocr_tesseract_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-render-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                if options.ocr_render_command.is_some() {
                    return Err(DaemonError::usage(run_usage()));
                }
                options.ocr_render_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-pdftoppm-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                if options.ocr_pdftoppm_command.is_some() {
                    return Err(DaemonError::usage(run_usage()));
                }
                options.ocr_pdftoppm_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-engine-profile" => {
                options.ocr_engine_profile = parse_non_empty_run_value(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-lang" => {
                options.ocr_lang = parse_non_empty_run_value(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-profile" => {
                options.ocr_profile = parse_non_empty_run_value(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-render-dpi" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.ocr_render_dpi = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| DaemonError::usage(run_usage()))?;
                index += 2;
            }
            "--ocr-page-timeout-ms" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.ocr_page_timeout_ms = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| DaemonError::usage(run_usage()))?;
                index += 2;
            }
            "--ocr-max-pages-per-document" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.ocr_max_pages_per_document = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| DaemonError::usage(run_usage()))?;
                index += 2;
            }
            "--ocr-jobs-per-tick" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.ocr_jobs_per_tick = Some(parse_positive_usize_run_value(value)?);
                index += 2;
            }
            "--embedding-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                if options.embedding_command.is_some() {
                    return Err(DaemonError::usage(run_usage()));
                }
                options.embedding_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--embedding-model-id" => {
                let value = parse_non_empty_run_value(args.get(index + 1))?;
                if !valid_run_identifier(&value) {
                    return Err(DaemonError::usage(run_usage()));
                }
                options.embedding_model_id = Some(value);
                index += 2;
            }
            "--embedding-dimension" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.embedding_dimension = Some(parse_positive_usize_run_value(value)?);
                index += 2;
            }
            "--embedding-timeout-ms" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.embedding_timeout_ms = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| DaemonError::usage(run_usage()))?;
                index += 2;
            }
            "--worker-interval-ms" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.worker_interval_ms = Some(
                    value
                        .parse::<u64>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| DaemonError::usage(run_usage()))?,
                );
                index += 2;
            }
            "--max-worker-ticks" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.max_worker_ticks = Some(
                    value
                        .parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| DaemonError::usage(run_usage()))?,
                );
                index += 2;
            }
            _ => return Err(DaemonError::usage(run_usage())),
        }
    }

    if options.max_requests.is_some() && options.ipc_listen.is_none() {
        return Err(DaemonError::usage(
            "usage: --max-requests requires --ipc-listen",
        ));
    }
    if options.ocr_command.is_some() && options.ocr_tesseract_command.is_some() {
        return Err(DaemonError::usage(run_usage()));
    }
    if options.ocr_render_command.is_some() && options.ocr_pdftoppm_command.is_some() {
        return Err(DaemonError::usage(run_usage()));
    }

    Ok(options)
}

fn run_usage() -> &'static str {
    "usage: resume-daemon run --foreground [--parent-lifecycle-stdin] [--once] [--work-imports-once|--work-imports [--rescan-completed-imports] [--watch-import-roots] [--import-rescan-min-age-seconds <n>] [--stale-import-task-seconds <n>] [--import-retry-backoff-seconds <n>]] [--resume-classifier-model <absolute-path>] [--work-ocr-once|--work-ocr] [--work-index-once|--work-index] [--ocr-command <path>|--ocr-tesseract-command <path>] [--ocr-render-command <path>|--ocr-pdftoppm-command <path>] [--ocr-engine-profile <name>] [--ocr-lang <lang>] [--ocr-profile <profile>] [--ocr-render-dpi <dpi>] [--ocr-page-timeout-ms <ms>] [--ocr-max-pages-per-document <n>] [--ocr-jobs-per-tick <n>] [--embedding-command <path>] [--embedding-model-id <id>] [--embedding-dimension <n>] [--embedding-timeout-ms <ms>] [--worker-interval-ms <n>] [--max-worker-ticks <n>] [--ipc-listen <127.0.0.1:port>] [--expected-ipc-protocol <version>] [--max-requests <n>]"
}

fn parse_non_empty_run_value(value: Option<&String>) -> Result<String> {
    let Some(value) = value else {
        return Err(DaemonError::usage(run_usage()));
    };
    if value.trim().is_empty() {
        return Err(DaemonError::usage(run_usage()));
    }
    Ok(value.clone())
}

fn parse_positive_usize_run_value(value: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| DaemonError::usage(run_usage()))
}

fn parse_nonnegative_i64_run_value(value: Option<&String>) -> Result<i64> {
    value
        .ok_or_else(|| DaemonError::usage(run_usage()))?
        .parse::<i64>()
        .ok()
        .filter(|value| *value >= 0)
        .ok_or_else(|| DaemonError::usage(run_usage()))
}

fn valid_run_identifier(value: &str) -> bool {
    !value.trim().is_empty()
        && !value.contains('\n')
        && !value.contains('\r')
        && !value.contains('\t')
}

fn run_worker_with_ipc(
    data_dir: &Path,
    owned_store: &OwnedMetaStore,
    options: &RunOptions,
    processing_contract: &ImportProcessingContract,
    startup_orphaned_recovered: usize,
    parent_shutdown: Option<&Arc<AtomicBool>>,
    daemon_owner: ipc::DaemonGenerationOwner<'_>,
) -> Result<()> {
    let ipc_addr = options
        .ipc_listen
        .expect("validated combined worker/ipc mode has ipc address");
    let bound_server = ipc::server::BoundServer::bind(ipc_addr, daemon_owner)?;
    let ipc_store = open_store(data_dir)?;
    let ipc_owned_store = owned_store.open_sibling().map_err(DaemonError::store)?;
    let worker_store = owned_store.open_sibling().map_err(DaemonError::store)?;
    let stop_worker = parent_shutdown
        .cloned()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let worker_stop = Arc::clone(&stop_worker);
    let worker_data_dir = data_dir.to_path_buf();
    let worker_options = options.clone();
    let worker_processing_contract = processing_contract.clone();
    let (artifact_fault_reporter, artifact_fault_receiver) = if options.work_index {
        let (reporter, receiver) = ipc::search_service::artifact_fault_latch();
        (Some(reporter), Some(receiver))
    } else {
        (None, None)
    };
    let (worker_result_sender, worker_result_receiver) =
        mpsc::channel::<std::result::Result<(), ipc::DaemonFatalError>>();
    let worker_handle = thread::spawn(move || {
        let result = run_worker_loop(
            &worker_data_dir,
            &worker_store,
            &worker_options,
            &worker_processing_contract,
            WorkerLoopRuntime {
                startup_orphaned_recovered,
                stop_signal: Some(worker_stop),
                artifact_fault_receiver,
                summary_output: WorkerSummaryOutput::Suppressed,
            },
        );
        let _ = worker_result_sender.send(result.map(|_| ()));
    });

    let ipc_result = bound_server.serve(ipc::server::Context {
        data_dir,
        store: &ipc_store,
        owned_store: &ipc_owned_store,
        max_requests: options.max_requests,
        search_runtime_config: search_runtime_config(options),
        processing_contract,
        shutdown: Some(&stop_worker),
        worker_result_receiver: Some(&worker_result_receiver),
        artifact_fault_reporter,
    });
    stop_worker.store(true, Ordering::Release);
    if let Err(fatal) = ipc_result {
        abort_worker_for_process_exit(worker_handle);
        return Err(DaemonError::from(fatal));
    }
    worker_handle
        .join()
        .map_err(|_| DaemonError::control_plane("worker thread panicked"))?;
    Ok(())
}

fn abort_worker_for_process_exit(worker_handle: thread::JoinHandle<()>) {
    // This is the explicit control-plane-fatal path. The stop signal has
    // already been raised; dropping the handle avoids waiting on a data-plane
    // call that cannot cooperate, and the process supervisor containment
    // deadline owns final tree termination.
    drop(worker_handle);
}

fn run_import_worker_once(
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
    )?);
    Ok(summary)
}

fn run_search_artifact_worker_once(
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

fn try_finalize_migration_rebuild(
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

fn run_import_worker_once_with_retry_due(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    processing_contract: &ImportProcessingContract,
    retryable_due_at: UnixTimestamp,
    run_control: PipelineRunControl,
) -> Result<ImportWorkerSummary> {
    let mut worker_summary = ImportWorkerSummary::default();
    let mut attempted = Vec::<ImportTaskId>::new();

    while !run_control.shutdown_requested() {
        if search_repair_is_blocked(store)? {
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

    let _ = try_finalize_migration_rebuild(store, options, processing_contract, &run_control)?;
    Ok(worker_summary)
}

fn should_requeue_interrupted_import(
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

fn search_repair_is_blocked(store: &OwnedMetaStore) -> Result<bool> {
    Ok(store
        .search_projection_state()
        .map_err(DaemonError::store)?
        .service_state
        == SearchProjectionServiceState::RepairBlocked)
}

fn mark_migration_rebuild_blocked(
    store: &OwnedMetaStore,
    reason: SearchRepairReason,
    now: UnixTimestamp,
) -> Result<()> {
    let _ = store
        .block_migration_rebuild(reason, now)
        .map_err(DaemonError::store)?;
    Ok(())
}

fn run_ocr_worker_once(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
) -> Result<OcrWorkerSummary> {
    let now = current_timestamp()?;
    match ocr_preclaim_decision(store).map_err(DaemonError::import)? {
        OcrPreclaimDecision::Ready => {}
        OcrPreclaimDecision::NotReady(_) => return Ok(OcrWorkerSummary::default()),
    }
    if store
        .worker_task_control(WorkerTaskKind::Ocr)
        .map_err(DaemonError::store)?
        .paused
    {
        return Ok(OcrWorkerSummary {
            paused: true,
            ..OcrWorkerSummary::default()
        });
    }

    if options.ocr_command.is_none() && options.ocr_tesseract_command.is_none() {
        return Err(DaemonError::configuration_invalid(
            "ocr worker blocked: local OCR command not configured",
        ));
    }

    let stale_recovered = recover_stale_ingest_jobs(store, now)?;
    let Some(job) = store.claim_next_ocr_job(now).map_err(DaemonError::store)? else {
        return Ok(OcrWorkerSummary {
            stale_recovered,
            ..OcrWorkerSummary::default()
        });
    };

    let mut summary = match run_claimed_ocr_job(data_dir, store, &job, options, now) {
        Ok(summary) => summary,
        Err(error) => {
            mark_ocr_job_failed_retryable(store, &job, now)?;
            return Err(error);
        }
    };
    summary.stale_recovered = stale_recovered;
    Ok(summary)
}

fn run_ocr_worker_batch(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    jobs_per_tick: usize,
) -> Result<OcrWorkerSummary> {
    let mut aggregate = OcrWorkerSummary::default();
    for _ in 0..jobs_per_tick {
        let summary = run_ocr_worker_once(data_dir, store, options)?;
        let stop_after_summary = summary.paused || (summary.processed == 0 && summary.failed == 0);
        aggregate.extend(summary);
        if stop_after_summary {
            break;
        }
    }
    Ok(aggregate)
}

fn run_claimed_ocr_job(
    data_dir: &Path,
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    options: &RunOptions,
    now: UnixTimestamp,
) -> Result<OcrWorkerSummary> {
    let Some(document) = store
        .document_by_id(&job.job.document_id)
        .map_err(DaemonError::store)?
    else {
        mark_ocr_job_failed_permanent(store, job, now)?;
        return Ok(OcrWorkerSummary {
            failed: 1,
            ..OcrWorkerSummary::default()
        });
    };
    let content_hash = job.source_fingerprint().to_string();

    let bytes = match fs::read(&document.normalized_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            mark_ocr_job_failed_retryable(store, job, now)?;
            return Ok(OcrWorkerSummary {
                failed: 1,
                ..OcrWorkerSummary::default()
            });
        }
    };
    let page_count = match detect_ocr_page_count(&document.extension, &bytes) {
        Ok(page_count) => page_count,
        Err(error) => return Err(DaemonError::import(error)),
    };
    if page_count > options.ocr_max_pages_per_document {
        mark_ocr_job_failed_retryable_with_failure_kind(
            store,
            job,
            IngestJobFailureKind::OcrPageBudgetExceeded,
            now,
        )?;
        return Ok(OcrWorkerSummary {
            failed: 1,
            ..OcrWorkerSummary::default()
        });
    }
    let budget = OcrWorkerBudget::new(options.ocr_page_timeout_ms).map_err(DaemonError::ocr)?;
    let cancellation = CancellationToken::new();
    let ocr_options = OcrOptions::new(options.ocr_lang.as_str(), options.ocr_profile.as_str())
        .map_err(DaemonError::ocr)?;
    let command_client = options
        .ocr_command
        .clone()
        .map(|command| {
            LocalOcrCommandSpec::new(
                command,
                Vec::<String>::new(),
                options.ocr_engine_profile.as_str(),
            )
            .map(LocalOcrCommandClient::new)
            .map_err(DaemonError::ocr)
        })
        .transpose()?;
    let tesseract_client = options
        .ocr_tesseract_command
        .clone()
        .map(|tesseract_command| {
            TesseractOcrSpec::new(tesseract_command, options.ocr_engine_profile.as_str())
                .map(TesseractOcrClient::new)
                .map_err(DaemonError::ocr)
        })
        .transpose()?;
    let renderer = options
        .ocr_render_command
        .clone()
        .map(|render_command| {
            LocalPdfRenderCommandSpec::new(render_command, Vec::<String>::new())
                .map(LocalPdfRenderCommandClient::new)
                .map_err(DaemonError::ocr)
        })
        .transpose()?;
    let pdftoppm_renderer = options
        .ocr_pdftoppm_command
        .clone()
        .map(|pdftoppm_command| {
            PdftoppmRenderSpec::new(pdftoppm_command)
                .map(PdftoppmPdfRenderer::new)
                .map_err(DaemonError::ocr)
        })
        .transpose()?;

    let mut page_texts = Vec::new();
    let mut confidence_sum = 0.0_f32;
    let mut confidence_count = 0_usize;
    let mut cache_writes = 0_usize;
    let mut cache_hits = 0_usize;

    for page_no in 1..=page_count {
        let cache_key = OcrPageCacheKey::new(
            content_hash.clone(),
            page_no,
            options.ocr_render_dpi,
            options.ocr_lang.as_str(),
            options.ocr_profile.as_str(),
        )
        .map_err(DaemonError::store)?;

        if let Some(entry) = store
            .ocr_page_cache_entry(&cache_key)
            .map_err(DaemonError::store)?
            .filter(|entry| entry.status() == OcrPageCacheStatus::Succeeded)
        {
            page_texts.push(entry.text().unwrap_or("").to_string());
            if let Some(confidence) = entry.confidence() {
                confidence_sum += confidence;
                confidence_count += 1;
            }
            cache_hits += 1;
            continue;
        }

        if command_client.is_none() {
            if let Some(tesseract_command) = options.ocr_tesseract_command.as_ref() {
                match inspect_tesseract_language_availability(
                    tesseract_command,
                    options.ocr_lang.as_str(),
                ) {
                    TesseractLanguageAvailability::Available => {}
                    TesseractLanguageAvailability::Missing => {
                        let entry = OcrPageCacheEntry::failed_retryable(
                            cache_key,
                            "LanguageUnavailable",
                            now,
                        )
                        .map_err(DaemonError::store)?;
                        store
                            .upsert_ocr_page_cache_entry(&entry)
                            .map_err(DaemonError::store)?;
                        mark_ocr_job_failed_retryable(store, job, now)?;
                        return Ok(OcrWorkerSummary {
                            failed: 1,
                            ..OcrWorkerSummary::default()
                        });
                    }
                    TesseractLanguageAvailability::Unknown => {
                        let entry = OcrPageCacheEntry::failed_retryable(
                            cache_key,
                            "WorkerUnavailable",
                            now,
                        )
                        .map_err(DaemonError::store)?;
                        store
                            .upsert_ocr_page_cache_entry(&entry)
                            .map_err(DaemonError::store)?;
                        mark_ocr_job_failed_retryable(store, job, now)?;
                        return Ok(OcrWorkerSummary {
                            failed: 1,
                            ..OcrWorkerSummary::default()
                        });
                    }
                }
            }
        }

        let rendered_page = if let Some(renderer) = &renderer {
            match renderer.render_page(
                &bytes,
                page_no,
                options.ocr_render_dpi,
                budget,
                &cancellation,
            ) {
                Ok(rendered_page) => rendered_page,
                Err(error) => {
                    let entry = OcrPageCacheEntry::failed_retryable(
                        cache_key,
                        format!("{:?}", error.kind()),
                        now,
                    )
                    .map_err(DaemonError::store)?;
                    store
                        .upsert_ocr_page_cache_entry(&entry)
                        .map_err(DaemonError::store)?;
                    mark_ocr_job_failed_retryable(store, job, now)?;
                    return Ok(OcrWorkerSummary {
                        failed: 1,
                        ..OcrWorkerSummary::default()
                    });
                }
            }
        } else if let Some(renderer) = &pdftoppm_renderer {
            match renderer.render_page(
                &bytes,
                page_no,
                options.ocr_render_dpi,
                budget,
                &cancellation,
            ) {
                Ok(rendered_page) => rendered_page,
                Err(error) => {
                    let entry = OcrPageCacheEntry::failed_retryable(
                        cache_key,
                        format!("{:?}", error.kind()),
                        now,
                    )
                    .map_err(DaemonError::store)?;
                    store
                        .upsert_ocr_page_cache_entry(&entry)
                        .map_err(DaemonError::store)?;
                    mark_ocr_job_failed_retryable(store, job, now)?;
                    return Ok(OcrWorkerSummary {
                        failed: 1,
                        ..OcrWorkerSummary::default()
                    });
                }
            }
        } else {
            RenderedPage::new(page_no, options.ocr_render_dpi, bytes.clone())
                .map_err(DaemonError::ocr)?
        };
        let request =
            OcrPageRequest::new(rendered_page, ocr_options.clone()).map_err(DaemonError::ocr)?;

        let page_result = if let Some(client) = &command_client {
            client.recognize_page(request, budget, &cancellation)
        } else if let Some(client) = &tesseract_client {
            client.recognize_page(request, budget, &cancellation)
        } else {
            return Err(DaemonError::configuration_invalid(
                "ocr worker blocked: local OCR command not configured",
            ));
        };
        let page = match page_result {
            Ok(page) => page,
            Err(error) => {
                let entry = OcrPageCacheEntry::failed_retryable(
                    cache_key,
                    format!("{:?}", error.kind()),
                    now,
                )
                .map_err(DaemonError::store)?;
                store
                    .upsert_ocr_page_cache_entry(&entry)
                    .map_err(DaemonError::store)?;
                mark_ocr_job_failed_retryable(store, job, now)?;
                return Ok(OcrWorkerSummary {
                    failed: 1,
                    ..OcrWorkerSummary::default()
                });
            }
        };
        let word_boxes = ocr_word_boxes_for_cache(&page)?;
        let entry = OcrPageCacheEntry::succeeded_with_word_boxes(
            cache_key,
            page.text(),
            page.confidence(),
            page.engine_profile(),
            page.duration_ms(),
            word_boxes,
            now,
        )
        .map_err(DaemonError::store)?;
        store
            .upsert_ocr_page_cache_entry(&entry)
            .map_err(DaemonError::store)?;
        page_texts.push(page.text().to_string());
        confidence_sum += page.confidence();
        confidence_count += 1;
        cache_writes += 1;
    }

    let combined_text = page_texts.join("\n");
    let confidence = (confidence_count > 0).then_some(confidence_sum / confidence_count as f32);
    let outcome = match index_claimed_ocr_text_with_policy(
        data_dir,
        store,
        job,
        &combined_text,
        confidence,
        Some(page_count),
        now,
        &options.linear_promotion,
        &options.search_vectorization,
    ) {
        Ok(outcome) => outcome,
        Err(error) => return Err(DaemonError::import(error)),
    };
    Ok(OcrWorkerSummary {
        processed: usize::from(matches!(
            outcome,
            import_pipeline::OcrTextIndexOutcome::Committed(_)
        )),
        cache_writes,
        cache_hits,
        ..OcrWorkerSummary::default()
    })
}

fn ocr_word_boxes_for_cache(page: &ocr_client::OcrPage) -> Result<Vec<meta_store::OcrWordBox>> {
    page.word_boxes()
        .iter()
        .map(|word_box| {
            meta_store::OcrWordBox::new(
                word_box.text(),
                word_box.left(),
                word_box.top(),
                word_box.width(),
                word_box.height(),
                word_box.confidence(),
            )
            .map_err(DaemonError::store)
        })
        .collect()
}

fn mark_ocr_job_failed_retryable(
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .finish_ocr_attempt_failure(job, OcrAttemptFailure::Retryable, now)
        .map(|_| ())
        .map_err(DaemonError::store)
}

fn mark_ocr_job_failed_retryable_with_failure_kind(
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    failure_kind: IngestJobFailureKind,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .finish_ocr_attempt_failure(job, OcrAttemptFailure::RetryableWithKind(failure_kind), now)
        .map(|_| ())
        .map_err(DaemonError::store)
}

fn mark_ocr_job_failed_permanent(
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .finish_ocr_attempt_failure(job, OcrAttemptFailure::Permanent, now)
        .map(|_| ())
        .map_err(DaemonError::store)
}

fn recover_stale_ingest_jobs(store: &OwnedMetaStore, now: UnixTimestamp) -> Result<usize> {
    store
        .recover_stale_running_ingest_jobs(
            now,
            timestamp_minus_seconds(now, STALE_INGEST_JOB_SECONDS),
        )
        .map_err(DaemonError::store)
}

fn recover_stale_import_tasks(
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

struct ImportWatcher {
    watcher: RecommendedWatcher,
    receiver: Receiver<notify::Result<NotifyEvent>>,
    watched_roots: BTreeSet<String>,
    watched_root_mtimes: BTreeMap<String, Option<u128>>,
    pending_roots: BTreeSet<String>,
}

impl ImportWatcher {
    fn new() -> Result<Self> {
        let (sender, receiver) = mpsc::channel();
        let watcher = RecommendedWatcher::new(
            move |event| {
                let _ = sender.send(event);
            },
            NotifyConfig::default(),
        )
        .map_err(|_| {
            DaemonError::recoverable_dependency("import watcher blocked: local watcher unavailable")
        })?;

        Ok(Self {
            watcher,
            receiver,
            watched_roots: BTreeSet::new(),
            watched_root_mtimes: BTreeMap::new(),
            pending_roots: BTreeSet::new(),
        })
    }

    fn sync_and_requeue(
        &mut self,
        store: &OwnedMetaStore,
        processing_contract: &ImportProcessingContract,
        now: UnixTimestamp,
    ) -> Result<ImportWatcherSummary> {
        let scopes = store
            .completed_import_scan_scopes_due_for_requeue(now)
            .map_err(DaemonError::store)?;
        let scopes_by_root = scopes
            .into_iter()
            .map(|scope| (scope.canonical_root_path.clone(), scope))
            .collect::<BTreeMap<_, _>>();
        let roots = scopes_by_root.keys().cloned().collect::<BTreeSet<_>>();
        let mut summary = self.sync_watched_roots(&roots);
        self.drain_events(&scopes_by_root, &mut summary);
        self.poll_changed_roots(&roots, &mut summary);
        let pending_roots = std::mem::take(&mut self.pending_roots);

        for (index, root) in pending_roots.into_iter().enumerate() {
            let Some(scope) = scopes_by_root.get(&root).cloned() else {
                continue;
            };
            if rescan_schedule::enqueue_import_from_completed_scope(
                store,
                processing_contract,
                scope,
                import_command::new_task_id(index)
                    .map_err(|_| DaemonError::user("system clock is before unix epoch"))?,
                now,
            )? {
                summary.requeued += 1;
            }
        }

        Ok(summary)
    }

    fn sync_watched_roots(&mut self, roots: &BTreeSet<String>) -> ImportWatcherSummary {
        let previous_roots = self.watched_roots.clone();
        let mut next_roots = BTreeSet::new();
        let mut event_errors = 0_usize;

        for root in previous_roots.difference(roots) {
            if self.watcher.unwatch(Path::new(root)).is_err() {
                event_errors += 1;
            }
            self.watched_root_mtimes.remove(root);
        }

        for root in roots {
            if previous_roots.contains(root) {
                next_roots.insert(root.clone());
                continue;
            }
            if !Path::new(root).exists() {
                event_errors += 1;
                continue;
            }
            if self
                .watcher
                .watch(Path::new(root), RecursiveMode::Recursive)
                .is_ok()
            {
                self.watched_root_mtimes
                    .insert(root.clone(), import_watcher_root_mtime(root));
                next_roots.insert(root.clone());
            } else {
                event_errors += 1;
            }
        }

        self.watched_roots = next_roots;
        ImportWatcherSummary {
            active_roots: (self.watched_roots != previous_roots)
                .then_some(self.watched_roots.len()),
            event_errors,
            ..ImportWatcherSummary::default()
        }
    }

    fn drain_events(
        &mut self,
        scopes_by_root: &BTreeMap<String, ImportScanScope>,
        summary: &mut ImportWatcherSummary,
    ) {
        loop {
            match self.receiver.try_recv() {
                Ok(Ok(event)) => {
                    if !import_watcher_event_is_relevant(&event) {
                        continue;
                    }
                    summary.events += 1;
                    for path in event.paths {
                        if let Some(root) = import_watcher_root_for_path(scopes_by_root, &path) {
                            self.pending_roots.insert(root.to_string());
                        }
                    }
                }
                Ok(Err(_)) => {
                    summary.event_errors += 1;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    summary.event_errors += 1;
                    return;
                }
            }
        }
    }

    fn poll_changed_roots(&mut self, roots: &BTreeSet<String>, summary: &mut ImportWatcherSummary) {
        for root in roots {
            if !self.watched_roots.contains(root) {
                continue;
            }
            let previous_mtime = self.watched_root_mtimes.get(root).copied().flatten();
            let current_mtime = import_watcher_root_mtime(root);
            self.watched_root_mtimes.insert(root.clone(), current_mtime);
            if previous_mtime.is_some() && current_mtime != previous_mtime {
                summary.events += 1;
                self.pending_roots.insert(root.clone());
            }
        }
    }
}

#[derive(Default)]
struct ImportWatcherSummary {
    active_roots: Option<usize>,
    events: usize,
    requeued: usize,
    event_errors: usize,
}

fn import_watcher_event_is_relevant(event: &NotifyEvent) -> bool {
    matches!(
        event.kind,
        NotifyEventKind::Any
            | NotifyEventKind::Create(_)
            | NotifyEventKind::Modify(_)
            | NotifyEventKind::Remove(_)
    )
}

fn import_watcher_root_for_path<'a>(
    scopes_by_root: &'a BTreeMap<String, ImportScanScope>,
    event_path: &Path,
) -> Option<&'a str> {
    scopes_by_root
        .keys()
        .find(|root| event_path.starts_with(Path::new(root.as_str())))
        .map(String::as_str)
}

fn import_watcher_root_mtime(root: &str) -> Option<u128> {
    fs::metadata(root)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
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
                ))
            }
        },
        linear_promotion: options.linear_promotion.clone(),
        search_vectorization: options.search_vectorization.clone(),
        ..ImportOptions::default()
    })
}

fn current_timestamp() -> Result<UnixTimestamp> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DaemonError::user("system clock is before unix epoch"))?
        .as_secs();
    let seconds =
        i64::try_from(seconds).map_err(|_| DaemonError::user("system timestamp is too large"))?;
    Ok(UnixTimestamp::from_unix_seconds(seconds))
}

fn timestamp_minus_seconds(now: UnixTimestamp, seconds: i64) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(now.as_unix_seconds().saturating_sub(seconds))
}

fn timestamp_at_or_after(now: UnixTimestamp, floor: UnixTimestamp) -> UnixTimestamp {
    UnixTimestamp::from_unix_seconds(now.as_unix_seconds().max(floor.as_unix_seconds()))
}

fn u64_to_usize(value: u64) -> Result<usize> {
    usize::try_from(value).map_err(|_| DaemonError::user("scan budget is too large"))
}

fn parse_loopback_addr(value: &str) -> Result<SocketAddr> {
    let addr = SocketAddr::from_str(value).map_err(|_| DaemonError::usage(run_usage()))?;
    if !addr.ip().is_loopback() {
        return Err(DaemonError::usage("ipc listener must bind to loopback"));
    }
    Ok(addr)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ParentLifecycleMode {
    #[default]
    Unmanaged,
    Stdin,
}

#[derive(Clone)]
struct RunOptions {
    foreground: bool,
    parent_lifecycle: ParentLifecycleMode,
    once: bool,
    ipc_listen: Option<SocketAddr>,
    expected_ipc_protocol: Option<String>,
    max_requests: Option<usize>,
    work_imports_once: bool,
    work_imports: bool,
    rescan_completed_imports: bool,
    watch_import_roots: bool,
    import_rescan_min_age_seconds: Option<i64>,
    stale_import_task_seconds: Option<i64>,
    import_retry_backoff_seconds: Option<i64>,
    classifier_model_configured: bool,
    linear_promotion: LinearPromotionPolicy,
    work_ocr_once: bool,
    work_ocr: bool,
    work_index_once: bool,
    work_index: bool,
    ocr_command: Option<PathBuf>,
    ocr_tesseract_command: Option<PathBuf>,
    ocr_render_command: Option<PathBuf>,
    ocr_pdftoppm_command: Option<PathBuf>,
    ocr_engine_profile: String,
    ocr_lang: String,
    ocr_profile: String,
    ocr_render_dpi: u32,
    ocr_page_timeout_ms: u64,
    ocr_max_pages_per_document: u32,
    ocr_jobs_per_tick: Option<usize>,
    embedding_command: Option<PathBuf>,
    embedding_model_id: Option<String>,
    embedding_dimension: Option<usize>,
    embedding_timeout_ms: u64,
    resident_embedding: Option<ResidentEmbeddingClient>,
    search_vectorization: SearchPublicationVectorization,
    worker_interval_ms: Option<u64>,
    max_worker_ticks: Option<usize>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            foreground: false,
            parent_lifecycle: ParentLifecycleMode::Unmanaged,
            once: false,
            ipc_listen: None,
            expected_ipc_protocol: None,
            max_requests: None,
            work_imports_once: false,
            work_imports: false,
            rescan_completed_imports: false,
            watch_import_roots: false,
            import_rescan_min_age_seconds: None,
            stale_import_task_seconds: None,
            import_retry_backoff_seconds: None,
            classifier_model_configured: false,
            linear_promotion: LinearPromotionPolicy::default(),
            work_ocr_once: false,
            work_ocr: false,
            work_index_once: false,
            work_index: false,
            ocr_command: None,
            ocr_tesseract_command: None,
            ocr_render_command: None,
            ocr_pdftoppm_command: None,
            ocr_engine_profile: DEFAULT_OCR_ENGINE_PROFILE.to_string(),
            ocr_lang: DEFAULT_OCR_LANG.to_string(),
            ocr_profile: DEFAULT_OCR_PROFILE.to_string(),
            ocr_render_dpi: DEFAULT_OCR_RENDER_DPI,
            ocr_page_timeout_ms: DEFAULT_OCR_PAGE_TIMEOUT_MS,
            ocr_max_pages_per_document: DEFAULT_OCR_MAX_PAGES_PER_DOCUMENT,
            ocr_jobs_per_tick: None,
            embedding_command: None,
            embedding_model_id: None,
            embedding_dimension: None,
            embedding_timeout_ms: DEFAULT_EMBEDDING_TIMEOUT_MS,
            resident_embedding: None,
            search_vectorization: SearchPublicationVectorization::default(),
            worker_interval_ms: None,
            max_worker_ticks: None,
        }
    }
}

impl RunOptions {
    fn has_worker_loop(&self) -> bool {
        self.work_imports || self.work_ocr || self.work_index
    }
}

fn search_runtime_config(options: &RunOptions) -> search_runtime_config::SearchRuntimeConfig {
    search_runtime_config::SearchRuntimeConfig::new(
        options.resident_embedding.clone(),
        options.embedding_model_id.clone(),
        options.embedding_dimension,
        options.embedding_timeout_ms,
    )
}

#[derive(Default)]
struct ImportWorkerSummary {
    orphaned_recovered: usize,
    stale_recovered: usize,
    repair_requeued: usize,
    completed_requeued: usize,
    watcher_active_roots: Option<usize>,
    watcher_events: usize,
    watcher_requeued: usize,
    watcher_event_errors: usize,
    processed: usize,
    cancelled: usize,
    failed: usize,
    failure_class: Option<ImportPipelineErrorClass>,
    metadata_failure_class: Option<&'static str>,
    searchable_documents: usize,
    ocr_jobs_queued: usize,
}

impl ImportWorkerSummary {
    fn has_activity(&self) -> bool {
        self.orphaned_recovered > 0
            || self.stale_recovered > 0
            || self.repair_requeued > 0
            || self.completed_requeued > 0
            || self.watcher_active_roots.is_some()
            || self.watcher_events > 0
            || self.watcher_requeued > 0
            || self.watcher_event_errors > 0
            || self.processed > 0
            || self.cancelled > 0
            || self.failed > 0
            || self.searchable_documents > 0
            || self.ocr_jobs_queued > 0
    }

    fn extend(&mut self, other: Self) {
        self.orphaned_recovered += other.orphaned_recovered;
        self.stale_recovered += other.stale_recovered;
        self.repair_requeued += other.repair_requeued;
        self.completed_requeued += other.completed_requeued;
        if other.watcher_active_roots.is_some() {
            self.watcher_active_roots = other.watcher_active_roots;
        }
        self.watcher_events += other.watcher_events;
        self.watcher_requeued += other.watcher_requeued;
        self.watcher_event_errors += other.watcher_event_errors;
        self.processed += other.processed;
        self.cancelled += other.cancelled;
        self.failed += other.failed;
        if other.failure_class.is_some() {
            self.failure_class = other.failure_class;
        }
        if other.metadata_failure_class.is_some() {
            self.metadata_failure_class = other.metadata_failure_class;
        }
        self.searchable_documents += other.searchable_documents;
        self.ocr_jobs_queued += other.ocr_jobs_queued;
    }

    fn extend_watcher(&mut self, watcher_summary: ImportWatcherSummary) {
        if watcher_summary.active_roots.is_some() {
            self.watcher_active_roots = watcher_summary.active_roots;
        }
        self.watcher_events += watcher_summary.events;
        self.watcher_requeued += watcher_summary.requeued;
        self.watcher_event_errors += watcher_summary.event_errors;
    }
}

fn print_import_worker_summary(import_summary: &ImportWorkerSummary) -> Result<()> {
    println!(
        "import worker recovered orphaned running: {}",
        import_summary.orphaned_recovered
    );
    println!(
        "import worker recovered stale running: {}",
        import_summary.stale_recovered
    );
    println!(
        "import worker requeued completed imports: {}",
        import_summary.completed_requeued
    );
    println!(
        "import worker queued migration repairs: {}",
        import_summary.repair_requeued
    );
    if let Some(active_roots) = import_summary.watcher_active_roots {
        println!("import watcher active roots: {active_roots}");
    }
    println!("import watcher events: {}", import_summary.watcher_events);
    println!(
        "import watcher requeued imports: {}",
        import_summary.watcher_requeued
    );
    println!(
        "import watcher event errors: {}",
        import_summary.watcher_event_errors
    );
    println!("import worker processed: {}", import_summary.processed);
    println!("import worker cancelled: {}", import_summary.cancelled);
    println!("import worker failed: {}", import_summary.failed);
    if let Some(class) = import_summary.failure_class {
        println!("import worker failure class: {}", class.label());
    }
    if let Some(class) = import_summary.metadata_failure_class {
        println!("import worker metadata failure class: {class}");
    }
    println!(
        "import worker searchable documents: {}",
        import_summary.searchable_documents
    );
    println!(
        "import worker ocr jobs queued: {}",
        import_summary.ocr_jobs_queued
    );
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::control_plane("unable to write daemon status"))
}

#[derive(Default)]
struct OcrWorkerSummary {
    stale_recovered: usize,
    paused: bool,
    processed: usize,
    failed: usize,
    cache_writes: usize,
    cache_hits: usize,
}

impl OcrWorkerSummary {
    fn has_activity(&self) -> bool {
        self.stale_recovered > 0
            || self.paused
            || self.processed > 0
            || self.failed > 0
            || self.cache_writes > 0
            || self.cache_hits > 0
    }

    fn extend(&mut self, other: Self) {
        self.stale_recovered += other.stale_recovered;
        self.paused = self.paused || other.paused;
        self.processed += other.processed;
        self.failed += other.failed;
        self.cache_writes += other.cache_writes;
        self.cache_hits += other.cache_hits;
    }
}

fn print_ocr_worker_summary(summary: &OcrWorkerSummary) -> Result<()> {
    println!(
        "ingest jobs recovered stale running: {}",
        summary.stale_recovered
    );
    println!("ocr worker paused: {}", summary.paused);
    println!("ocr worker processed: {}", summary.processed);
    println!("ocr worker cache writes: {}", summary.cache_writes);
    println!("ocr worker cache hits: {}", summary.cache_hits);
    println!("ocr worker failed: {}", summary.failed);
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::control_plane("unable to write daemon status"))
}

fn search_artifact_recovery_has_activity(summary: &SearchArtifactRecoverySummary) -> bool {
    summary.interrupted_publications_abandoned > 0
        || summary.fulltext_staging_directories_removed > 0
        || summary.vector_staging_directories_removed > 0
        || summary.fulltext_generations_removed > 0
        || summary.vector_generations_removed > 0
        || summary.active_generation_rebuilt
        || summary.gc_deferred
        || summary.gc_partial
        || summary.gc_failed
}

fn print_search_artifact_worker_summary(summary: &SearchArtifactRecoverySummary) -> Result<()> {
    println!(
        "search artifact worker active generation rebuilt: {}",
        if summary.active_generation_rebuilt {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "search artifact worker interrupted publications abandoned: {}",
        summary.interrupted_publications_abandoned
    );
    println!(
        "search artifact worker fulltext staging removed: {}",
        summary.fulltext_staging_directories_removed
    );
    println!(
        "search artifact worker vector staging removed: {}",
        summary.vector_staging_directories_removed
    );
    println!(
        "search artifact worker fulltext generations removed: {}",
        summary.fulltext_generations_removed
    );
    println!(
        "search artifact worker vector generations removed: {}",
        summary.vector_generations_removed
    );
    println!(
        "search artifact worker gc deferred: {}",
        summary.gc_deferred
    );
    println!("search artifact worker gc partial: {}", summary.gc_partial);
    println!("search artifact worker gc failed: {}", summary.gc_failed);
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::control_plane("unable to write daemon status"))
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

fn open_store(data_dir: &Path) -> Result<ReadMetaStore> {
    ReadMetaStore::open_data_dir(data_dir).map_err(DaemonError::store)
}

fn open_owned_store(owner: &import_pipeline::DataDirectoryOwnerLease) -> Result<OwnedMetaStore> {
    owner.open_store().map_err(DaemonError::store)
}

fn index_health_label(status: IndexStateStatus) -> &'static str {
    match status {
        IndexStateStatus::Empty => "empty",
        IndexStateStatus::Building => "building",
        IndexStateStatus::Ready => "ready",
        IndexStateStatus::Stale => "stale",
    }
}

#[cfg(test)]
mod daemon_contract_tests {
    use super::{
        import_processing, open_owned_store, run_import_worker_once_with_retry_due,
        run_ocr_worker_once, should_requeue_interrupted_import, ImportPipelineErrorClass,
        PipelineRunControl, RunOptions, IPC_METADATA_READ_ATTEMPTS,
    };
    use crate::ipc::projection_service_health;
    use crate::ipc::routes::status::{
        projection_query_error, status_json_with, unavailable_status_json,
    };
    use crate::worker_runtime::run_fault_priority_gate;
    use import_pipeline::prepare_migration_rebuild_artifacts;
    use meta_store::{
        ClassificationStatus, ContentDigest, CurrentClassifierEpoch, Document, DocumentId,
        DocumentStatus, FileExtension, ImportProcessingContract, ImportRootKind, ImportScanProfile,
        ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus, IngestJobId, IngestJobStatus,
        MetaStoreErrorClass, OwnedMetaStore, ReasonCode, SearchProjectionServiceState,
        SearchRepairReason, SourceRevision, SourceRevisionTriage, UnixTimestamp, CLASSIFIER_EPOCH,
    };
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn open_test_store(data_dir: &std::path::Path) -> OwnedMetaStore {
        let owner = import_processing::acquire_owner(data_dir).unwrap();
        open_owned_store(&owner).unwrap()
    }

    #[test]
    fn reported_artifact_fault_completes_before_lower_priority_work_enters() {
        let phase = AtomicUsize::new(0);
        let (lower_entered_sender, lower_entered_receiver) = mpsc::sync_channel(1);
        let (lower_release_sender, lower_release_receiver) = mpsc::sync_channel(1);

        std::thread::scope(|scope| {
            let worker_phase = &phase;
            let worker = scope.spawn(move || {
                run_fault_priority_gate(
                    || {
                        assert_eq!(worker_phase.swap(1, Ordering::SeqCst), 0);
                        Ok(true)
                    },
                    |repaired_reported_fault| {
                        assert!(repaired_reported_fault);
                        assert_eq!(worker_phase.load(Ordering::SeqCst), 1);
                        lower_entered_sender.send(()).unwrap();
                        lower_release_receiver.recv().unwrap();
                        worker_phase.store(2, Ordering::SeqCst);
                        Ok(())
                    },
                )
            });

            lower_entered_receiver.recv().unwrap();
            assert_eq!(phase.load(Ordering::SeqCst), 1);
            lower_release_sender.send(()).unwrap();
            worker.join().unwrap().unwrap();
        });
        assert_eq!(phase.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn durable_user_cancellation_is_never_requeued_as_lifecycle_interruption() {
        assert!(!should_requeue_interrupted_import(
            ImportPipelineErrorClass::Cancelled,
            true,
            true,
        ));
        assert!(should_requeue_interrupted_import(
            ImportPipelineErrorClass::Cancelled,
            true,
            false,
        ));
        assert!(should_requeue_interrupted_import(
            ImportPipelineErrorClass::Interrupted,
            true,
            false,
        ));
        assert!(!should_requeue_interrupted_import(
            ImportPipelineErrorClass::Cancelled,
            false,
            false,
        ));
    }

    #[test]
    fn metadata_unavailable_status_keeps_process_and_service_health() {
        let body = unavailable_status_json();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["schema_version"], "daemon.status.v2");
        assert_eq!(value["process_state"], "ready");
        assert_eq!(value["services"]["metadata"], "unavailable");
        assert_eq!(value["services"]["query"], "unavailable");
        assert_eq!(value["error"]["code"], "METADATA_UNAVAILABLE");
        assert!(value["ipc"].is_object());
        assert!(value["indexed_documents"].is_null());
    }

    #[test]
    fn worker_cancels_ready_task_bound_to_a_different_processing_contract() {
        let data_dir = std::env::temp_dir().join(format!(
            "resume-ir-daemon-contract-mismatch-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let store = open_test_store(&data_dir);
        let options = RunOptions::default();
        let contract = import_processing::current_contract(&options).unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_800_280_000);
        import_processing::activate_contract(&store, &contract, now).unwrap();
        prepare_migration_rebuild_artifacts(&store, now, &PipelineRunControl::default()).unwrap();
        super::finalize_migration_rebuild(
            &store,
            now,
            &contract,
            &options.search_vectorization,
            &PipelineRunControl::default(),
        )
        .unwrap();

        let wrong_contract = ImportProcessingContract::new(
            "synthetic-wrong-primary-v28",
            "synthetic-wrong-ocr-v28",
            contract.derived_schema_version(),
            contract.classifier_epoch(),
        )
        .unwrap();
        let task = ImportTask {
            id: ImportTaskId::from_non_secret_parts(&["daemon-wrong-contract"]),
            root_path: "/synthetic/wrong-contract".to_string(),
            status: ImportTaskStatus::Queued,
            queued_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
        };
        let scope = ImportScanScope {
            import_task_id: task.id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: task.root_path.clone(),
            canonical_root_path: task.root_path.clone(),
            files_discovered: 0,
            ignored_entries: 0,
            scan_errors: 0,
            searchable_documents: 0,
            ocr_required_documents: 0,
            ocr_jobs_queued: 0,
            failed_documents: 0,
            deleted_documents: 0,
            scan_budget_kind: None,
            scan_budget_limit: None,
            scan_budget_observed: None,
            scan_budget_exhausted: false,
            updated_at: now,
        };
        store
            .insert_import_task_with_scan_scope(&task, &scope, &wrong_contract)
            .unwrap();

        let summary = run_import_worker_once_with_retry_due(
            &data_dir,
            &store,
            &options,
            &contract,
            now,
            PipelineRunControl::default(),
        )
        .unwrap();
        assert_eq!(summary.failed, 1);
        assert!(store.is_import_task_cancelled(&task.id).unwrap());
        assert_eq!(
            store
                .import_task_processing_contract_id(&task.id)
                .unwrap()
                .as_ref(),
            Some(wrong_contract.id())
        );

        drop(store);
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn runtime_metadata_read_failure_returns_status_v2_with_unavailable_dependencies() {
        let mut attempts = 0;
        let body = status_json_with(|| {
            attempts += 1;
            Err(MetaStoreErrorClass::Storage)
        });
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();

        assert_eq!(attempts, IPC_METADATA_READ_ATTEMPTS);
        assert_eq!(value["schema_version"], "daemon.status.v2");
        assert_eq!(value["process_state"], "ready");
        assert_eq!(value["service_state"], "degraded");
        assert_eq!(value["services"]["metadata"], "unavailable");
        assert_eq!(value["services"]["query"], "unavailable");
        assert_eq!(value["error"]["code"], "METADATA_UNAVAILABLE");
    }

    #[test]
    fn v27_projection_repair_state_is_not_reported_ready() {
        let repairing = projection_service_health(SearchProjectionServiceState::Repairing);
        let blocked = projection_service_health(SearchProjectionServiceState::RepairBlocked);
        assert_eq!(repairing.aggregate(), crate::ipc::ServiceState::Repairing);
        assert_eq!(blocked.aggregate(), crate::ipc::ServiceState::Degraded);
    }

    #[test]
    fn v27_projection_state_gates_query_routes_with_fixed_codes() {
        assert_eq!(
            projection_query_error(Some(SearchProjectionServiceState::Ready)),
            None
        );
        assert_eq!(
            projection_query_error(Some(SearchProjectionServiceState::Repairing)),
            Some(crate::ipc::ServiceErrorCode::Repairing)
        );
        assert_eq!(
            projection_query_error(Some(SearchProjectionServiceState::RepairBlocked)),
            Some(crate::ipc::ServiceErrorCode::QueryServiceRepairRequired)
        );
        assert_eq!(
            projection_query_error(None),
            Some(crate::ipc::ServiceErrorCode::MetadataUnavailable)
        );
    }

    #[test]
    fn unpublished_migration_repair_does_not_claim_ocr_until_projection_is_ready() {
        let data_dir = std::env::temp_dir().join(format!(
            "resume-ir-daemon-ocr-migration-gate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let store = open_test_store(&data_dir);
        let now = UnixTimestamp::from_unix_seconds(1_800_281_000);
        let job_id = enqueue_ocr_job_for_worker_gate(&data_dir, &store, now, "repairing");
        let options = RunOptions {
            ocr_command: Some(data_dir.join("unused-ocr-command")),
            ..RunOptions::default()
        };

        let repairing_summary = run_ocr_worker_once(&data_dir, &store, &options).unwrap();
        assert!(!repairing_summary.paused);
        assert!(!repairing_summary.has_activity());
        let still_queued = store.ingest_job_by_id(&job_id).unwrap().unwrap();
        assert_eq!(still_queued.status, IngestJobStatus::Queued);
        assert_eq!(still_queued.attempt_count, 0);

        let contract = import_processing::current_contract(&options).unwrap();
        import_processing::activate_contract(&store, &contract, now).unwrap();
        prepare_migration_rebuild_artifacts(&store, now, &PipelineRunControl::default()).unwrap();
        super::finalize_migration_rebuild(
            &store,
            now,
            &contract,
            &options.search_vectorization,
            &PipelineRunControl::default(),
        )
        .unwrap();
        assert_eq!(
            store.search_projection_state().unwrap().service_state,
            SearchProjectionServiceState::Ready
        );

        let ready_summary = run_ocr_worker_once(&data_dir, &store, &options).unwrap();
        assert_eq!(ready_summary.failed, 1);
        let attempted = store.ingest_job_by_id(&job_id).unwrap().unwrap();
        assert_eq!(attempted.status, IngestJobStatus::FailedRetryable);
        assert_eq!(attempted.attempt_count, 1);

        drop(store);
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn repair_blocked_projection_keeps_ocr_queued_across_worker_ticks() {
        let data_dir = std::env::temp_dir().join(format!(
            "resume-ir-daemon-ocr-repair-blocked-gate-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let store = open_test_store(&data_dir);
        let now = UnixTimestamp::from_unix_seconds(1_800_281_100);
        let job_id = enqueue_ocr_job_for_worker_gate(&data_dir, &store, now, "blocked");
        store
            .block_migration_rebuild(SearchRepairReason::RuntimeInvariant, now)
            .unwrap();
        let options = RunOptions {
            ocr_command: Some(data_dir.join("unused-ocr-command")),
            ..RunOptions::default()
        };

        for _ in 0..2 {
            let summary = run_ocr_worker_once(&data_dir, &store, &options).unwrap();
            assert!(!summary.has_activity());
        }

        let still_queued = store.ingest_job_by_id(&job_id).unwrap().unwrap();
        assert_eq!(still_queued.status, IngestJobStatus::Queued);
        assert_eq!(still_queued.attempt_count, 0);
        assert_eq!(
            store.search_projection_state().unwrap().service_state,
            SearchProjectionServiceState::RepairBlocked
        );

        drop(store);
        let _ = fs::remove_dir_all(data_dir);
    }

    fn enqueue_ocr_job_for_worker_gate(
        data_dir: &std::path::Path,
        store: &OwnedMetaStore,
        now: UnixTimestamp,
        fixture_id: &str,
    ) -> IngestJobId {
        let digest = ContentDigest::from_bytes(fixture_id.as_bytes());
        let document_id = DocumentId::from_non_secret_parts(&["daemon-ocr-gate", fixture_id]);
        let missing_document_path = data_dir.join(format!("synthetic-{fixture_id}-scanned.pdf"));
        store
            .upsert_document(&Document {
                id: document_id.clone(),
                source_uri: format!("synthetic://ocr-gate/{fixture_id}"),
                normalized_path: missing_document_path.to_string_lossy().into_owned(),
                file_name: format!("synthetic-{fixture_id}-scanned.pdf"),
                extension: FileExtension::Pdf,
                byte_size: 32,
                mtime: now,
                content_hash: Some(digest.as_str().to_string()),
                text_hash: None,
                is_deleted: false,
                created_at: now,
                updated_at: now,
                status: DocumentStatus::OcrRequired,
            })
            .unwrap();
        let source_revision = SourceRevision::for_content(document_id, digest, 32);
        store.insert_source_revision(&source_revision).unwrap();
        store
            .insert_source_revision_triage(&SourceRevisionTriage {
                source_revision_id: source_revision.id.clone(),
                status: ClassificationStatus::OcrBacklog,
                triage_epoch: CLASSIFIER_EPOCH.to_string(),
                reason_codes: vec![ReasonCode::OcrRequired],
                triaged_at: now,
            })
            .unwrap();
        store
            .enqueue_ocr_job_for_source_triage(
                &source_revision.id,
                CurrentClassifierEpoch::parse(CLASSIFIER_EPOCH).unwrap(),
                now,
            )
            .unwrap()
            .job
            .id
    }
}
