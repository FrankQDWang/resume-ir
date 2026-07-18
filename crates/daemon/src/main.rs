use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use embedder::{
    EmbeddingBudget, EmbeddingError, EmbeddingInput, EmbeddingPriority, LocalEmbeddingCommandSpec,
    ResidentEmbeddingClient, ResidentEmbeddingOwner, ResidentEmbeddingSpec,
};
use import_pipeline::{
    detect_ocr_page_count, import_root_with_options, index_claimed_ocr_text_with_policy,
    reconcile_search_artifacts, ImportOptions, ImportPipelineErrorClass, ImportResourcePolicy,
    ImportScanBudgetKind as PipelineImportScanBudgetKind, ImportSummary, ImportTaskOwnerLock,
    LinearPromotionPolicy, ScanProfile, SearchArtifactRecoverySummary, SearchProjectionRemoval,
    SearchProjectionRemovalReason, SearchPublicationEmbeddingFailure,
    SearchPublicationEmbeddingInput, SearchPublicationEmbeddingOutput,
    SearchPublicationVectorization, SearchPublicationVectorizer,
};
use meta_store::{
    ContactHash, DocumentId, DocumentStatus, EntityType, ImportRootKind, ImportRootPreset,
    ImportScanBudgetKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId,
    ImportTaskStatus, IndexStateStatus, IngestJobFailureKind, MetaStore, MetaStoreErrorClass,
    OcrAttemptFailure, OcrPageCacheEntry, OcrPageCacheKey, OcrPageCacheStatus, SearchFilterCase,
    SearchProjectionFilter, SearchProjectionPredicate, SearchProjectionServiceState, UnixTimestamp,
    WorkerTaskKind,
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
use privacy::redact_contact_values;
#[cfg(unix)]
use process_containment::CurrentProcessGroupLeader;
use rank_fusion::{DateRange, DegreeLevel, SchoolTier, SearchFilters};
use search_planner::plan_search;

mod detail_hydrate;
mod detail_ipc;
mod diagnostics_ipc;
mod import_root_control;
mod ipc;
mod query_runtime;
mod query_timing;
mod search_batch;
mod search_ipc;

use query_timing::{QueryStage, QueryStageTiming};

const IMPORT_RETRY_BACKOFF_SECONDS: i64 = 60;
const DEFAULT_IMPORT_RESCAN_MIN_AGE_SECONDS: i64 = 300;
const IMPORT_TASK_HEARTBEAT_SECONDS: u64 = 30;
const STALE_IMPORT_TASK_SECONDS: i64 = 15 * 60;
const STALE_INGEST_JOB_SECONDS: i64 = 15 * 60;
const IPC_MAX_REQUEST_BYTES: usize = 64 * 1024;
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
    let daemon_owner = if options.once {
        None
    } else {
        let owner_mode = match options.parent_lifecycle {
            ParentLifecycleMode::Unmanaged => ipc::OwnerMode::Standalone,
            ParentLifecycleMode::Stdin => ipc::OwnerMode::DesktopSupervised,
        };
        Some(
            match ipc::DaemonGenerationOwner::acquire(data_dir, owner_mode) {
                Ok(owner) => owner,
                Err(ipc::GenerationError::OwnershipConflict) => {
                    return Err(DaemonError::ownership_conflict());
                }
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
    let _resident_embedding_owner = start_resident_embedding(&mut options)?;

    let store = open_store(data_dir)?;
    reconcile_search_artifacts(
        data_dir,
        &store,
        current_timestamp()?,
        &options.search_vectorization,
    )
    .map_err(DaemonError::import)?;
    let summary = store.status_summary().map_err(DaemonError::store)?;

    println!("resume-daemon foreground ready");
    println!("mode: {}", if options.once { "once" } else { "foreground" });
    println!("index health: {}", index_health_label(summary.index_health));
    println!("import tasks queued: {}", summary.import_tasks_queued);
    println!("import tasks cancelled: {}", summary.import_tasks_cancelled);
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::control_plane("unable to write daemon status"))?;

    if options.work_imports_once {
        let import_summary = run_import_worker_once(data_dir, &store, &options)?;
        print_import_worker_summary(&import_summary)?;
    }
    if options.work_ocr_once {
        let ocr_summary = run_ocr_worker_once(data_dir, &store, &options)?;
        print_ocr_worker_summary(&ocr_summary)?;
    }
    if options.work_index_once {
        let recovery = run_search_artifact_worker_once(data_dir, &store, &options)?;
        print_search_artifact_worker_summary(&recovery)?;
    }

    if options.once {
        return Ok(());
    }
    if options.has_worker_loop() && options.ipc_listen.is_some() {
        run_worker_with_ipc(
            data_dir,
            &options,
            parent_shutdown.as_ref(),
            daemon_owner
                .as_ref()
                .expect("persistent daemon owns its data directory"),
        )?;
        return Ok(());
    }
    if options.has_worker_loop() {
        run_worker_loop(data_dir, &store, &options, parent_shutdown)?;
        return Ok(());
    }
    if let Some(ipc_addr) = options.ipc_listen {
        serve_ipc(
            data_dir,
            ipc_addr,
            options.max_requests,
            &options,
            parent_shutdown.as_ref(),
            daemon_owner
                .as_ref()
                .expect("persistent daemon owns its data directory"),
        )?;
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
    options: &RunOptions,
    parent_shutdown: Option<&Arc<AtomicBool>>,
    daemon_owner: &ipc::DaemonGenerationOwner,
) -> Result<()> {
    let ipc_addr = options
        .ipc_listen
        .expect("validated combined worker/ipc mode has ipc address");
    let listener = bind_ipc_listener(ipc_addr, daemon_owner)?;
    let ipc_store = open_store(data_dir)?;
    let stop_worker = parent_shutdown
        .cloned()
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let worker_stop = Arc::clone(&stop_worker);
    let worker_data_dir = data_dir.to_path_buf();
    let worker_options = options.clone();
    let (worker_result_sender, worker_result_receiver) = mpsc::channel::<Result<()>>();
    let worker_handle = thread::spawn(move || {
        let result = (|| -> Result<()> {
            let store = open_store(&worker_data_dir)?;
            run_worker_loop(&worker_data_dir, &store, &worker_options, Some(worker_stop))
        })();
        let _ = worker_result_sender.send(result);
    });

    let ipc_result = serve_ipc_listener(
        &listener,
        IpcServerContext {
            data_dir,
            store: &ipc_store,
            max_requests: options.max_requests,
            options,
            shutdown: Some(&stop_worker),
            worker_result_receiver: Some(&worker_result_receiver),
            daemon_owner,
        },
    );
    stop_worker.store(true, Ordering::Relaxed);
    worker_handle
        .join()
        .map_err(|_| DaemonError::control_plane("worker thread panicked"))?;
    ipc_result
}

fn run_worker_loop(
    data_dir: &Path,
    store: &MetaStore,
    options: &RunOptions,
    stop_signal: Option<Arc<AtomicBool>>,
) -> Result<()> {
    let interval = Duration::from_millis(options.worker_interval_ms.unwrap_or(1_000));
    let mut ticks = 0_usize;
    let mut import_watcher = if options.watch_import_roots {
        Some(ImportWatcher::new()?)
    } else {
        None
    };

    loop {
        if stop_signal
            .as_ref()
            .is_some_and(|stop| stop.load(Ordering::Relaxed))
        {
            return Ok(());
        }
        ticks += 1;
        if options.work_imports {
            let now = current_timestamp()?;
            let mut import_summary = ImportWorkerSummary {
                stale_recovered: recover_stale_import_tasks(
                    store,
                    now,
                    options
                        .stale_import_task_seconds
                        .unwrap_or(STALE_IMPORT_TASK_SECONDS),
                )?,
                ..ImportWorkerSummary::default()
            };
            if options.rescan_completed_imports {
                let min_age_seconds = if ticks == 1 {
                    0
                } else {
                    options
                        .import_rescan_min_age_seconds
                        .unwrap_or(DEFAULT_IMPORT_RESCAN_MIN_AGE_SECONDS)
                };
                import_summary.completed_requeued =
                    requeue_completed_imports(store, now, min_age_seconds)?;
            }
            if let Some(watcher) = import_watcher.as_mut() {
                import_summary.extend_watcher(watcher.sync_and_requeue(store, now)?);
            }
            import_summary.extend(run_import_worker_once_with_retry_due(
                data_dir,
                store,
                options,
                timestamp_minus_seconds(
                    now,
                    options
                        .import_retry_backoff_seconds
                        .unwrap_or(IMPORT_RETRY_BACKOFF_SECONDS),
                ),
            )?);
            if import_summary.has_activity() {
                print_import_worker_summary(&import_summary)?;
            }
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
            if ocr_summary.has_activity() {
                print_ocr_worker_summary(&ocr_summary)?;
            }
        }
        if options.work_index {
            let recovery = run_search_artifact_worker_once(data_dir, store, options)?;
            if search_artifact_recovery_has_activity(&recovery) {
                print_search_artifact_worker_summary(&recovery)?;
            }
        }
        if options
            .max_worker_ticks
            .is_some_and(|max_ticks| ticks >= max_ticks)
        {
            return Ok(());
        }
        sleep_worker_interval(interval, stop_signal.as_ref());
    }
}

fn sleep_worker_interval(interval: Duration, stop_signal: Option<&Arc<AtomicBool>>) {
    let Some(stop_signal) = stop_signal else {
        thread::sleep(interval);
        return;
    };
    let deadline = std::time::Instant::now() + interval;
    while std::time::Instant::now() < deadline {
        if stop_signal.load(Ordering::Relaxed) {
            return;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        thread::sleep(Duration::from_millis(25).min(remaining));
    }
}

fn run_import_worker_once(
    data_dir: &Path,
    store: &MetaStore,
    options: &RunOptions,
) -> Result<ImportWorkerSummary> {
    let retryable_due_at = current_timestamp()?;
    run_import_worker_once_with_retry_due(data_dir, store, options, retryable_due_at)
}

fn run_search_artifact_worker_once(
    data_dir: &Path,
    store: &MetaStore,
    options: &RunOptions,
) -> Result<SearchArtifactRecoverySummary> {
    reconcile_search_artifacts(
        data_dir,
        store,
        current_timestamp()?,
        &options.search_vectorization,
    )
    .map_err(DaemonError::import)
}

fn run_import_worker_once_with_retry_due(
    data_dir: &Path,
    store: &MetaStore,
    options: &RunOptions,
    retryable_due_at: UnixTimestamp,
) -> Result<ImportWorkerSummary> {
    let mut worker_summary = ImportWorkerSummary::default();
    let mut attempted = Vec::<ImportTaskId>::new();

    while let Some(task) = store
        .claim_next_import_task_for_worker_excluding_due_at(
            current_timestamp()?,
            retryable_due_at,
            &attempted,
        )
        .map_err(DaemonError::store)?
    {
        attempted.push(task.id.clone());
        let now = current_timestamp()?;
        let Some(scope) = store
            .import_scan_scope_by_task_id(&task.id)
            .map_err(DaemonError::store)?
        else {
            mark_import_task_failed_permanent(store, &task, now)?;
            worker_summary.failed += 1;
            continue;
        };

        let import_options = match import_options_from_scope(&scope, options) {
            Ok(import_options) => import_options,
            Err(_) => {
                mark_import_task_failed_permanent(store, &task, now)?;
                worker_summary.failed += 1;
                continue;
            }
        };
        let owner_lock = match ImportTaskOwnerLock::acquire(data_dir, &task.id) {
            Ok(owner_lock) => owner_lock,
            Err(_) => {
                let _ = store.update_import_task_status(
                    &task.id,
                    ImportTaskStatus::FailedRetryable,
                    now,
                );
                worker_summary.failed += 1;
                continue;
            }
        };
        let heartbeat = ImportTaskHeartbeat::start(data_dir, task.id.clone());
        let import_result = import_root_with_options(
            data_dir,
            store,
            &task,
            Path::new(&scope.canonical_root_path),
            now,
            import_options,
        );
        drop(heartbeat);
        drop(owner_lock);
        let import_summary = match import_result {
            Ok(import_summary) => import_summary,
            Err(error) => {
                worker_summary.failure_class = Some(error.class());
                if store
                    .is_import_task_cancelled(&task.id)
                    .map_err(DaemonError::store)?
                {
                    worker_summary.cancelled += 1;
                } else {
                    worker_summary.failed += 1;
                }
                continue;
            }
        };

        let finished_at = current_timestamp()?;
        upsert_scope_summary(store, scope, &import_summary, finished_at)?;
        worker_summary.processed += 1;
        worker_summary.searchable_documents += import_summary.searchable_documents;
        worker_summary.ocr_jobs_queued += import_summary.ocr_jobs_queued;
    }

    Ok(worker_summary)
}

fn run_ocr_worker_once(
    data_dir: &Path,
    store: &MetaStore,
    options: &RunOptions,
) -> Result<OcrWorkerSummary> {
    let now = current_timestamp()?;
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
    store: &MetaStore,
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
    store: &MetaStore,
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
        Err(error) => {
            mark_ocr_job_failed_retryable(store, job, now)?;
            return Err(DaemonError::import(error));
        }
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
        Err(error) => {
            let _ = mark_ocr_job_failed_retryable(store, job, now);
            return Err(DaemonError::import(error));
        }
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
    store: &MetaStore,
    job: &meta_store::ClaimedOcrJob,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .finish_ocr_attempt_failure(job, OcrAttemptFailure::Retryable, now)
        .map(|_| ())
        .map_err(DaemonError::store)
}

fn mark_ocr_job_failed_retryable_with_failure_kind(
    store: &MetaStore,
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
    store: &MetaStore,
    job: &meta_store::ClaimedOcrJob,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .finish_ocr_attempt_failure(job, OcrAttemptFailure::Permanent, now)
        .map(|_| ())
        .map_err(DaemonError::store)
}

fn recover_stale_ingest_jobs(store: &MetaStore, now: UnixTimestamp) -> Result<usize> {
    store
        .recover_stale_running_ingest_jobs(
            now,
            timestamp_minus_seconds(now, STALE_INGEST_JOB_SECONDS),
        )
        .map_err(DaemonError::store)
}

fn recover_stale_import_tasks(
    store: &MetaStore,
    now: UnixTimestamp,
    stale_seconds: i64,
) -> Result<usize> {
    store
        .recover_stale_running_import_tasks(now, timestamp_minus_seconds(now, stale_seconds))
        .map_err(DaemonError::store)
}

fn requeue_completed_imports(
    store: &MetaStore,
    now: UnixTimestamp,
    min_age_seconds: i64,
) -> Result<usize> {
    let due_before = timestamp_minus_seconds(now, min_age_seconds);
    let scopes = store
        .completed_import_scan_scopes_due_for_requeue(due_before)
        .map_err(DaemonError::store)?;
    let mut requeued = 0_usize;

    for (index, scope) in scopes.into_iter().enumerate() {
        let task_id = new_import_task_id(index)?;
        enqueue_import_from_completed_scope(store, scope, task_id, now)?;
        requeued += 1;
    }

    Ok(requeued)
}

fn enqueue_import_from_completed_scope(
    store: &MetaStore,
    scope: ImportScanScope,
    task_id: ImportTaskId,
    now: UnixTimestamp,
) -> Result<()> {
    let task = ImportTask {
        id: task_id.clone(),
        root_path: scope.canonical_root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let next_scope = ImportScanScope {
        import_task_id: task_id,
        root_kind: scope.root_kind,
        root_preset: scope.root_preset,
        scan_profile: scope.scan_profile,
        requested_root_path: scope.requested_root_path,
        canonical_root_path: scope.canonical_root_path,
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: scope.scan_budget_kind,
        scan_budget_limit: scope.scan_budget_limit,
        scan_budget_observed: scope.scan_budget_limit.map(|_| 0),
        scan_budget_exhausted: false,
        updated_at: now,
    };

    store
        .insert_import_task_with_scan_scope(&task, &next_scope)
        .map_err(DaemonError::store)
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
        store: &MetaStore,
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
            if store
                .pending_import_task_by_root(&root)
                .map_err(DaemonError::store)?
                .is_some()
            {
                continue;
            }
            enqueue_import_from_completed_scope(store, scope, new_import_task_id(index)?, now)?;
            summary.requeued += 1;
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

fn mark_import_task_failed_permanent(
    store: &MetaStore,
    task: &meta_store::ImportTask,
    now: UnixTimestamp,
) -> Result<()> {
    if task.status != ImportTaskStatus::Running {
        store
            .update_import_task_status(&task.id, ImportTaskStatus::Running, now)
            .map_err(DaemonError::store)?;
    }
    store
        .update_import_task_status(&task.id, ImportTaskStatus::FailedPermanent, now)
        .map_err(DaemonError::store)
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

fn upsert_scope_summary(
    store: &MetaStore,
    mut scope: ImportScanScope,
    summary: &ImportSummary,
    now: UnixTimestamp,
) -> Result<()> {
    scope.files_discovered = usize_to_u64(summary.files_discovered)?;
    scope.ignored_entries = usize_to_u64(summary.ignored_entries)?;
    scope.scan_errors = usize_to_u64(summary.scan_errors)?;
    scope.searchable_documents = usize_to_u64(summary.searchable_documents)?;
    scope.ocr_required_documents = usize_to_u64(summary.ocr_required_documents)?;
    scope.ocr_jobs_queued = usize_to_u64(summary.ocr_jobs_queued)?;
    scope.failed_documents = usize_to_u64(summary.failed_documents)?;
    scope.deleted_documents = usize_to_u64(summary.deleted_documents)?;
    scope.scan_budget_kind = summary.scan_budget.map(|budget| match budget.kind {
        PipelineImportScanBudgetKind::Files => ImportScanBudgetKind::Files,
    });
    scope.scan_budget_limit = summary
        .scan_budget
        .map(|budget| usize_to_u64(budget.limit))
        .transpose()?;
    scope.scan_budget_observed = summary
        .scan_budget
        .map(|budget| usize_to_u64(budget.observed))
        .transpose()?;
    scope.scan_budget_exhausted = summary.scan_budget.is_some_and(|budget| budget.exhausted);
    scope.updated_at = now;
    store
        .upsert_import_scan_scope(&scope)
        .map_err(DaemonError::store)
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

fn usize_to_u64(value: usize) -> Result<u64> {
    u64::try_from(value).map_err(|_| DaemonError::user("import summary count is too large"))
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

fn serve_ipc(
    data_dir: &Path,
    addr: SocketAddr,
    max_requests: Option<usize>,
    options: &RunOptions,
    shutdown: Option<&Arc<AtomicBool>>,
    daemon_owner: &ipc::DaemonGenerationOwner,
) -> Result<()> {
    let listener = bind_ipc_listener(addr, daemon_owner)?;
    let ipc_store = open_store(data_dir)?;
    serve_ipc_listener(
        &listener,
        IpcServerContext {
            data_dir,
            store: &ipc_store,
            max_requests,
            options,
            shutdown,
            worker_result_receiver: None,
            daemon_owner,
        },
    )
}

fn bind_ipc_listener(
    addr: SocketAddr,
    daemon_owner: &ipc::DaemonGenerationOwner,
) -> Result<TcpListener> {
    let listener = TcpListener::bind(addr)
        .map_err(|_| DaemonError::control_plane("unable to bind daemon ipc listener"))?;
    listener
        .set_nonblocking(true)
        .map_err(|_| DaemonError::control_plane("unable to configure daemon ipc listener"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|_| DaemonError::control_plane("unable to inspect daemon ipc listener"))?;
    daemon_owner
        .publish(local_addr)
        .map_err(|error| match error {
            ipc::GenerationError::RuntimeIntegrity => DaemonError::runtime_integrity(),
            ipc::GenerationError::OwnershipConflict => DaemonError::ownership_conflict(),
            ipc::GenerationError::Storage => {
                DaemonError::control_plane("unable to publish daemon ipc discovery")
            }
        })?;
    println!("ipc status endpoint: http://{local_addr}/status");
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::control_plane("unable to write daemon status"))?;
    Ok(listener)
}

struct IpcServerContext<'a> {
    data_dir: &'a Path,
    store: &'a MetaStore,
    max_requests: Option<usize>,
    options: &'a RunOptions,
    shutdown: Option<&'a Arc<AtomicBool>>,
    worker_result_receiver: Option<&'a Receiver<Result<()>>>,
    daemon_owner: &'a ipc::DaemonGenerationOwner,
}

fn serve_ipc_listener(listener: &TcpListener, context: IpcServerContext<'_>) -> Result<()> {
    let request_limit = context.max_requests.unwrap_or(usize::MAX);
    let mut handled_requests = 0_usize;
    let query_service = search_ipc::SearchService::start(context.data_dir, context.options)?;
    while handled_requests < request_limit {
        if context
            .shutdown
            .is_some_and(|shutdown| shutdown.load(Ordering::Acquire))
        {
            break;
        }
        if let Some(worker_result_receiver) = context.worker_result_receiver {
            match poll_import_worker(worker_result_receiver, context.shutdown)? {
                WorkerMonitorOutcome::Running => {}
                WorkerMonitorOutcome::ShutdownComplete => break,
            }
        }
        query_service.check_health()?;
        match listener.accept() {
            Ok((stream, _)) => {
                ipc::process_metrics().record_accepted();
                let connection_outcome = handle_ipc_stream(
                    context.data_dir,
                    context.store,
                    stream,
                    &query_service,
                    context.daemon_owner,
                )?;
                ipc::process_metrics().record_connection_outcome(connection_outcome);
                handled_requests += 1;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                return Err(DaemonError::control_plane(
                    "unable to accept daemon ipc request",
                ));
            }
        }
    }
    query_service.finish()
}

enum WorkerMonitorOutcome {
    Running,
    ShutdownComplete,
}

fn poll_import_worker(
    worker_result_receiver: &Receiver<Result<()>>,
    shutdown: Option<&Arc<AtomicBool>>,
) -> Result<WorkerMonitorOutcome> {
    match worker_result_receiver.try_recv() {
        Ok(Ok(())) if shutdown.is_some_and(|shutdown| shutdown.load(Ordering::Acquire)) => {
            Ok(WorkerMonitorOutcome::ShutdownComplete)
        }
        Ok(Ok(())) => Err(DaemonError::control_plane(
            "import worker exited while daemon ipc was still running",
        )),
        Ok(Err(error)) => Err(error),
        Err(TryRecvError::Disconnected) => Err(DaemonError::control_plane(
            "import worker thread stopped unexpectedly",
        )),
        Err(TryRecvError::Empty) => Ok(WorkerMonitorOutcome::Running),
    }
}

fn handle_ipc_stream(
    data_dir: &Path,
    ipc_store: &MetaStore,
    stream: TcpStream,
    query_service: &search_ipc::SearchService,
    daemon_owner: &ipc::DaemonGenerationOwner,
) -> Result<ipc::ConnectionOutcome> {
    match handle_ipc_request(data_dir, ipc_store, stream, query_service, daemon_owner) {
        Ok(()) => Ok(ipc::ConnectionOutcome::Completed),
        Err(error) => match error.into_request_failure() {
            Ok(error) => Ok(ipc::ConnectionOutcome::from_request_result(Err(error))),
            Err(fatal) => Err(fatal),
        },
    }
}

fn handle_ipc_request(
    data_dir: &Path,
    ipc_store: &MetaStore,
    mut stream: TcpStream,
    query_service: &search_ipc::SearchService,
    daemon_owner: &ipc::DaemonGenerationOwner,
) -> Result<()> {
    stream
        .set_nonblocking(false)
        .map_err(|_| DaemonError::user("unable to configure daemon ipc stream"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| DaemonError::user("unable to set daemon ipc timeout"))?;
    ipc::response::configure(&stream).map_err(DaemonError::response_sink)?;
    let request = match read_ipc_request(&mut stream)? {
        IpcReadOutcome::Request(request) => request,
        IpcReadOutcome::TooLarge => {
            return write_http_response(&mut stream, 413, "text/plain", "request too large")
        }
        IpcReadOutcome::BadRequest => {
            return write_http_response(&mut stream, 400, "text/plain", "bad request")
        }
    };

    if request.method == "GET"
        && request.path == "/status"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        let body = status_json(ipc_store);
        return write_http_response(&mut stream, 200, "application/json", &body);
    }

    if request.method == "GET"
        && request.path == "/diagnostics"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_diagnostics_ipc(ipc_store, daemon_owner.auth_token(), &request, &mut stream);
    }

    if request.method == "POST"
        && request.path == "/imports"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_import_command_ipc(
            data_dir,
            daemon_owner.auth_token(),
            &request,
            &mut stream,
        );
    }

    if request.method == "POST"
        && request.path == "/imports/cancel"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_import_cancel_command_ipc(
            data_dir,
            daemon_owner.auth_token(),
            &request,
            &mut stream,
        );
    }

    if request.method == "POST"
        && request.path == "/imports/control"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return import_root_control::handle_ipc(
            data_dir,
            daemon_owner.auth_token(),
            &request,
            &mut stream,
        );
    }

    if request.method == "GET"
        && request.path == "/imports/progress"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_import_progress_stream_ipc(
            data_dir,
            daemon_owner.auth_token(),
            &request,
            &mut stream,
        );
    }

    if request.method == "POST"
        && request.path == "/search"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_search_command_ipc(
            ipc_store,
            daemon_owner.auth_token(),
            &request,
            stream,
            query_service,
        );
    }

    if request.method == "POST"
        && request.path == "/search/batch"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_search_batch_ipc(
            ipc_store,
            daemon_owner.auth_token(),
            &request,
            stream,
            query_service,
        );
    }

    if request.method == "POST"
        && request.path == "/search/cancel"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_search_cancel_ipc(
            daemon_owner.auth_token(),
            &request,
            stream,
            query_service,
        );
    }

    if request.method == "POST"
        && request.path == "/details"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_detail_command_ipc(
            ipc_store,
            daemon_owner.auth_token(),
            &request,
            &mut stream,
        );
    }

    if request.method == "POST"
        && request.path == "/details/hydrate"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_detail_hydrate_command_ipc(
            ipc_store,
            daemon_owner.auth_token(),
            &request,
            &mut stream,
        );
    }

    if request.method == "POST"
        && request.path == "/delete"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_delete_command_ipc(
            data_dir,
            daemon_owner.auth_token(),
            &request,
            &mut stream,
        );
    }

    write_http_response(&mut stream, 404, "text/plain", "not found")
}

fn query_service_error(store: &MetaStore) -> Option<ipc::ServiceErrorCode> {
    match store.search_projection_state() {
        Ok(state) => projection_query_error(Some(state.service_state)),
        Err(_) => projection_query_error(None),
    }
}

fn projection_query_error(
    state: Option<SearchProjectionServiceState>,
) -> Option<ipc::ServiceErrorCode> {
    match state {
        Some(SearchProjectionServiceState::Ready) => None,
        Some(SearchProjectionServiceState::Repairing) => Some(ipc::ServiceErrorCode::Repairing),
        Some(SearchProjectionServiceState::RepairBlocked) => {
            Some(ipc::ServiceErrorCode::QueryServiceUnavailable)
        }
        None => Some(ipc::ServiceErrorCode::MetadataUnavailable),
    }
}

fn read_ipc_request(stream: &mut TcpStream) -> Result<IpcReadOutcome> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];
    let header_end = loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(_) => return Ok(IpcReadOutcome::BadRequest),
        };
        if read == 0 {
            break None;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > IPC_MAX_REQUEST_BYTES {
            return Ok(IpcReadOutcome::TooLarge);
        }
        if let Some(header_end) = find_http_header_end(&request) {
            break Some(header_end);
        }
    };

    let Some(header_end) = header_end else {
        return Ok(IpcReadOutcome::Request(IpcRequest::empty()));
    };
    let Ok(header_text) = std::str::from_utf8(&request[..header_end]) else {
        return Ok(IpcReadOutcome::BadRequest);
    };
    let mut lines = header_text.lines();
    let first_line = lines.next().unwrap_or_default();
    let mut first_line_parts = first_line.split_whitespace();
    let method = first_line_parts.next().unwrap_or_default().to_string();
    let path = first_line_parts.next().unwrap_or_default().to_string();
    let version = first_line_parts.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    let content_length = match header_value(&headers, "content-length") {
        Some(value) => {
            let Some(content_length) = parse_content_length(value) else {
                return Ok(IpcReadOutcome::BadRequest);
            };
            content_length
        }
        None => 0,
    };
    let Some(request_end) = header_end.checked_add(content_length) else {
        return Ok(IpcReadOutcome::TooLarge);
    };

    if request_end > IPC_MAX_REQUEST_BYTES {
        return Ok(IpcReadOutcome::TooLarge);
    }

    while request.len() < request_end {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(_) => return Ok(IpcReadOutcome::BadRequest),
        };
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > IPC_MAX_REQUEST_BYTES {
            return Ok(IpcReadOutcome::TooLarge);
        }
    }

    if request.len() < request_end {
        return Ok(IpcReadOutcome::BadRequest);
    }
    let body = request[header_end..request_end].to_vec();

    Ok(IpcReadOutcome::Request(IpcRequest {
        method,
        path,
        version,
        headers,
        body,
    }))
}

fn find_http_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn parse_content_length(value: &str) -> Option<usize> {
    value.parse::<usize>().ok()
}

fn handle_import_command_ipc(
    data_dir: &Path,
    auth_token: &str,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(auth_token, &request.headers) {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(stream, 401, "application/json", &body);
    }

    match enqueue_import_command(data_dir, &request.body) {
        Ok(body) => write_http_response(stream, 202, "application/json", &body),
        Err(IpcCommandError::BadRequest(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "bad_request",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 400, "application/json", &body)
        }
        Err(IpcCommandError::Conflict(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "conflict",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 409, "application/json", &body)
        }
        Err(IpcCommandError::NotFound(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "not_found",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 404, "application/json", &body)
        }
        Err(IpcCommandError::TooLarge(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "too_large",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 413, "application/json", &body)
        }
        Err(IpcCommandError::ServiceUnavailable(_)) => {
            write_service_unavailable(stream, ipc::ServiceErrorCode::QueryServiceUnavailable)
        }
        Err(IpcCommandError::Internal(_error)) => {
            write_service_unavailable(stream, ipc::ServiceErrorCode::MetadataUnavailable)
        }
    }
}

fn handle_import_cancel_command_ipc(
    data_dir: &Path,
    auth_token: &str,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(auth_token, &request.headers) {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(stream, 401, "application/json", &body);
    }

    match cancel_import_command(data_dir, &request.body) {
        Ok(body) => write_http_response(stream, 202, "application/json", &body),
        Err(IpcCommandError::BadRequest(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "bad_request",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 400, "application/json", &body)
        }
        Err(IpcCommandError::Conflict(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "conflict",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 409, "application/json", &body)
        }
        Err(IpcCommandError::NotFound(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "not_found",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 404, "application/json", &body)
        }
        Err(IpcCommandError::TooLarge(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "too_large",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 413, "application/json", &body)
        }
        Err(IpcCommandError::ServiceUnavailable(_)) => {
            write_service_unavailable(stream, ipc::ServiceErrorCode::QueryServiceUnavailable)
        }
        Err(IpcCommandError::Internal(_error)) => {
            write_service_unavailable(stream, ipc::ServiceErrorCode::MetadataUnavailable)
        }
    }
}

fn handle_import_progress_stream_ipc(
    data_dir: &Path,
    auth_token: &str,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(auth_token, &request.headers) {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(stream, 401, "application/json", &body);
    }

    let first_event = match import_progress_stream_event_json(data_dir) {
        Ok(event) => event,
        Err(_) => {
            return write_service_unavailable(stream, ipc::ServiceErrorCode::MetadataUnavailable);
        }
    };
    ipc::response::write_all(
        stream,
        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n",
    )
    .map_err(DaemonError::response_sink)?;
    for event_index in 0..IMPORT_PROGRESS_STREAM_EVENTS {
        let event = if event_index == 0 {
            first_event.clone()
        } else {
            import_progress_stream_event_json(data_dir)?
        };
        ipc::response::write_all(stream, event.as_bytes())
            .and_then(|_| ipc::response::write_all(stream, b"\n"))
            .and_then(|_| ipc::response::flush(stream))
            .map_err(DaemonError::response_sink)?;
        if event_index + 1 < IMPORT_PROGRESS_STREAM_EVENTS {
            thread::sleep(Duration::from_millis(IMPORT_PROGRESS_STREAM_INTERVAL_MS));
        }
    }
    Ok(())
}

fn ipc_command_authorized(expected: &str, headers: &[(String, String)]) -> bool {
    let Some(header) = header_value(headers, "authorization") else {
        return false;
    };
    let Some(token) = header.strip_prefix("Bearer ") else {
        return false;
    };

    constant_time_eq(token.trim().as_bytes(), expected.as_bytes())
}

fn handle_diagnostics_ipc(
    store: &MetaStore,
    auth_token: &str,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(auth_token, &request.headers) {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(stream, 401, "application/json", &body);
    }
    let body = diagnostics_ipc::render(store);
    write_http_response(stream, 200, "application/json", &body)
}

fn handle_search_command_ipc(
    store: &MetaStore,
    auth_token: &str,
    request: &IpcRequest,
    mut stream: TcpStream,
    query_service: &search_ipc::SearchService,
) -> Result<()> {
    if !ipc_command_authorized(auth_token, &request.headers) {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(&mut stream, 401, "application/json", &body);
    }

    let request_started = Instant::now();
    let envelope = match search_ipc::parse_request(&request.body) {
        Ok(envelope) => envelope,
        Err(message) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "bad_request",
                "message": message,
            })
            .to_string();
            return write_http_response(&mut stream, 400, "application/json", &body);
        }
    };
    let query_parse_started = Instant::now();
    let args = match parse_search_command(&envelope.payload) {
        Ok(args) => args,
        Err(IpcCommandError::BadRequest(message)) => {
            return write_search_command_error(
                &mut stream,
                &envelope.request_id,
                400,
                "BAD_REQUEST",
                message,
            );
        }
        Err(_) => {
            return write_search_command_error(
                &mut stream,
                &envelope.request_id,
                500,
                "INTERNAL",
                "search request validation failed",
            );
        }
    };
    if let Some(code) = query_service_error(store) {
        return write_search_command_error(
            &mut stream,
            &envelope.request_id,
            503,
            code.label(),
            "search service is unavailable",
        );
    }
    let query_parse_duration = query_parse_started.elapsed();
    query_service.dispatch(
        stream,
        envelope,
        args,
        query_parse_duration,
        request_started,
    )
}

fn handle_search_cancel_ipc(
    auth_token: &str,
    request: &IpcRequest,
    mut stream: TcpStream,
    query_service: &search_ipc::SearchService,
) -> Result<()> {
    if !ipc_command_authorized(auth_token, &request.headers) {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(&mut stream, 401, "application/json", &body);
    }
    let cancel_request = match search_ipc::parse_cancel_request(&request.body) {
        Ok(request) => request,
        Err(message) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "bad_request",
                "message": message,
            })
            .to_string();
            return write_http_response(&mut stream, 400, "application/json", &body);
        }
    };
    query_service.cancel(stream, cancel_request)
}

fn handle_search_batch_ipc(
    store: &MetaStore,
    auth_token: &str,
    request: &IpcRequest,
    mut stream: TcpStream,
    query_service: &search_ipc::SearchService,
) -> Result<()> {
    if !ipc_command_authorized(auth_token, &request.headers) {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(&mut stream, 401, "application/json", &body);
    }

    let request_started = Instant::now();
    let batch = match search_batch::parse_request(&request.body) {
        Ok(batch) => batch,
        Err(message) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "bad_request",
                "message": message,
            })
            .to_string();
            return write_http_response(&mut stream, 400, "application/json", &body);
        }
    };
    let mut children = Vec::with_capacity(batch.requests.len());
    for envelope in batch.requests {
        let query_parse_started = Instant::now();
        let args = match parse_search_command(&envelope.payload) {
            Ok(args) => args,
            Err(IpcCommandError::BadRequest(message)) => {
                let body = serde_json::json!({
                    "schema_version": "daemon.error.v1",
                    "status": "bad_request",
                    "message": message,
                })
                .to_string();
                return write_http_response(&mut stream, 400, "application/json", &body);
            }
            Err(_) => {
                let body = serde_json::json!({
                    "schema_version": "daemon.error.v1",
                    "status": "internal",
                })
                .to_string();
                return write_http_response(&mut stream, 500, "application/json", &body);
            }
        };
        children.push((envelope, args, query_parse_started.elapsed()));
    }
    if let Some(code) = query_service_error(store) {
        let body = search_ipc::error_body(
            &batch.batch_id,
            code.label(),
            "search service is unavailable",
        );
        return write_http_response(&mut stream, 503, "application/json", &body);
    }
    let Some(admission) = query_service.acquire_batch() else {
        let body = search_batch::overload_body(&batch.batch_id);
        return write_http_response(&mut stream, 503, "application/json", &body);
    };
    let writer =
        search_batch::BatchWriter::start(stream, batch.batch_id, children.len(), admission)?;
    for (sequence, (envelope, args, query_parse_duration)) in children.into_iter().enumerate() {
        query_service.dispatch_batch_child(
            writer.child(sequence, envelope.request_id.clone()),
            envelope,
            args,
            query_parse_duration,
            request_started,
        )?;
    }
    Ok(())
}

fn write_search_command_error(
    stream: &mut TcpStream,
    request_id: &str,
    status_code: u16,
    code: &str,
    message: &str,
) -> Result<()> {
    let body = search_ipc::error_body(request_id, code, message);
    write_http_response(stream, status_code, "application/json", &body)
}

fn handle_detail_command_ipc(
    store: &MetaStore,
    auth_token: &str,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    let request_id = detail_ipc::request_id(&request.body);
    if !ipc_command_authorized(auth_token, &request.headers) {
        return write_detail_error(
            stream,
            request_id.as_deref(),
            401,
            "UNAUTHORIZED",
            "authenticate",
        );
    }

    match detail_ipc::execute(store, &request.body) {
        Ok(body) => write_http_response(stream, 200, "application/json", &body),
        Err(detail_ipc::DetailError::BadRequest) => write_detail_error(
            stream,
            request_id.as_deref(),
            400,
            "BAD_REQUEST",
            "correct_request",
        ),
        Err(detail_ipc::DetailError::StaleSelection) => write_detail_error(
            stream,
            request_id.as_deref(),
            409,
            "STALE_SELECTION",
            "refresh_search",
        ),
        Err(detail_ipc::DetailError::NotFound) => write_detail_error(
            stream,
            request_id.as_deref(),
            404,
            "NOT_FOUND",
            "refresh_search",
        ),
        Err(detail_ipc::DetailError::ResponseTooLarge) => write_detail_error(
            stream,
            request_id.as_deref(),
            413,
            "RESPONSE_TOO_LARGE",
            "reduce_page_size",
        ),
        Err(detail_ipc::DetailError::Repairing) => write_detail_service_unavailable(
            stream,
            request_id.as_deref(),
            ipc::ServiceErrorCode::Repairing,
        ),
        Err(detail_ipc::DetailError::QueryServiceUnavailable) => write_detail_service_unavailable(
            stream,
            request_id.as_deref(),
            ipc::ServiceErrorCode::QueryServiceUnavailable,
        ),
        Err(detail_ipc::DetailError::MetadataUnavailable) => write_detail_service_unavailable(
            stream,
            request_id.as_deref(),
            ipc::ServiceErrorCode::MetadataUnavailable,
        ),
    }
}

fn handle_detail_hydrate_command_ipc(
    store: &MetaStore,
    auth_token: &str,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    let request_id = detail_ipc::request_id(&request.body);
    if !ipc_command_authorized(auth_token, &request.headers) {
        return write_detail_error(
            stream,
            request_id.as_deref(),
            401,
            "UNAUTHORIZED",
            "authenticate",
        );
    }

    match detail_hydrate::execute(store, &request.body) {
        Ok(body) => write_http_response(stream, 200, "application/json", &body),
        Err(detail_hydrate::DetailHydrateError::BadRequest) => write_detail_error(
            stream,
            request_id.as_deref(),
            400,
            "BAD_REQUEST",
            "correct_request",
        ),
        Err(detail_hydrate::DetailHydrateError::StaleSelection) => write_detail_error(
            stream,
            request_id.as_deref(),
            409,
            "STALE_SELECTION",
            "refresh_search",
        ),
        Err(detail_hydrate::DetailHydrateError::NotFound) => write_detail_error(
            stream,
            request_id.as_deref(),
            404,
            "NOT_FOUND",
            "refresh_search",
        ),
        Err(detail_hydrate::DetailHydrateError::ResponseTooLarge) => write_detail_error(
            stream,
            request_id.as_deref(),
            413,
            "RESPONSE_TOO_LARGE",
            "reduce_page_size",
        ),
        Err(detail_hydrate::DetailHydrateError::Repairing) => write_detail_service_unavailable(
            stream,
            request_id.as_deref(),
            ipc::ServiceErrorCode::Repairing,
        ),
        Err(detail_hydrate::DetailHydrateError::QueryServiceUnavailable) => {
            write_detail_service_unavailable(
                stream,
                request_id.as_deref(),
                ipc::ServiceErrorCode::QueryServiceUnavailable,
            )
        }
        Err(detail_hydrate::DetailHydrateError::MetadataUnavailable) => {
            write_detail_service_unavailable(
                stream,
                request_id.as_deref(),
                ipc::ServiceErrorCode::MetadataUnavailable,
            )
        }
    }
}

fn write_detail_error(
    stream: &mut TcpStream,
    request_id: Option<&str>,
    status_code: u16,
    code: &'static str,
    action: &'static str,
) -> Result<()> {
    let body = unified_error_body(request_id, code, action);
    write_http_response(stream, status_code, "application/json", &body)
}

fn write_detail_service_unavailable(
    stream: &mut TcpStream,
    request_id: Option<&str>,
    code: ipc::ServiceErrorCode,
) -> Result<()> {
    let body = unified_error_body(request_id, code.label(), code.action());
    write_http_response(stream, 503, "application/json", &body)
}

fn handle_delete_command_ipc(
    data_dir: &Path,
    auth_token: &str,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(auth_token, &request.headers) {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(stream, 401, "application/json", &body);
    }

    match execute_delete_command(data_dir, &request.body) {
        Ok(body) => write_http_response(stream, 200, "application/json", &body),
        Err(IpcCommandError::BadRequest(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "bad_request",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 400, "application/json", &body)
        }
        Err(IpcCommandError::Conflict(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "conflict",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 409, "application/json", &body)
        }
        Err(IpcCommandError::NotFound(_message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "not_found",
            })
            .to_string();
            write_http_response(stream, 404, "application/json", &body)
        }
        Err(IpcCommandError::TooLarge(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "too_large",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 413, "application/json", &body)
        }
        Err(IpcCommandError::ServiceUnavailable(_)) => {
            write_service_unavailable(stream, ipc::ServiceErrorCode::QueryServiceUnavailable)
        }
        Err(IpcCommandError::Internal(_error)) => {
            write_service_unavailable(stream, ipc::ServiceErrorCode::MetadataUnavailable)
        }
    }
}

struct DaemonSearchOutput {
    body: String,
    stage_timing: QueryStageTiming,
    cancelled: bool,
}

struct DaemonSearchExecution<'a> {
    request_id: &'a str,
    args: &'a DaemonSearchArgs,
    query_parse_duration: Duration,
    deadline: &'a search_ipc::RequestDeadline,
    cancellation: &'a search_ipc::RequestCancellation,
}

fn execute_search_command(
    store: &MetaStore,
    execution: &DaemonSearchExecution<'_>,
    options: &RunOptions,
    query_runtime: &mut query_runtime::DaemonQueryRuntime,
) -> std::result::Result<DaemonSearchOutput, IpcCommandError> {
    let args = execution.args;
    let deadline = execution.deadline;
    let mut stage_timing = QueryStageTiming::default();
    stage_timing.record_duration(QueryStage::QueryParse, execution.query_parse_duration);
    if execution.cancellation.is_cancelled() {
        return Ok(daemon_search_cancelled_output(
            execution.request_id,
            0,
            args.mode,
            deadline.elapsed(),
            execution.query_parse_duration,
        ));
    }
    if deadline.expired() {
        return Ok(daemon_search_deadline_output(
            execution.request_id,
            0,
            args.mode,
            deadline.elapsed(),
            stage_timing,
            Vec::new(),
        ));
    }
    let query_started = Instant::now();
    let outcome = query_runtime
        .execute(
            args,
            options,
            deadline,
            execution.cancellation,
            &mut stage_timing,
        )
        .map_err(map_query_failure)?;
    match outcome {
        query_runtime::SearchExecutionOutcome::Complete(search) => {
            record_daemon_query_observation(
                store,
                args.mode,
                query_started.elapsed(),
                search.hits.len(),
            );
            Ok(DaemonSearchOutput {
                body: daemon_search_ok_body(
                    execution.request_id,
                    search.visible_epoch,
                    args.mode,
                    deadline.elapsed(),
                    &stage_timing,
                    &search.hits,
                    search.partial_reasons,
                ),
                stage_timing,
                cancelled: false,
            })
        }
        query_runtime::SearchExecutionOutcome::Cancelled { visible_epoch } => {
            Ok(daemon_search_cancelled_output(
                execution.request_id,
                visible_epoch,
                args.mode,
                deadline.elapsed(),
                execution.query_parse_duration,
            ))
        }
        query_runtime::SearchExecutionOutcome::DeadlineExceeded(search) => {
            Ok(daemon_search_deadline_output(
                execution.request_id,
                search.visible_epoch,
                args.mode,
                deadline.elapsed(),
                stage_timing,
                search.hits,
            ))
        }
    }
}

fn map_query_failure(error: query_runtime::QueryFailure) -> IpcCommandError {
    match error {
        query_runtime::QueryFailure::BadRequest => {
            IpcCommandError::BadRequest("semantic query configuration is invalid")
        }
        query_runtime::QueryFailure::SelectionTooLarge => {
            IpcCommandError::TooLarge("search filter selection exceeds the bounded limit")
        }
        query_runtime::QueryFailure::SemanticDisabled => {
            IpcCommandError::ServiceUnavailable("SEMANTIC_DISABLED")
        }
        query_runtime::QueryFailure::Integrity | query_runtime::QueryFailure::Unavailable => {
            IpcCommandError::ServiceUnavailable("QUERY_SERVICE_UNAVAILABLE")
        }
    }
}

fn daemon_search_ok_body(
    request_id: &str,
    visible_epoch: u64,
    mode: DaemonSearchMode,
    elapsed: Duration,
    stage_timing: &QueryStageTiming,
    hits: &[query_runtime::SearchHit],
    partial_reasons: Vec<&'static str>,
) -> String {
    let results = hits
        .iter()
        .map(|hit| {
            serde_json::json!({
                "rank": hit.rank,
                "selection": {
                    "doc_id": hit.selection.document_id.as_str(),
                    "version_id": hit.selection.resume_version_id.as_str(),
                    "visible_epoch": hit.selection.visible_epoch,
                },
                "file_name": hit.file_name,
                "snippet": hit.snippet,
            })
        })
        .collect::<Vec<_>>();
    search_ipc::response_body(search_ipc::SearchResponse {
        request_id: request_id.to_string(),
        status: "ok",
        visible_epoch,
        query_mode: mode.response_label(),
        partial_reasons,
        latency_ms: elapsed.as_secs_f64() * 1_000.0,
        stage_latency_ms: search_stage_latency_json(stage_timing),
        search_index: "available",
        results,
    })
}

fn daemon_search_deadline_output(
    request_id: &str,
    visible_epoch: u64,
    mode: DaemonSearchMode,
    elapsed: Duration,
    stage_timing: QueryStageTiming,
    hits: Vec<query_runtime::SearchHit>,
) -> DaemonSearchOutput {
    let body = daemon_search_ok_body(
        request_id,
        visible_epoch,
        mode,
        elapsed,
        &stage_timing,
        &hits,
        vec!["deadline_exceeded"],
    );
    DaemonSearchOutput {
        body,
        stage_timing,
        cancelled: false,
    }
}

fn daemon_search_cancelled_output(
    request_id: &str,
    visible_epoch: u64,
    mode: DaemonSearchMode,
    elapsed: Duration,
    query_parse_duration: Duration,
) -> DaemonSearchOutput {
    let mut stage_timing = QueryStageTiming::default();
    stage_timing.record_duration(QueryStage::QueryParse, query_parse_duration);
    let body = search_ipc::response_body(search_ipc::SearchResponse {
        request_id: request_id.to_string(),
        status: "cancelled",
        visible_epoch,
        query_mode: mode.response_label(),
        partial_reasons: Vec::new(),
        latency_ms: elapsed.as_secs_f64() * 1_000.0,
        stage_latency_ms: search_stage_latency_json(&stage_timing),
        search_index: "not_observed",
        results: Vec::new(),
    });
    DaemonSearchOutput {
        body,
        stage_timing,
        cancelled: true,
    }
}

fn search_stage_latency_json(stage_timing: &QueryStageTiming) -> serde_json::Value {
    serde_json::json!({
        "parse": stage_timing.duration_ms(QueryStage::QueryParse),
        "prefilter": stage_timing.duration_ms(QueryStage::Prefilter),
        "bm25": stage_timing.duration_ms(QueryStage::Bm25),
        "ann": stage_timing.duration_ms(QueryStage::Ann),
        "fusion": stage_timing.duration_ms(QueryStage::Fusion),
        "bulk_hydrate": stage_timing.duration_ms(QueryStage::BulkHydrate),
        "snippet": stage_timing.duration_ms(QueryStage::Snippet),
    })
}

fn parse_search_command(
    payload: &serde_json::Value,
) -> std::result::Result<DaemonSearchArgs, IpcCommandError> {
    let object = payload.as_object().ok_or(IpcCommandError::BadRequest(
        "search payload must be an object",
    ))?;
    const ALLOWED_FIELDS: &[&str] = &["query", "mode", "top_k", "filters"];
    if object
        .keys()
        .any(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
    {
        return Err(IpcCommandError::BadRequest(
            "search payload contains an unknown field",
        ));
    }
    let query = payload
        .get("query")
        .and_then(serde_json::Value::as_str)
        .filter(|query| !query.trim().is_empty())
        .ok_or(IpcCommandError::BadRequest(
            "query must be a non-empty string",
        ))?
        .to_string();
    let mode = payload
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("fulltext");
    let mode =
        DaemonSearchMode::parse(mode).ok_or(IpcCommandError::BadRequest("mode is invalid"))?;
    let top_k = match payload.get("top_k") {
        Some(value) => value
            .as_u64()
            .filter(|value| *value > 0)
            .and_then(|value| usize::try_from(value).ok())
            .map(|value| value.min(100))
            .ok_or(IpcCommandError::BadRequest("top_k must be positive"))?,
        None => 10,
    };
    let query = plan_search(&query, top_k)
        .map_err(|_| IpcCommandError::BadRequest("query is outside semantic bounds"))?
        .query_text()
        .to_string();
    let filter = parse_search_filters(payload.get("filters"))?;
    Ok(DaemonSearchArgs {
        query,
        mode,
        top_k,
        filter,
    })
}

fn parse_search_filters(
    filters: Option<&serde_json::Value>,
) -> std::result::Result<SearchProjectionFilter, IpcCommandError> {
    let Some(filters) = filters else {
        return Ok(SearchProjectionFilter::default());
    };
    if filters.is_null() {
        return Ok(SearchProjectionFilter::default());
    }
    let Some(object) = filters.as_object() else {
        return Err(IpcCommandError::BadRequest("filters must be an object"));
    };

    const ALLOWED_FIELDS: &[&str] = &[
        "degree_min",
        "skills_any",
        "contact_hashes_any",
        "school_tiers_any",
        "names_any",
        "schools_any",
        "majors_any",
        "certificates_any",
        "date_range_overlaps",
        "companies_any",
        "titles_any",
        "locations_any",
        "years_experience_min",
    ];
    if object
        .keys()
        .any(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
    {
        return Err(IpcCommandError::BadRequest(
            "filters contain an unknown field",
        ));
    }

    let mut parsed = SearchFilters::default();
    if let Some(value) = object.get("degree_min") {
        if !value.is_null() {
            let degree = value
                .as_str()
                .and_then(DegreeLevel::parse)
                .ok_or(IpcCommandError::BadRequest("degree_min is invalid"))?;
            parsed = parsed.with_degree_min(degree);
        }
    }
    if let Some(value) = object.get("skills_any") {
        if !value.is_null() {
            let skills = value
                .as_array()
                .ok_or(IpcCommandError::BadRequest("skills_any must be an array"))?;
            if skills.len() > 64 {
                return Err(IpcCommandError::BadRequest("too many skills"));
            }
            let skills = skills
                .iter()
                .map(|skill| {
                    skill
                        .as_str()
                        .filter(|skill| !skill.trim().is_empty())
                        .ok_or(IpcCommandError::BadRequest("skills_any must be strings"))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_skills_any(skills);
        }
    }
    if let Some(value) = object.get("contact_hashes_any") {
        if !value.is_null() {
            let contact_hashes = value.as_array().ok_or(IpcCommandError::BadRequest(
                "contact_hashes_any must be an array",
            ))?;
            if contact_hashes.len() > 64 {
                return Err(IpcCommandError::BadRequest("too many contact hashes"));
            }
            let contact_hashes = contact_hashes
                .iter()
                .map(|contact_hash| {
                    let contact_hash = contact_hash.as_str().ok_or(IpcCommandError::BadRequest(
                        "contact_hashes_any values must be strings",
                    ))?;
                    ContactHash::from_keyed_digest(contact_hash.to_string())
                        .map(|hash| hash.as_str().to_string())
                        .map_err(|_| {
                            IpcCommandError::BadRequest(
                                "contact_hashes_any values must be contact hashes",
                            )
                        })
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_contact_hashes_any(contact_hashes);
        }
    }
    if let Some(value) = object.get("school_tiers_any") {
        if !value.is_null() {
            let school_tiers = value.as_array().ok_or(IpcCommandError::BadRequest(
                "school_tiers_any must be an array",
            ))?;
            if school_tiers.len() > 16 {
                return Err(IpcCommandError::BadRequest("too many school tiers"));
            }
            let school_tiers = school_tiers
                .iter()
                .map(|school_tier| {
                    school_tier.as_str().and_then(SchoolTier::parse).ok_or(
                        IpcCommandError::BadRequest("school_tiers_any values are invalid"),
                    )
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_school_tiers_any(school_tiers);
        }
    }
    if let Some(value) = object.get("names_any") {
        if !value.is_null() {
            let names = value
                .as_array()
                .ok_or(IpcCommandError::BadRequest("names_any must be an array"))?;
            if names.len() > 64 {
                return Err(IpcCommandError::BadRequest("too many names"));
            }
            let names = names
                .iter()
                .map(|name| {
                    name.as_str().ok_or(IpcCommandError::BadRequest(
                        "names_any values must be strings",
                    ))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_names_any(names);
        }
    }
    if let Some(value) = object.get("schools_any") {
        if !value.is_null() {
            let schools = value
                .as_array()
                .ok_or(IpcCommandError::BadRequest("schools_any must be an array"))?;
            if schools.len() > 64 {
                return Err(IpcCommandError::BadRequest("too many schools"));
            }
            let schools = schools
                .iter()
                .map(|school| {
                    school.as_str().ok_or(IpcCommandError::BadRequest(
                        "schools_any values must be strings",
                    ))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_schools_any(schools);
        }
    }
    if let Some(value) = object.get("majors_any") {
        if !value.is_null() {
            let majors = value
                .as_array()
                .ok_or(IpcCommandError::BadRequest("majors_any must be an array"))?;
            if majors.len() > 64 {
                return Err(IpcCommandError::BadRequest("too many majors"));
            }
            let majors = majors
                .iter()
                .map(|major| {
                    major.as_str().ok_or(IpcCommandError::BadRequest(
                        "majors_any values must be strings",
                    ))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_majors_any(majors);
        }
    }
    if let Some(value) = object.get("certificates_any") {
        if !value.is_null() {
            let certificates = value.as_array().ok_or(IpcCommandError::BadRequest(
                "certificates_any must be an array",
            ))?;
            if certificates.len() > 32 {
                return Err(IpcCommandError::BadRequest("too many certificates"));
            }
            let certificates = certificates
                .iter()
                .map(|certificate| {
                    certificate.as_str().ok_or(IpcCommandError::BadRequest(
                        "certificates_any values must be strings",
                    ))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_certificates_any(certificates);
        }
    }
    if let Some(value) = object.get("date_range_overlaps") {
        if !value.is_null() {
            let date_range =
                value
                    .as_str()
                    .and_then(DateRange::parse)
                    .ok_or(IpcCommandError::BadRequest(
                        "date_range_overlaps is invalid",
                    ))?;
            parsed = parsed.with_date_range_overlaps(&date_range.canonical());
        }
    }
    if let Some(value) = object.get("companies_any") {
        if !value.is_null() {
            let companies = value.as_array().ok_or(IpcCommandError::BadRequest(
                "companies_any must be an array",
            ))?;
            if companies.len() > 64 {
                return Err(IpcCommandError::BadRequest("too many companies"));
            }
            let companies = companies
                .iter()
                .map(|company| {
                    company.as_str().ok_or(IpcCommandError::BadRequest(
                        "companies_any values must be strings",
                    ))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_companies_any(companies);
        }
    }
    if let Some(value) = object.get("titles_any") {
        if !value.is_null() {
            let titles = value
                .as_array()
                .ok_or(IpcCommandError::BadRequest("titles_any must be an array"))?;
            if titles.len() > 64 {
                return Err(IpcCommandError::BadRequest("too many titles"));
            }
            let titles = titles
                .iter()
                .map(|title| {
                    title.as_str().ok_or(IpcCommandError::BadRequest(
                        "titles_any values must be strings",
                    ))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_titles_any(titles);
        }
    }
    if let Some(value) = object.get("locations_any") {
        if !value.is_null() {
            let locations = value.as_array().ok_or(IpcCommandError::BadRequest(
                "locations_any must be an array",
            ))?;
            if locations.len() > 64 {
                return Err(IpcCommandError::BadRequest("too many locations"));
            }
            let locations = locations
                .iter()
                .map(|location| {
                    location.as_str().ok_or(IpcCommandError::BadRequest(
                        "locations_any values must be strings",
                    ))
                })
                .collect::<std::result::Result<Vec<_>, _>>()?;
            parsed = parsed.with_locations_any(locations);
        }
    }
    if let Some(value) = object.get("years_experience_min") {
        if !value.is_null() {
            let years = value
                .as_f64()
                .filter(|years| years.is_finite() && *years >= 0.0)
                .ok_or(IpcCommandError::BadRequest(
                    "years_experience_min is invalid",
                ))? as f32;
            if !years.is_finite() {
                return Err(IpcCommandError::BadRequest(
                    "years_experience_min is invalid",
                ));
            }
            parsed = parsed.with_years_experience_min(years);
        }
    }
    search_projection_filter(&parsed)
}

fn search_projection_filter(
    filters: &SearchFilters,
) -> std::result::Result<SearchProjectionFilter, IpcCommandError> {
    let mut predicates = Vec::new();
    if let Some(degree) = filters.degree_min() {
        predicates.push(SearchProjectionPredicate::EntityValuesAny {
            entity_type: EntityType::Degree,
            normalized_values: degree_filter_values(degree),
            min_confidence: FIELD_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::Exact,
        });
    }
    push_text_filter(&mut predicates, EntityType::Name, filters.names_any());
    push_school_tier_filter(&mut predicates, filters.school_tiers_any());
    push_text_filter(&mut predicates, EntityType::School, filters.schools_any());
    push_text_filter(&mut predicates, EntityType::Major, filters.majors_any());
    push_text_filter(
        &mut predicates,
        EntityType::Certificate,
        filters.certificates_any(),
    );
    if let Some(range) = filters.date_range_overlaps() {
        predicates.push(SearchProjectionPredicate::DateRangeOverlap {
            start_month: range.start_month(),
            end_month: range.end_month(),
            min_confidence: FIELD_CONFIDENCE_THRESHOLD,
        });
    }
    push_text_filter(
        &mut predicates,
        EntityType::Company,
        filters.companies_any(),
    );
    push_text_filter(&mut predicates, EntityType::Title, filters.titles_any());
    push_text_filter(
        &mut predicates,
        EntityType::Location,
        filters.locations_any(),
    );
    push_text_filter(&mut predicates, EntityType::Skill, filters.skills_any());
    if !filters.contact_hashes_any().is_empty() {
        let hashes = filters
            .contact_hashes_any()
            .iter()
            .map(|value| {
                ContactHash::from_keyed_digest(value.clone()).map_err(|_| {
                    IpcCommandError::BadRequest("contact_hashes_any values must be contact hashes")
                })
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        predicates.push(SearchProjectionPredicate::ContactHashesAny(hashes));
    }
    if let Some(minimum) = filters.years_experience_min() {
        predicates.push(SearchProjectionPredicate::NumericEntityMinimum {
            entity_type: EntityType::YearsExperience,
            minimum,
            min_confidence: FIELD_CONFIDENCE_THRESHOLD,
        });
    }
    SearchProjectionFilter::new(predicates)
        .map_err(|_| IpcCommandError::BadRequest("filters are invalid"))
}

fn push_text_filter(
    predicates: &mut Vec<SearchProjectionPredicate>,
    entity_type: EntityType,
    values: &[String],
) {
    if values.is_empty() {
        return;
    }
    predicates.push(SearchProjectionPredicate::EntityValuesAny {
        entity_type,
        normalized_values: values.to_vec(),
        min_confidence: FIELD_CONFIDENCE_THRESHOLD,
        case: SearchFilterCase::AsciiInsensitive,
    });
}

fn push_school_tier_filter(predicates: &mut Vec<SearchProjectionPredicate>, tiers: &[SchoolTier]) {
    if tiers.is_empty() {
        return;
    }
    let include_missing = tiers.contains(&SchoolTier::Unknown);
    let values = tiers
        .iter()
        .filter(|tier| **tier != SchoolTier::Unknown)
        .map(|tier| tier.canonical().to_string())
        .collect::<Vec<_>>();
    let predicate = match (values.is_empty(), include_missing) {
        (true, true) => SearchProjectionPredicate::MissingEntityType {
            entity_type: EntityType::SchoolTier,
            min_confidence: FIELD_CONFIDENCE_THRESHOLD,
        },
        (false, true) => SearchProjectionPredicate::EntityValuesAnyOrMissing {
            entity_type: EntityType::SchoolTier,
            normalized_values: values,
            min_confidence: FIELD_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::Exact,
        },
        (false, false) => SearchProjectionPredicate::EntityValuesAny {
            entity_type: EntityType::SchoolTier,
            normalized_values: values,
            min_confidence: FIELD_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::Exact,
        },
        (true, false) => return,
    };
    predicates.push(predicate);
}

fn degree_filter_values(minimum: DegreeLevel) -> Vec<String> {
    [
        DegreeLevel::HighSchool,
        DegreeLevel::Associate,
        DegreeLevel::Bachelor,
        DegreeLevel::Master,
        DegreeLevel::Doctor,
    ]
    .into_iter()
    .filter(|degree| *degree >= minimum)
    .map(|degree| degree.canonical().to_string())
    .collect()
}

fn record_daemon_query_observation(
    store: &MetaStore,
    mode: DaemonSearchMode,
    duration: Duration,
    result_count: usize,
) {
    let Ok(observed_at) = current_timestamp() else {
        return;
    };
    let _ = store.record_query_observation(mode.label(), duration, result_count, observed_at);
}

fn execute_delete_command(
    data_dir: &Path,
    body: &[u8],
) -> std::result::Result<String, IpcCommandError> {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| IpcCommandError::BadRequest("invalid json"))?;
    let document_id = parse_delete_command(&payload)?;
    let store = open_store(data_dir).map_err(IpcCommandError::Internal)?;
    let now = current_timestamp().map_err(IpcCommandError::Internal)?;
    let Some(document) = store
        .document_by_id(&document_id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
    else {
        return Err(IpcCommandError::NotFound("delete document was not found"));
    };
    if document.is_deleted || document.status == DocumentStatus::Deleted {
        return Err(IpcCommandError::NotFound("delete document was not found"));
    }
    let publication = import_pipeline::publish_search_projection_removals(
        data_dir,
        &store,
        &[SearchProjectionRemoval {
            document_id: document_id.clone(),
            reason: SearchProjectionRemovalReason::ConfirmedSourceDeletion,
        }],
        now,
        &SearchPublicationVectorization::default(),
    )
    .map_err(DaemonError::import)
    .map_err(IpcCommandError::Internal)?;
    Ok(serde_json::json!({
        "schema_version": "resume-ir.delete-response.v2",
        "status": "ok",
        "doc_id": document_id.as_str(),
        "publication_committed": true,
        "indexed_documents": publication.active_projection_count,
    })
    .to_string())
}

fn parse_delete_command(
    payload: &serde_json::Value,
) -> std::result::Result<DocumentId, IpcCommandError> {
    let value = payload
        .get("doc_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(IpcCommandError::BadRequest("doc_id is required"))?;
    DocumentId::from_str(value).map_err(|_| IpcCommandError::BadRequest("doc_id is invalid"))
}

fn cancel_import_command(
    data_dir: &Path,
    body: &[u8],
) -> std::result::Result<String, IpcCommandError> {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| IpcCommandError::BadRequest("invalid json"))?;
    let task_id = parse_import_cancel_task_id(&payload)?;
    let store = open_store(data_dir).map_err(IpcCommandError::Internal)?;
    let Some(task) = store
        .import_task_by_id(&task_id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
    else {
        return Err(IpcCommandError::NotFound("import task was not found"));
    };
    if !matches!(
        task.status,
        ImportTaskStatus::Queued | ImportTaskStatus::Running | ImportTaskStatus::FailedRetryable
    ) {
        return Err(IpcCommandError::Conflict("import task cannot be cancelled"));
    }
    let now = current_timestamp().map_err(IpcCommandError::Internal)?;
    let inserted = store
        .cancel_import_task(&task_id, now)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?;
    let body = serde_json::json!({
        "schema_version": "daemon.import_cancel.v1",
        "status": "cancel_requested",
        "task_id": task_id.to_string(),
        "already_cancelled": !inserted,
    });
    Ok(body.to_string())
}

fn parse_import_cancel_task_id(
    payload: &serde_json::Value,
) -> std::result::Result<ImportTaskId, IpcCommandError> {
    let value = payload
        .get("task_id")
        .and_then(serde_json::Value::as_str)
        .ok_or(IpcCommandError::BadRequest("task_id is required"))?;
    ImportTaskId::from_str(value).map_err(|_| IpcCommandError::BadRequest("task_id is invalid"))
}

fn redact_search_file_name(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = redact_contact_values(&compact);
    truncate_utf8_bytes(&redacted, SEARCH_RESULT_FILE_NAME_MAX_BYTES)
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }

    const ELLIPSIS: &str = "...";
    let mut end = max_bytes.saturating_sub(ELLIPSIS.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], ELLIPSIS)
}

fn enqueue_import_command(
    data_dir: &Path,
    body: &[u8],
) -> std::result::Result<String, IpcCommandError> {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| IpcCommandError::BadRequest("invalid json"))?;
    let roots = parse_import_command_roots(&payload)?;
    let root_preset = parse_import_command_root_preset(&payload)?;
    let profile = parse_import_command_profile(&payload)?;
    let max_files = parse_import_command_max_files(&payload)?;
    let canonical_roots = canonical_import_roots(&roots)?;
    let store = open_store(data_dir).map_err(IpcCommandError::Internal)?;
    let now = current_timestamp().map_err(IpcCommandError::Internal)?;
    let mut task_ids = Vec::new();
    let mut new_tasks = 0_usize;

    for (root_index, root) in canonical_roots.iter().enumerate() {
        let canonical_root_path = path_string(&root.canonical);
        let requested_root_path = path_string(&root.requested);
        if store
            .import_root_control_status(&canonical_root_path)
            .map_err(DaemonError::store)
            .map_err(IpcCommandError::Internal)?
            == Some(meta_store::ImportRootControlStatus::Paused)
        {
            return Err(IpcCommandError::Conflict("managed root is paused"));
        }
        let task = match store
            .pending_import_task_by_root(&canonical_root_path)
            .map_err(DaemonError::store)
            .map_err(IpcCommandError::Internal)?
        {
            Some(task) if task.status == ImportTaskStatus::Running => {
                return Err(IpcCommandError::Conflict("import task is already running"));
            }
            Some(task) => {
                let scope = import_command_scan_scope(
                    &task.id,
                    requested_root_path,
                    canonical_root_path,
                    root_preset,
                    profile,
                    max_files,
                    now,
                )?;
                store
                    .upsert_import_scan_scope(&scope)
                    .map_err(DaemonError::store)
                    .map_err(IpcCommandError::Internal)?;
                task
            }
            None => {
                let task = ImportTask {
                    id: new_import_task_id(root_index).map_err(IpcCommandError::Internal)?,
                    root_path: canonical_root_path.clone(),
                    status: ImportTaskStatus::Queued,
                    queued_at: now,
                    started_at: None,
                    finished_at: None,
                    updated_at: now,
                };
                let scope = import_command_scan_scope(
                    &task.id,
                    requested_root_path,
                    canonical_root_path,
                    root_preset,
                    profile,
                    max_files,
                    now,
                )?;
                store
                    .insert_import_task_with_scan_scope(&task, &scope)
                    .map_err(DaemonError::store)
                    .map_err(IpcCommandError::Internal)?;
                new_tasks += 1;
                task
            }
        };

        task_ids.push(task.id.to_string());
    }

    let body = serde_json::json!({
        "schema_version": "daemon.import.v1",
        "status": "accepted",
        "accepted_roots": canonical_roots.len(),
        "new_tasks": new_tasks,
        "task_ids": task_ids,
        "scan_profile": import_scan_profile_label(profile),
        "scan_file_limit": max_files,
    });
    Ok(body.to_string())
}

fn import_command_scan_scope(
    task_id: &ImportTaskId,
    requested_root_path: String,
    canonical_root_path: String,
    root_preset: Option<ImportRootPreset>,
    profile: ImportScanProfile,
    max_files: Option<usize>,
    updated_at: UnixTimestamp,
) -> std::result::Result<ImportScanScope, IpcCommandError> {
    Ok(ImportScanScope {
        import_task_id: task_id.clone(),
        root_kind: if root_preset.is_some() {
            ImportRootKind::Preset
        } else {
            ImportRootKind::Explicit
        },
        root_preset,
        scan_profile: profile,
        requested_root_path,
        canonical_root_path,
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: max_files.map(|_| ImportScanBudgetKind::Files),
        scan_budget_limit: max_files
            .map(usize_to_u64)
            .transpose()
            .map_err(IpcCommandError::Internal)?,
        scan_budget_observed: max_files.map(|_| 0),
        scan_budget_exhausted: false,
        updated_at,
    })
}

fn parse_import_command_root_preset(
    payload: &serde_json::Value,
) -> std::result::Result<Option<ImportRootPreset>, IpcCommandError> {
    let Some(value) = payload.get("root_preset") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    match value.as_str() {
        Some("local-discovery") => Ok(Some(ImportRootPreset::LocalDiscovery)),
        _ => Err(IpcCommandError::BadRequest("invalid root_preset")),
    }
}

fn parse_import_command_roots(
    payload: &serde_json::Value,
) -> std::result::Result<Vec<PathBuf>, IpcCommandError> {
    let roots = payload
        .get("roots")
        .and_then(serde_json::Value::as_array)
        .filter(|roots| !roots.is_empty())
        .ok_or(IpcCommandError::BadRequest(
            "roots must be a non-empty array",
        ))?;
    if roots.len() > 64 {
        return Err(IpcCommandError::BadRequest("too many roots"));
    }
    roots
        .iter()
        .map(|root| {
            let value = root
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or(IpcCommandError::BadRequest("roots must be strings"))?;
            Ok(PathBuf::from(value))
        })
        .collect()
}

fn parse_import_command_profile(
    payload: &serde_json::Value,
) -> std::result::Result<ImportScanProfile, IpcCommandError> {
    match payload
        .get("profile")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("explicit")
    {
        "explicit" => Ok(ImportScanProfile::Explicit),
        "discovery" => Ok(ImportScanProfile::Discovery),
        _ => Err(IpcCommandError::BadRequest("invalid profile")),
    }
}

fn parse_import_command_max_files(
    payload: &serde_json::Value,
) -> std::result::Result<Option<usize>, IpcCommandError> {
    let Some(value) = payload.get("max_files") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_u64()
        .filter(|value| *value > 0)
        .ok_or(IpcCommandError::BadRequest("max_files must be positive"))?;
    let value = usize::try_from(value)
        .map_err(|_| IpcCommandError::BadRequest("max_files is too large"))?;
    Ok(Some(value))
}

fn canonical_import_roots(
    requested_roots: &[PathBuf],
) -> std::result::Result<Vec<CanonicalImportRoot>, IpcCommandError> {
    let mut roots = requested_roots
        .iter()
        .map(|requested_root| {
            let metadata = fs::metadata(requested_root).map_err(|_| {
                IpcCommandError::BadRequest("import root must exist and be a directory")
            })?;
            if !metadata.is_dir() {
                return Err(IpcCommandError::BadRequest(
                    "import root must exist and be a directory",
                ));
            }
            let canonical = fs::canonicalize(requested_root).map_err(|_| {
                IpcCommandError::BadRequest("import root must exist and be a directory")
            })?;
            Ok(CanonicalImportRoot {
                requested: requested_root.clone(),
                canonical,
            })
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;

    roots.sort_by(|left, right| left.canonical.cmp(&right.canonical));
    for window in roots.windows(2) {
        let [left, right] = window else {
            continue;
        };
        if left.canonical == right.canonical || right.canonical.starts_with(&left.canonical) {
            return Err(IpcCommandError::BadRequest(
                "import roots must be distinct and non-overlapping",
            ));
        }
    }
    Ok(roots)
}

fn new_import_task_id(root_index: usize) -> Result<ImportTaskId> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DaemonError::user("system clock is before unix epoch"))?;
    let nanos = duration.as_nanos().to_string();
    let pid = std::process::id().to_string();
    let root_index = root_index.to_string();

    Ok(ImportTaskId::from_non_secret_parts(&[
        "s46-import-task",
        &nanos,
        &pid,
        &root_index,
    ]))
}

fn path_string(path: &Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

fn import_scan_profile_label(profile: ImportScanProfile) -> &'static str {
    match profile {
        ImportScanProfile::Explicit => "explicit",
        ImportScanProfile::Discovery => "discovery",
    }
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (left, right) in left.iter().zip(right.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

enum IpcReadOutcome {
    Request(IpcRequest),
    TooLarge,
    BadRequest,
}

struct IpcRequest {
    method: String,
    path: String,
    version: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl IpcRequest {
    fn empty() -> Self {
        Self {
            method: String::new(),
            path: String::new(),
            version: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

enum IpcCommandError {
    BadRequest(&'static str),
    Conflict(&'static str),
    NotFound(&'static str),
    TooLarge(&'static str),
    ServiceUnavailable(&'static str),
    Internal(DaemonError),
}

struct CanonicalImportRoot {
    requested: PathBuf,
    canonical: PathBuf,
}

struct DaemonSearchArgs {
    query: String,
    mode: DaemonSearchMode,
    top_k: usize,
    filter: SearchProjectionFilter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DaemonSearchMode {
    FullText,
    Semantic,
    Hybrid,
}

impl DaemonSearchMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "fulltext" | "keyword" => Some(Self::FullText),
            "semantic" => Some(Self::Semantic),
            "hybrid" => Some(Self::Hybrid),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::FullText => "fulltext",
            Self::Semantic => "semantic",
            Self::Hybrid => "hybrid",
        }
    }

    fn response_label(self) -> &'static str {
        match self {
            Self::FullText => "keyword",
            Self::Semantic => "semantic",
            Self::Hybrid => "hybrid",
        }
    }
}

fn status_json(store: &MetaStore) -> String {
    status_json_with(|| status_json_once(store))
}

fn status_json_with(read: impl FnMut() -> Result<String>) -> String {
    retry_ipc_metadata_read(read).unwrap_or_else(|_| unavailable_status_json())
}

fn unavailable_status_json() -> String {
    let services = ipc::ServiceHealth {
        metadata: ipc::ServiceState::Unavailable,
        query: ipc::ServiceState::Unavailable,
    };
    let metrics = ipc::process_metrics().snapshot();
    serde_json::json!({
        "schema_version": "daemon.status.v2",
        "status": "degraded",
        "process_state": "ready",
        "service_state": services.aggregate().label(),
        "services": {
            "metadata": services.metadata.label(),
            "query": services.query.label(),
        },
        "error": {
            "code": "METADATA_UNAVAILABLE",
            "action": "retry",
        },
        "indexed_documents": serde_json::Value::Null,
        "searchable_documents": serde_json::Value::Null,
        "partial_documents": serde_json::Value::Null,
        "visible_epoch": serde_json::Value::Null,
        "ipc": ipc_metrics_json(metrics),
    })
    .to_string()
}

fn status_json_once(store: &MetaStore) -> Result<String> {
    let summary = store.status_summary().map_err(DaemonError::store)?;
    let projection = store
        .search_projection_state()
        .map_err(DaemonError::store)?;
    let latest_import_scan = store
        .latest_import_scan_scope()
        .map_err(DaemonError::store)?
        .map(|scope| latest_import_scan_json(&scope))
        .unwrap_or(serde_json::Value::Null);
    let services = projection_service_health(projection.service_state);
    let metrics = ipc::process_metrics().snapshot();
    let body = serde_json::json!({
        "schema_version": "daemon.status.v2",
        "status": match services.aggregate() {
            ipc::ServiceState::Ready => "ok",
            ipc::ServiceState::Repairing => "repairing",
            ipc::ServiceState::Degraded | ipc::ServiceState::Unavailable => "degraded",
        },
        "process_state": "ready",
        "service_state": services.aggregate().label(),
        "services": {
            "metadata": services.metadata.label(),
            "query": services.query.label(),
        },
        "error": service_error_json(services),
        "ipc": ipc_metrics_json(metrics),
        "visible_epoch": projection.visible_epoch,
        "indexed_documents": summary.indexed_documents,
        "searchable_documents": summary.searchable_documents,
        "partial_documents": summary.partial_documents,
        "failed_retryable": summary.failed_retryable,
        "failed_permanent": summary.failed_permanent,
        "recovery_queue_depth": summary.recovery_queue_depth,
        "ocr_queue_depth": summary.ocr_queue_depth,
        "ocr_jobs_queued": summary.ocr_jobs_queued,
        "ocr_page_budget_blocked": summary.ocr_page_budget_blocked,
        "ocr_remediation": if summary.ocr_page_budget_blocked > 0 {
            OCR_PAGE_BUDGET_REMEDIATION
        } else {
            "none"
        },
        "ocr_language_unavailable": summary.ocr_language_unavailable,
        "ocr_language_remediation": if summary.ocr_language_unavailable > 0 {
            OCR_LANGUAGE_REMEDIATION
        } else {
            "none"
        },
        "embedding_queue_depth": summary.embedding_queue_depth,
        "entity_mentions": summary.entity_mentions,
        "import_tasks_queued": summary.import_tasks_queued,
        "import_tasks_recoverable": summary.import_tasks_recoverable,
        "import_tasks_cancelled": summary.import_tasks_cancelled,
        "import_scan_scopes": summary.import_scan_scopes,
        "import_scan_errors": summary.import_scan_errors,
        "query_latency": {
            "sample_count": summary.query_latency.sample_count,
            "p50_ms": summary.query_latency.p50_ms,
            "p95_ms": summary.query_latency.p95_ms,
            "p99_ms": summary.query_latency.p99_ms,
            "last_result_count": summary.query_latency.last_result_count,
            "raw_queries": "<redacted>",
        },
        "latest_import_scan": latest_import_scan,
        "active_profile": "balanced",
        "index_health": index_health_label(summary.index_health),
        "snapshot_present": summary.last_snapshot_id.is_some(),
    });
    Ok(body.to_string())
}

fn ipc_metrics_json(metrics: ipc::IpcMetricsSnapshot) -> serde_json::Value {
    serde_json::json!({
        "accepted": metrics.accepted,
        "completed": metrics.completed,
        "client_disconnect": metrics.client_disconnect,
        "request_failure": metrics.request_failure,
        "response_failure": metrics.response_failure,
    })
}

fn projection_service_health(state: SearchProjectionServiceState) -> ipc::ServiceHealth {
    ipc::ServiceHealth {
        metadata: ipc::ServiceState::Ready,
        query: match state {
            SearchProjectionServiceState::Ready => ipc::ServiceState::Ready,
            SearchProjectionServiceState::Repairing => ipc::ServiceState::Repairing,
            SearchProjectionServiceState::RepairBlocked => ipc::ServiceState::Unavailable,
        },
    }
}

fn service_error_json(services: ipc::ServiceHealth) -> serde_json::Value {
    match services.aggregate() {
        ipc::ServiceState::Ready => serde_json::Value::Null,
        ipc::ServiceState::Repairing => serde_json::json!({
            "code": "REPAIRING",
            "action": "wait_for_repair",
        }),
        ipc::ServiceState::Degraded | ipc::ServiceState::Unavailable => serde_json::json!({
            "code": "QUERY_SERVICE_UNAVAILABLE",
            "action": "retry",
        }),
    }
}

fn retry_ipc_metadata_read<T>(mut read: impl FnMut() -> Result<T>) -> Result<T> {
    let mut last_error = None;
    for attempt in 1..=IPC_METADATA_READ_ATTEMPTS {
        match read() {
            Ok(value) => return Ok(value),
            Err(error)
                if error.retryable_metadata_error() && attempt < IPC_METADATA_READ_ATTEMPTS =>
            {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(IPC_METADATA_READ_RETRY_MS));
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_error.expect("metadata read retry loop records a failed attempt"))
}

fn import_progress_stream_event_json(data_dir: &Path) -> Result<String> {
    let store = open_store(data_dir)?;
    let latest_import_scan = store
        .latest_import_scan_scope()
        .map_err(DaemonError::store)?
        .map(|scope| latest_import_scan_json(&scope))
        .unwrap_or(serde_json::Value::Null);
    let body = serde_json::json!({
        "schema_version": "daemon.import_progress.v1",
        "event": "snapshot",
        "latest_import_scan": latest_import_scan,
    });
    Ok(body.to_string())
}

fn latest_import_scan_json(scope: &ImportScanScope) -> serde_json::Value {
    serde_json::json!({
        "scan_profile": import_scan_profile_label(scope.scan_profile),
        "files_discovered": scope.files_discovered,
        "ignored_entries": scope.ignored_entries,
        "scan_errors": scope.scan_errors,
        "searchable_documents": scope.searchable_documents,
        "ocr_required_documents": scope.ocr_required_documents,
        "ocr_jobs_queued": scope.ocr_jobs_queued,
        "failed_documents": scope.failed_documents,
        "deleted_documents": scope.deleted_documents,
        "scan_budget_observed": scope.scan_budget_observed,
        "scan_budget_limit": scope.scan_budget_limit,
        "scan_budget_exhausted": scope.scan_budget_exhausted,
    })
}

fn write_http_response(
    stream: &mut TcpStream,
    status_code: u16,
    content_type: &str,
    body: &str,
) -> Result<()> {
    let reason = match status_code {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    };
    ipc::response::write_http_response(stream, status_code, reason, content_type, body)
        .map_err(DaemonError::response_sink)
}

fn write_service_unavailable(stream: &mut TcpStream, code: ipc::ServiceErrorCode) -> Result<()> {
    let body = unified_error_body(None, code.label(), code.action());
    write_http_response(stream, 503, "application/json", &body)
}

fn unified_error_body(request_id: Option<&str>, code: &str, action: &str) -> String {
    let mut body = serde_json::json!({
        "schema_version": "resume-ir.error.v1",
        "status": "error",
        "error": {
            "code": code,
            "action": action,
        },
    });
    if let Some(request_id) = request_id {
        body["request_id"] = serde_json::json!(request_id);
    }
    body.to_string()
}

fn write_search_http_response(stream: &mut TcpStream, output: DaemonSearchOutput) -> Result<()> {
    let server_timing = output.stage_timing.server_timing_header_value();
    ipc::response::write_search_response(stream, &server_timing, &output.body)
        .map_err(DaemonError::response_sink)
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

#[derive(Default)]
struct ImportWorkerSummary {
    stale_recovered: usize,
    completed_requeued: usize,
    watcher_active_roots: Option<usize>,
    watcher_events: usize,
    watcher_requeued: usize,
    watcher_event_errors: usize,
    processed: usize,
    cancelled: usize,
    failed: usize,
    failure_class: Option<ImportPipelineErrorClass>,
    searchable_documents: usize,
    ocr_jobs_queued: usize,
}

impl ImportWorkerSummary {
    fn has_activity(&self) -> bool {
        self.stale_recovered > 0
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
        self.stale_recovered += other.stale_recovered;
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
        "import worker recovered stale running: {}",
        import_summary.stale_recovered
    );
    println!(
        "import worker requeued completed imports: {}",
        import_summary.completed_requeued
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
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::control_plane("unable to write daemon status"))
}

struct ImportTaskHeartbeat {
    stop: Arc<AtomicBool>,
}

impl ImportTaskHeartbeat {
    fn start(data_dir: &Path, task_id: ImportTaskId) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let heartbeat_data_dir = data_dir.to_path_buf();

        let _ = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(IMPORT_TASK_HEARTBEAT_SECONDS));
                if thread_stop.load(Ordering::Relaxed) {
                    return;
                }

                let Ok(now) = current_timestamp() else {
                    continue;
                };
                let Ok(store) = MetaStore::open_data_dir(&heartbeat_data_dir) else {
                    continue;
                };
                if store.run_migrations().is_err() {
                    continue;
                }
                let _ = store.heartbeat_running_import_task(&task_id, now);
            }
        });

        Self { stop }
    }
}

impl Drop for ImportTaskHeartbeat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn open_store(data_dir: &Path) -> Result<MetaStore> {
    fs::create_dir_all(data_dir)
        .map_err(|_| DaemonError::recoverable_dependency("local metadata directory unavailable"))?;
    let store = MetaStore::open_data_dir(data_dir).map_err(DaemonError::store)?;
    store.run_migrations().map_err(DaemonError::store)?;
    Ok(store)
}

fn index_health_label(status: IndexStateStatus) -> &'static str {
    match status {
        IndexStateStatus::Empty => "empty",
        IndexStateStatus::Building => "building",
        IndexStateStatus::Ready => "ready",
        IndexStateStatus::Stale => "stale",
    }
}

type Result<T> = std::result::Result<T, DaemonError>;

#[derive(Debug)]
struct DaemonError {
    message: String,
    exit_code: i32,
    kind: DaemonErrorKind,
}

#[derive(Clone, Copy, Debug)]
enum DaemonErrorKind {
    ConfigurationInvalid,
    RuntimeIntegrity,
    RecoverableDependency,
    Store(MetaStoreErrorClass),
    OwnershipConflict,
    ProtocolMismatch,
    ControlPlane,
    ResponseSink(ipc::ResponseSinkError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DaemonFatalClass {
    OwnershipConflict,
    ConfigurationInvalid,
    RuntimeIntegrity,
    ProtocolMismatch,
    ControlPlaneFailure,
}

impl DaemonFatalClass {
    fn label(self) -> &'static str {
        match self {
            Self::OwnershipConflict => "ownership_conflict",
            Self::ConfigurationInvalid => "configuration_invalid",
            Self::RuntimeIntegrity => "runtime_integrity",
            Self::ProtocolMismatch => "protocol_mismatch",
            Self::ControlPlaneFailure => "control_plane_failure",
        }
    }

    fn disposition(self) -> &'static str {
        match self {
            Self::ControlPlaneFailure => "restartable",
            Self::OwnershipConflict
            | Self::ConfigurationInvalid
            | Self::RuntimeIntegrity
            | Self::ProtocolMismatch => "blocked",
        }
    }
}

impl DaemonError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 2,
            kind: DaemonErrorKind::ConfigurationInvalid,
        }
    }

    fn user(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
            kind: DaemonErrorKind::RuntimeIntegrity,
        }
    }

    fn store(error: meta_store::MetaStoreError) -> Self {
        Self {
            message: "metadata store operation failed".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::Store(error.class()),
        }
    }

    fn import(_error: import_pipeline::ImportPipelineError) -> Self {
        Self {
            message: "import pipeline operation failed".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::RecoverableDependency,
        }
    }

    fn ocr(error: ocr_client::OcrError) -> Self {
        Self {
            message: "ocr service operation failed".to_string(),
            exit_code: 1,
            kind: match error.kind() {
                ocr_client::OcrErrorKind::InvalidRequest => DaemonErrorKind::ConfigurationInvalid,
                ocr_client::OcrErrorKind::Disabled
                | ocr_client::OcrErrorKind::Cancelled
                | ocr_client::OcrErrorKind::Timeout
                | ocr_client::OcrErrorKind::WorkerUnavailable
                | ocr_client::OcrErrorKind::LanguageUnavailable
                | ocr_client::OcrErrorKind::EngineFailed => DaemonErrorKind::RecoverableDependency,
            },
        }
    }

    fn embedding(error: embedder::EmbeddingError) -> Self {
        let kind = match error {
            embedder::EmbeddingError::InvalidDimension
            | embedder::EmbeddingError::InvalidRequest
            | embedder::EmbeddingError::BudgetExceeded { .. }
            | embedder::EmbeddingError::TextBudgetExceeded { .. } => {
                DaemonErrorKind::ConfigurationInvalid
            }
            embedder::EmbeddingError::WorkerUnavailable
            | embedder::EmbeddingError::EngineFailed
            | embedder::EmbeddingError::Overloaded
            | embedder::EmbeddingError::Cancelled
            | embedder::EmbeddingError::Timeout => DaemonErrorKind::RecoverableDependency,
        };
        Self {
            message: "embedding service operation failed".to_string(),
            exit_code: 1,
            kind,
        }
    }

    fn response_sink(error: ipc::ResponseSinkError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::ResponseSink(error),
        }
    }

    fn control_plane(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
            kind: DaemonErrorKind::ControlPlane,
        }
    }

    fn ownership_conflict() -> Self {
        Self {
            message: "daemon ownership conflict".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::OwnershipConflict,
        }
    }

    fn runtime_integrity() -> Self {
        Self {
            message: "daemon runtime integrity failure".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::RuntimeIntegrity,
        }
    }

    fn configuration_invalid(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
            kind: DaemonErrorKind::ConfigurationInvalid,
        }
    }

    fn recoverable_dependency(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
            kind: DaemonErrorKind::RecoverableDependency,
        }
    }

    fn protocol_mismatch() -> Self {
        Self {
            message: "daemon ipc protocol mismatch".to_string(),
            exit_code: 1,
            kind: DaemonErrorKind::ProtocolMismatch,
        }
    }

    fn into_request_failure(self) -> std::result::Result<ipc::RequestFailure, DaemonError> {
        match self.kind {
            DaemonErrorKind::ConfigurationInvalid
            | DaemonErrorKind::RuntimeIntegrity
            | DaemonErrorKind::RecoverableDependency
            | DaemonErrorKind::Store(_)
            | DaemonErrorKind::OwnershipConflict
            | DaemonErrorKind::ProtocolMismatch => Ok(ipc::RequestFailure::Handler),
            DaemonErrorKind::ControlPlane => Err(self),
            DaemonErrorKind::ResponseSink(error) => Ok(ipc::RequestFailure::ResponseSink(error)),
        }
    }

    fn exit_code(&self) -> i32 {
        self.exit_code
    }

    fn retryable_metadata_error(&self) -> bool {
        matches!(
            self.kind,
            DaemonErrorKind::Store(MetaStoreErrorClass::Storage)
        )
    }

    fn fatal_class(&self) -> DaemonFatalClass {
        match self.kind {
            DaemonErrorKind::OwnershipConflict => DaemonFatalClass::OwnershipConflict,
            DaemonErrorKind::ConfigurationInvalid => DaemonFatalClass::ConfigurationInvalid,
            DaemonErrorKind::RuntimeIntegrity => DaemonFatalClass::RuntimeIntegrity,
            DaemonErrorKind::RecoverableDependency => DaemonFatalClass::ControlPlaneFailure,
            DaemonErrorKind::ProtocolMismatch => DaemonFatalClass::ProtocolMismatch,
            DaemonErrorKind::Store(MetaStoreErrorClass::Storage)
            | DaemonErrorKind::ControlPlane
            | DaemonErrorKind::ResponseSink(_) => DaemonFatalClass::ControlPlaneFailure,
            DaemonErrorKind::Store(MetaStoreErrorClass::MigrationOwnershipRequired) => {
                DaemonFatalClass::OwnershipConflict
            }
            DaemonErrorKind::Store(
                MetaStoreErrorClass::WeakPassphrase
                | MetaStoreErrorClass::InvalidBackup
                | MetaStoreErrorClass::Crypto
                | MetaStoreErrorClass::KeyAlreadyExists,
            ) => DaemonFatalClass::ConfigurationInvalid,
            DaemonErrorKind::Store(
                MetaStoreErrorClass::Migration
                | MetaStoreErrorClass::InvalidValue
                | MetaStoreErrorClass::NotFound
                | MetaStoreErrorClass::InvalidTransition
                | MetaStoreErrorClass::ImmutableIdentityConflict
                | MetaStoreErrorClass::StorageInvariant,
            ) => DaemonFatalClass::RuntimeIntegrity,
        }
    }

    fn fatal_event_json(&self) -> String {
        fatal_event_json_for_class(self.fatal_class())
    }
}

fn fatal_event_json_for_class(class: DaemonFatalClass) -> String {
    serde_json::json!({
        "schema_version": "resume-ir.daemon-fatal.v1",
        "event": "fatal",
        "class": class.label(),
        "disposition": class.disposition(),
    })
    .to_string()
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[cfg(test)]
mod daemon_contract_tests {
    use super::{
        fatal_event_json_for_class, projection_query_error, projection_service_health,
        status_json_with, unavailable_status_json, DaemonError, DaemonErrorKind, DaemonFatalClass,
        IPC_METADATA_READ_ATTEMPTS,
    };
    use meta_store::{MetaStoreErrorClass, SearchProjectionServiceState};

    #[test]
    fn fatal_wire_is_closed_bounded_and_contains_no_raw_message() {
        let classes = [
            DaemonFatalClass::OwnershipConflict,
            DaemonFatalClass::ConfigurationInvalid,
            DaemonFatalClass::RuntimeIntegrity,
            DaemonFatalClass::ProtocolMismatch,
            DaemonFatalClass::ControlPlaneFailure,
        ];
        for class in classes {
            let body = fatal_event_json_for_class(class);
            assert!(body.len() <= 1024);
            let value: serde_json::Value = serde_json::from_str(&body).unwrap();
            assert_eq!(value.as_object().unwrap().len(), 4);
            assert_eq!(value["schema_version"], "resume-ir.daemon-fatal.v1");
            assert_eq!(value["event"], "fatal");
            assert_eq!(value["class"], class.label());
            assert_eq!(value["disposition"], class.disposition());
        }

        let secret = "PRIVATE_PATH_TOKEN_QUERY";
        let event = DaemonError::control_plane(secret).fatal_event_json();
        assert!(!event.contains(secret));
    }

    #[test]
    fn fatal_mapping_separates_restartable_dependencies_from_blocked_failures() {
        let restartable = DaemonError::recoverable_dependency("transient dependency");
        let storage = DaemonError {
            message: String::new(),
            exit_code: 1,
            kind: DaemonErrorKind::Store(MetaStoreErrorClass::Storage),
        };
        let invariant = DaemonError {
            message: String::new(),
            exit_code: 1,
            kind: DaemonErrorKind::Store(MetaStoreErrorClass::StorageInvariant),
        };
        let ownership = DaemonError {
            message: String::new(),
            exit_code: 1,
            kind: DaemonErrorKind::Store(MetaStoreErrorClass::MigrationOwnershipRequired),
        };

        assert_eq!(
            restartable.fatal_class(),
            DaemonFatalClass::ControlPlaneFailure
        );
        assert_eq!(storage.fatal_class(), DaemonFatalClass::ControlPlaneFailure);
        assert_eq!(invariant.fatal_class(), DaemonFatalClass::RuntimeIntegrity);
        assert_eq!(ownership.fatal_class(), DaemonFatalClass::OwnershipConflict);
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
    fn metadata_retry_is_limited_to_storage_class() {
        let storage = DaemonError {
            message: String::new(),
            exit_code: 1,
            kind: DaemonErrorKind::Store(MetaStoreErrorClass::Storage),
        };
        let invariant = DaemonError {
            message: String::new(),
            exit_code: 1,
            kind: DaemonErrorKind::Store(MetaStoreErrorClass::StorageInvariant),
        };
        assert!(storage.retryable_metadata_error());
        assert!(!invariant.retryable_metadata_error());
    }

    #[test]
    fn runtime_metadata_read_failure_returns_status_v2_with_unavailable_dependencies() {
        let mut attempts = 0;
        let body = status_json_with(|| {
            attempts += 1;
            Err(DaemonError {
                message: String::new(),
                exit_code: 1,
                kind: DaemonErrorKind::Store(MetaStoreErrorClass::Storage),
            })
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
            Some(crate::ipc::ServiceErrorCode::QueryServiceUnavailable)
        );
        assert_eq!(
            projection_query_error(None),
            Some(crate::ipc::ServiceErrorCode::MetadataUnavailable)
        );
    }
}
