use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
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
    Embedder, EmbeddingBudget, EmbeddingInput, LocalEmbeddingCommandEmbedder,
    LocalEmbeddingCommandSpec,
};
use import_pipeline::{
    detect_ocr_page_count, import_root_with_options, index_ocr_text, rebuild_full_text_index,
    ImportOptions, ImportScanBudgetKind as PipelineImportScanBudgetKind, ImportSummary,
    ScanProfile,
};
use index_fulltext::{
    inspect_snapshot_root, redact_contact_values, FullTextIndex, SearchHit, SearchQuery,
    SnapshotReadTarget, SnapshotRootState,
};
use index_vector::{
    inspect_persistent_vector_snapshot, PersistentVectorIndex, PersistentVectorSnapshotState,
    QueryVector, VectorDocument, VectorHit, VectorIndex,
};
use meta_store::{
    ContactHash, Document, DocumentId, DocumentStatus, EntityMention, EntityType, FileExtension,
    ImportRootKind, ImportRootPreset, ImportScanBudgetKind, ImportScanProfile, ImportScanScope,
    ImportTask, ImportTaskId, ImportTaskStatus, IndexStateStatus, IngestJob, IngestJobFailureKind,
    IngestJobKind, IngestJobStatus, MetaStore, OcrPageCacheEntry, OcrPageCacheKey,
    OcrPageCacheStatus, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
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
use rank_fusion::{
    fuse_hybrid_rrf, soft_dedupe_score, DateRange, DedupeProfile, DegreeLevel, HybridRecall,
    RankedHit, ResumeProfile, SchoolTier, SearchFilters,
};
use search_planner::plan_search;
use sectionizer::Sectionizer;

const IMPORT_RETRY_BACKOFF_SECONDS: i64 = 60;
const DEFAULT_IMPORT_RESCAN_MIN_AGE_SECONDS: i64 = 300;
const IMPORT_TASK_HEARTBEAT_SECONDS: u64 = 30;
const STALE_IMPORT_TASK_SECONDS: i64 = 15 * 60;
const IPC_AUTH_TOKEN_FILE: &str = "ipc.auth";
const IPC_ENDPOINT_FILE: &str = "ipc.endpoints.json";
const IPC_ENDPOINT_SCHEMA_VERSION: &str = "resume-ir.daemon-ipc.v1";
const IPC_AUTH_TOKEN_BYTES: usize = 32;
const IPC_MAX_REQUEST_BYTES: usize = 64 * 1024;
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
const OCR_PAGE_BUDGET_REMEDIATION: &str =
    "raise OCR max pages per document or skip oversized scanned PDFs";
const OCR_LANGUAGE_REMEDIATION: &str =
    "install requested OCR language packs or choose an installed OCR language";
const DEFAULT_EMBEDDING_MAX_DOCS: usize = 64;
const DEFAULT_EMBEDDING_MAX_TEXT_BYTES: usize = 1_000_000;
const DEFAULT_EMBEDDING_TIMEOUT_MS: u64 = 30_000;
const FIELD_CONFIDENCE_THRESHOLD: f32 = 0.75;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

fn main() {
    if let Err(error) = run() {
        eprintln!("resume-daemon: {error}");
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
    let options = parse_run_options(args)?;

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
    if options.work_embeddings_once && !options.once {
        return Err(DaemonError::usage(
            "usage: --work-embeddings-once requires --once",
        ));
    }
    if options.work_embeddings && options.once {
        return Err(DaemonError::usage(
            "usage: --work-embeddings cannot be combined with --once",
        ));
    }
    if options.work_embeddings && options.work_embeddings_once {
        return Err(DaemonError::usage(
            "usage: choose either --work-embeddings or --work-embeddings-once",
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
    if (options.worker_interval_ms.is_some() || options.max_worker_ticks.is_some())
        && !options.has_worker_loop()
    {
        return Err(DaemonError::usage(
            "usage: worker loop options require --work-imports, --work-ocr, --work-embeddings, or --work-index",
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
        return Err(DaemonError::user(
            "ocr worker blocked: local OCR command not configured",
        ));
    }
    if (options.work_embeddings_once || options.work_embeddings)
        && options.embedding_command.is_none()
    {
        return Err(DaemonError::user(
            "embedding worker blocked: local embedding command not configured",
        ));
    }
    if (options.work_embeddings_once || options.work_embeddings)
        && (options.embedding_model_id.is_none() || options.embedding_dimension.is_none())
    {
        return Err(DaemonError::usage(run_usage()));
    }

    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(DaemonError::store)?;

    println!("resume-daemon foreground ready");
    println!("mode: {}", if options.once { "once" } else { "foreground" });
    println!("index health: {}", index_health_label(summary.index_health));
    println!("import tasks queued: {}", summary.import_tasks_queued);
    println!("import tasks cancelled: {}", summary.import_tasks_cancelled);
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::user("unable to write daemon status"))?;

    if options.work_imports_once {
        let import_summary = run_import_worker_once(data_dir, &store)?;
        print_import_worker_summary(&import_summary)?;
    }
    if options.work_ocr_once {
        let ocr_summary = run_ocr_worker_once(data_dir, &store, &options)?;
        print_ocr_worker_summary(&ocr_summary)?;
    }
    if options.work_embeddings_once {
        let embedding_summary = run_embedding_worker_once(data_dir, &store, &options)?;
        print_embedding_worker_summary(&embedding_summary)?;
    }
    if options.work_index_once {
        let index_summary = run_index_worker_once(data_dir, &store, true)?;
        print_index_worker_summary(&index_summary)?;
    }

    if options.once {
        return Ok(());
    }
    if options.has_worker_loop() && options.ipc_listen.is_some() {
        run_worker_with_ipc(data_dir, &options)?;
        return Ok(());
    }
    if options.has_worker_loop() {
        run_worker_loop(data_dir, &store, &options, None)?;
        return Ok(());
    }
    if let Some(ipc_addr) = options.ipc_listen {
        serve_ipc(data_dir, ipc_addr, options.max_requests, &options)?;
        if options.max_requests.is_some() {
            return Ok(());
        }
    }

    loop {
        thread::sleep(Duration::from_secs(3_600));
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
            "--work-ocr-once" => {
                options.work_ocr_once = true;
                index += 1;
            }
            "--work-ocr" => {
                options.work_ocr = true;
                index += 1;
            }
            "--work-embeddings-once" => {
                options.work_embeddings_once = true;
                index += 1;
            }
            "--work-embeddings" => {
                options.work_embeddings = true;
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
            "--embedding-max-docs" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.embedding_max_docs = parse_positive_usize_run_value(value)?;
                index += 2;
            }
            "--embedding-max-text-bytes" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(run_usage()));
                };
                options.embedding_max_text_bytes = parse_positive_usize_run_value(value)?;
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
    "usage: resume-daemon run --foreground [--once] [--work-imports-once|--work-imports [--rescan-completed-imports] [--watch-import-roots] [--import-rescan-min-age-seconds <n>] [--stale-import-task-seconds <n>] [--import-retry-backoff-seconds <n>]] [--work-ocr-once|--work-ocr] [--work-embeddings-once|--work-embeddings] [--work-index-once|--work-index] [--ocr-command <path>|--ocr-tesseract-command <path>] [--ocr-render-command <path>|--ocr-pdftoppm-command <path>] [--ocr-engine-profile <name>] [--ocr-lang <lang>] [--ocr-profile <profile>] [--ocr-render-dpi <dpi>] [--ocr-page-timeout-ms <ms>] [--ocr-max-pages-per-document <n>] [--embedding-command <path>] [--embedding-model-id <id>] [--embedding-dimension <n>] [--embedding-max-docs <n>] [--embedding-max-text-bytes <bytes>] [--embedding-timeout-ms <ms>] [--worker-interval-ms <n>] [--max-worker-ticks <n>] [--ipc-listen <127.0.0.1:port>] [--max-requests <n>]"
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

fn run_worker_with_ipc(data_dir: &Path, options: &RunOptions) -> Result<()> {
    let ipc_addr = options
        .ipc_listen
        .expect("validated combined worker/ipc mode has ipc address");
    let listener = bind_ipc_listener(data_dir, ipc_addr)?;
    listener
        .set_nonblocking(true)
        .map_err(|_| DaemonError::user("unable to configure daemon ipc listener"))?;
    let ipc_store = open_store(data_dir)?;
    let stop_worker = Arc::new(AtomicBool::new(false));
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

    let ipc_result = serve_ipc_listener_with_worker_monitor(
        data_dir,
        &ipc_store,
        &listener,
        options.max_requests,
        options,
        &worker_result_receiver,
    );
    remove_ipc_endpoint_manifest(data_dir);
    stop_worker.store(true, Ordering::Relaxed);
    worker_handle
        .join()
        .map_err(|_| DaemonError::user("worker thread panicked"))?;
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
                import_summary.completed_requeued = requeue_completed_imports(
                    store,
                    now,
                    options
                        .import_rescan_min_age_seconds
                        .unwrap_or(DEFAULT_IMPORT_RESCAN_MIN_AGE_SECONDS),
                )?;
            }
            if let Some(watcher) = import_watcher.as_mut() {
                import_summary.extend_watcher(watcher.sync_and_requeue(store, now)?);
            }
            import_summary.extend(run_import_worker_once_with_retry_due(
                data_dir,
                store,
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
            let ocr_summary = run_ocr_worker_once(data_dir, store, options)?;
            if ocr_summary.has_activity() {
                print_ocr_worker_summary(&ocr_summary)?;
            }
        }
        if options.work_embeddings {
            let embedding_summary = run_embedding_worker_once(data_dir, store, options)?;
            if embedding_summary.has_activity() {
                print_embedding_worker_summary(&embedding_summary)?;
            }
        }
        if options.work_index {
            let index_summary = run_index_worker_once(data_dir, store, false)?;
            if index_summary.has_activity() {
                print_index_worker_summary(&index_summary)?;
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

fn run_import_worker_once(data_dir: &Path, store: &MetaStore) -> Result<ImportWorkerSummary> {
    let retryable_due_at = current_timestamp()?;
    run_import_worker_once_with_retry_due(data_dir, store, retryable_due_at)
}

fn run_index_worker_once(
    data_dir: &Path,
    store: &MetaStore,
    force_rebuild: bool,
) -> Result<IndexWorkerSummary> {
    if !force_rebuild && !full_text_index_needs_rebuild(data_dir) {
        return Ok(IndexWorkerSummary::default());
    }

    let summary = rebuild_full_text_index(data_dir, store, current_timestamp()?)
        .map_err(DaemonError::import)?;
    Ok(IndexWorkerSummary {
        rebuilt: true,
        indexed_documents: summary.indexed_documents,
    })
}

fn full_text_index_needs_rebuild(data_dir: &Path) -> bool {
    match inspect_snapshot_root(&data_dir.join("search-index")) {
        Ok(inspection) => !matches!(
            (inspection.state(), inspection.read_target()),
            (
                SnapshotRootState::Ready,
                Some(SnapshotReadTarget::PublishedSnapshot),
            )
        ),
        Err(_) => true,
    }
}

fn run_import_worker_once_with_retry_due(
    data_dir: &Path,
    store: &MetaStore,
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

        let import_options = match import_options_from_scope(&scope) {
            Ok(import_options) => import_options,
            Err(_) => {
                mark_import_task_failed_permanent(store, &task, now)?;
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
        let import_summary = match import_result {
            Ok(import_summary) => import_summary,
            Err(_) => {
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
        upsert_scope_summary(store, scope, import_summary, finished_at)?;
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
        return Err(DaemonError::user(
            "ocr worker blocked: local OCR command not configured",
        ));
    }

    let Some(job) = store
        .claim_next_job_by_kind(IngestJobKind::OcrDocument, now)
        .map_err(DaemonError::store)?
    else {
        return Ok(OcrWorkerSummary::default());
    };

    run_claimed_ocr_job(data_dir, store, &job, options, now)
}

fn run_claimed_ocr_job(
    data_dir: &Path,
    store: &MetaStore,
    job: &IngestJob,
    options: &RunOptions,
    now: UnixTimestamp,
) -> Result<OcrWorkerSummary> {
    let Some(document) = store
        .document_by_id(&job.document_id)
        .map_err(DaemonError::store)?
    else {
        mark_ocr_job_failed_permanent(store, job, now)?;
        return Ok(OcrWorkerSummary {
            failed: 1,
            ..OcrWorkerSummary::default()
        });
    };
    let Some(content_hash) = document.content_hash.clone() else {
        mark_ocr_job_failed_permanent(store, job, now)?;
        return Ok(OcrWorkerSummary {
            failed: 1,
            ..OcrWorkerSummary::default()
        });
    };

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
            return Err(DaemonError::user(
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
    if let Err(error) = index_ocr_text(
        data_dir,
        store,
        &document.id,
        &combined_text,
        confidence,
        Some(page_count),
        now,
    ) {
        let _ = mark_ocr_job_failed_retryable(store, job, now);
        return Err(DaemonError::import(error));
    }
    store
        .update_job_status(&job.id, IngestJobStatus::Completed, now)
        .map_err(DaemonError::store)?;
    Ok(OcrWorkerSummary {
        processed: 1,
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
    job: &IngestJob,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .update_job_status(&job.id, IngestJobStatus::FailedRetryable, now)
        .map_err(DaemonError::store)
}

fn mark_ocr_job_failed_retryable_with_failure_kind(
    store: &MetaStore,
    job: &IngestJob,
    failure_kind: IngestJobFailureKind,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .update_job_status_with_failure_kind(
            &job.id,
            IngestJobStatus::FailedRetryable,
            Some(failure_kind),
            now,
        )
        .map_err(DaemonError::store)
}

fn mark_ocr_job_failed_permanent(
    store: &MetaStore,
    job: &IngestJob,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .update_job_status(&job.id, IngestJobStatus::FailedPermanent, now)
        .map_err(DaemonError::store)
}

fn run_embedding_worker_once(
    data_dir: &Path,
    store: &MetaStore,
    options: &RunOptions,
) -> Result<EmbeddingWorkerSummary> {
    let Some(command) = options.embedding_command.clone() else {
        return Err(DaemonError::user(
            "embedding worker blocked: local embedding command not configured",
        ));
    };
    let model_id = options
        .embedding_model_id
        .as_deref()
        .ok_or_else(|| DaemonError::usage(run_usage()))?;
    let dimension = options
        .embedding_dimension
        .ok_or_else(|| DaemonError::usage(run_usage()))?;
    let now = current_timestamp()?;
    enqueue_embedding_jobs_for_candidates(
        store,
        options.embedding_max_docs,
        model_id,
        dimension,
        now,
    )?;
    let jobs = claim_embedding_jobs(store, options.embedding_max_docs, model_id, dimension, now)?;
    let documents_considered = jobs.len();
    if jobs.is_empty() {
        return Ok(EmbeddingWorkerSummary::default());
    }

    let mut candidates = Vec::new();
    for job in jobs {
        match embedding_candidate_for_job(store, &job)? {
            Some(candidate) => candidates.push((job, candidate)),
            None => store
                .update_job_status(&job.id, IngestJobStatus::Completed, now)
                .map_err(DaemonError::store)?,
        }
    }
    if candidates.is_empty() {
        return Ok(EmbeddingWorkerSummary {
            documents_considered,
            processed: 0,
            vector_writes: 0,
            failed: 0,
        });
    }

    let embedder = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), model_id, dimension)
            .map_err(DaemonError::embedding)?
            .with_timeout_ms(options.embedding_timeout_ms)
            .map_err(DaemonError::embedding)?,
    );
    let vector_inputs = embedding_inputs_for_candidates(&candidates);
    let inputs = vector_inputs
        .iter()
        .map(|input| EmbeddingInput::new(input.input_id.as_str(), input.text.as_str()))
        .collect::<Vec<_>>();
    let vectors = match embedder.embed_batch(
        &inputs,
        EmbeddingBudget::new(inputs.len(), options.embedding_max_text_bytes),
    ) {
        Ok(vectors) => vectors,
        Err(error) => {
            mark_embedding_jobs_failed_retryable(store, &candidates, now)?;
            return Err(DaemonError::embedding(error));
        }
    };
    let vector_writes = vectors.len();
    let vector_documents = vectors
        .into_iter()
        .zip(vector_inputs.iter())
        .map(|(vector, input)| {
            VectorDocument::new_for_model(
                vector.model_id(),
                format!("{}:{}", vector.model_id(), vector.id()),
                input.document_id.as_str(),
                vector.values().to_vec(),
            )
            .map_err(DaemonError::vector)
        })
        .collect::<Result<Vec<_>>>()?;
    let index = match PersistentVectorIndex::open(data_dir.join("vector-index"), dimension) {
        Ok(index) => index,
        Err(error) => {
            mark_embedding_jobs_failed_retryable(store, &candidates, now)?;
            return Err(DaemonError::vector(error));
        }
    };
    if let Err(error) = index.upsert(vector_documents) {
        mark_embedding_jobs_failed_retryable(store, &candidates, now)?;
        return Err(DaemonError::vector(error));
    }

    for (job, _) in &candidates {
        store
            .update_job_status(&job.id, IngestJobStatus::Completed, now)
            .map_err(DaemonError::store)?;
    }

    Ok(EmbeddingWorkerSummary {
        documents_considered,
        processed: candidates.len(),
        vector_writes,
        failed: 0,
    })
}

fn enqueue_embedding_jobs_for_candidates(
    store: &MetaStore,
    max_docs: usize,
    model_id: &str,
    dimension: usize,
    now: UnixTimestamp,
) -> Result<usize> {
    let mut pending_jobs = 0_usize;
    for document in store.visible_documents().map_err(DaemonError::store)? {
        if !matches!(
            document.status,
            DocumentStatus::FieldsExtracted
                | DocumentStatus::EmbeddingDone
                | DocumentStatus::IndexedPartial
                | DocumentStatus::Searchable
        ) {
            continue;
        }

        for version in store
            .resume_versions_for_document(&document.id)
            .map_err(DaemonError::store)?
        {
            if version.visibility != ResumeVisibility::Searchable {
                continue;
            }
            if embedding_text_for_version(&version).is_none() {
                continue;
            };
            let enqueued = store
                .enqueue_embedding_job_for_resume_version(
                    &document.id,
                    &version.id,
                    model_id,
                    dimension,
                    now,
                )
                .map_err(DaemonError::store)?;
            if embedding_job_is_retryable(&enqueued.job) {
                pending_jobs += 1;
                if pending_jobs == max_docs {
                    return Ok(pending_jobs);
                }
            }
        }
    }

    Ok(pending_jobs)
}

fn claim_embedding_jobs(
    store: &MetaStore,
    max_docs: usize,
    model_id: &str,
    dimension: usize,
    now: UnixTimestamp,
) -> Result<Vec<IngestJob>> {
    let mut jobs = Vec::new();
    while jobs.len() < max_docs {
        let Some(job) = store
            .claim_next_embedding_job(model_id, dimension, now)
            .map_err(DaemonError::store)?
        else {
            break;
        };
        jobs.push(job);
    }
    Ok(jobs)
}

fn embedding_candidate_for_job(
    store: &MetaStore,
    job: &IngestJob,
) -> Result<Option<EmbeddingWorkerCandidate>> {
    let Some(version_id) = job.resume_version_id.as_ref() else {
        return Ok(None);
    };
    let Some(document) = store
        .document_by_id(&job.document_id)
        .map_err(DaemonError::store)?
    else {
        return Ok(None);
    };
    if document.is_deleted
        || document.status == DocumentStatus::Deleted
        || !matches!(
            document.status,
            DocumentStatus::FieldsExtracted
                | DocumentStatus::EmbeddingDone
                | DocumentStatus::IndexedPartial
                | DocumentStatus::Searchable
        )
    {
        return Ok(None);
    }
    let Some(version) = store
        .resume_version_by_id(version_id)
        .map_err(DaemonError::store)?
    else {
        return Ok(None);
    };
    if version.document_id != document.id || version.visibility != ResumeVisibility::Searchable {
        return Ok(None);
    }
    let Some(text) = embedding_text_for_version(&version) else {
        return Ok(None);
    };
    Ok(Some(EmbeddingWorkerCandidate {
        document_id: document.id,
        version_id: version.id,
        text,
    }))
}

fn embedding_text_for_version(version: &ResumeVersion) -> Option<String> {
    version
        .clean_text
        .as_deref()
        .or(version.raw_text.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

#[derive(Clone, PartialEq, Eq)]
struct EmbeddingWorkerInput {
    document_id: DocumentId,
    input_id: String,
    text: String,
}

impl fmt::Debug for EmbeddingWorkerInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingWorkerInput")
            .field("document_id", &self.document_id)
            .field("input_id", &self.input_id)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

fn embedding_inputs_for_candidates(
    candidates: &[(IngestJob, EmbeddingWorkerCandidate)],
) -> Vec<EmbeddingWorkerInput> {
    let sectionizer = Sectionizer::default();
    candidates
        .iter()
        .flat_map(|(_, candidate)| embedding_inputs_for_candidate(candidate, &sectionizer))
        .collect()
}

fn embedding_inputs_for_candidate(
    candidate: &EmbeddingWorkerCandidate,
    sectionizer: &Sectionizer,
) -> Vec<EmbeddingWorkerInput> {
    let mut inputs = vec![EmbeddingWorkerInput {
        document_id: candidate.document_id.clone(),
        input_id: candidate.version_id.to_string(),
        text: candidate.text.clone(),
    }];
    let sections = sectionizer.sectionize(&candidate.text);
    let full_text = candidate.text.trim();
    let should_embed_sections = sections.len() > 1
        || sections
            .iter()
            .any(|section| section.text.trim() != full_text);

    if should_embed_sections {
        inputs.extend(sections.into_iter().filter_map(|section| {
            let text = section.text.trim();
            if text.is_empty() {
                return None;
            }

            Some(EmbeddingWorkerInput {
                document_id: candidate.document_id.clone(),
                input_id: section_embedding_input_id(&candidate.version_id, section.order_no),
                text: text.to_string(),
            })
        }));
    }

    inputs
}

fn section_embedding_input_id(version_id: &ResumeVersionId, order_no: u32) -> String {
    format!("{version_id}:section:{order_no}")
}

fn embedding_job_is_retryable(job: &IngestJob) -> bool {
    matches!(
        job.status,
        IngestJobStatus::Queued | IngestJobStatus::Interrupted
    ) || (job.status == IngestJobStatus::FailedRetryable && job.attempt_count < job.max_attempts)
}

fn mark_embedding_jobs_failed_retryable(
    store: &MetaStore,
    candidates: &[(IngestJob, EmbeddingWorkerCandidate)],
    now: UnixTimestamp,
) -> Result<()> {
    for (job, _) in candidates {
        store
            .update_job_status(&job.id, IngestJobStatus::FailedRetryable, now)
            .map_err(DaemonError::store)?;
    }
    Ok(())
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
        .map_err(|_| DaemonError::user("import watcher blocked: local watcher unavailable"))?;

        Ok(Self {
            watcher,
            receiver,
            watched_roots: BTreeSet::new(),
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

fn import_options_from_scope(scope: &ImportScanScope) -> Result<ImportOptions> {
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
    })
}

fn upsert_scope_summary(
    store: &MetaStore,
    mut scope: ImportScanScope,
    summary: ImportSummary,
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
) -> Result<()> {
    let listener = bind_ipc_listener(data_dir, addr)?;
    let result = serve_ipc_listener(data_dir, &listener, max_requests, options);
    remove_ipc_endpoint_manifest(data_dir);
    result
}

fn bind_ipc_listener(data_dir: &Path, addr: SocketAddr) -> Result<TcpListener> {
    let _ = load_or_create_ipc_auth_token(data_dir)?;
    let listener = TcpListener::bind(addr)
        .map_err(|_| DaemonError::user("unable to bind daemon ipc listener"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|_| DaemonError::user("unable to inspect daemon ipc listener"))?;
    write_ipc_endpoint_manifest(data_dir, local_addr)?;
    println!("ipc status endpoint: http://{local_addr}/status");
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::user("unable to write daemon status"))?;
    Ok(listener)
}

fn write_ipc_endpoint_manifest(data_dir: &Path, addr: SocketAddr) -> Result<()> {
    fs::create_dir_all(data_dir)
        .map_err(|_| DaemonError::user("unable to prepare daemon ipc auto-discovery"))?;
    let body = serde_json::json!({
        "schema_version": IPC_ENDPOINT_SCHEMA_VERSION,
        "status": format!("http://{addr}/status"),
        "imports": format!("http://{addr}/imports"),
        "import_cancel": format!("http://{addr}/imports/cancel"),
        "import_progress": format!("http://{addr}/imports/progress"),
        "search": format!("http://{addr}/search"),
        "details": format!("http://{addr}/details"),
    })
    .to_string();
    let path = data_dir.join(IPC_ENDPOINT_FILE);
    reject_unsafe_ipc_endpoint_manifest_path(&path)?;
    let temp_path = ipc_endpoint_temp_path(data_dir);
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp_path)
        .map_err(|_| DaemonError::user("unable to write daemon ipc auto-discovery"))?;
    file.write_all(body.as_bytes())
        .map_err(|_| DaemonError::user("unable to write daemon ipc auto-discovery"))?;
    file.flush()
        .map_err(|_| DaemonError::user("unable to write daemon ipc auto-discovery"))?;
    #[cfg(unix)]
    {
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600))
            .map_err(|_| DaemonError::user("unable to secure daemon ipc auto-discovery"))?;
    }
    if fs::rename(&temp_path, &path).is_err() {
        let _ = fs::remove_file(&temp_path);
        return Err(DaemonError::user(
            "unable to publish daemon ipc auto-discovery",
        ));
    }
    Ok(())
}

fn reject_unsafe_ipc_endpoint_manifest_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() || !file_type.is_file() {
                return Err(DaemonError::user(
                    "unable to secure daemon ipc auto-discovery",
                ));
            }
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(DaemonError::user(
            "unable to inspect daemon ipc auto-discovery",
        )),
    }
}

fn ipc_endpoint_temp_path(data_dir: &Path) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    data_dir.join(format!(
        ".ipc.endpoints.{}.{}.tmp",
        std::process::id(),
        unique
    ))
}

fn remove_ipc_endpoint_manifest(data_dir: &Path) {
    let _ = fs::remove_file(data_dir.join(IPC_ENDPOINT_FILE));
}

fn serve_ipc_listener(
    data_dir: &Path,
    listener: &TcpListener,
    max_requests: Option<usize>,
    options: &RunOptions,
) -> Result<()> {
    let request_limit = max_requests.unwrap_or(usize::MAX);
    let ipc_store = open_store(data_dir)?;
    for _ in 0..request_limit {
        let (stream, _) = listener
            .accept()
            .map_err(|_| DaemonError::user("unable to accept daemon ipc request"))?;
        handle_ipc_stream(data_dir, &ipc_store, stream, options)?;
    }

    Ok(())
}

fn serve_ipc_listener_with_worker_monitor(
    data_dir: &Path,
    ipc_store: &MetaStore,
    listener: &TcpListener,
    max_requests: Option<usize>,
    options: &RunOptions,
    worker_result_receiver: &Receiver<Result<()>>,
) -> Result<()> {
    let request_limit = max_requests.unwrap_or(usize::MAX);
    let mut handled_requests = 0_usize;

    while handled_requests < request_limit {
        match worker_result_receiver.try_recv() {
            Ok(Ok(())) => {
                return Err(DaemonError::user(
                    "import worker exited while daemon ipc was still running",
                ))
            }
            Ok(Err(error)) => return Err(error),
            Err(TryRecvError::Disconnected) => {
                return Err(DaemonError::user(
                    "import worker thread stopped unexpectedly",
                ))
            }
            Err(TryRecvError::Empty) => {}
        }

        match listener.accept() {
            Ok((stream, _)) => {
                handle_ipc_stream(data_dir, ipc_store, stream, options)?;
                handled_requests += 1;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return Err(DaemonError::user("unable to accept daemon ipc request")),
        }
    }

    Ok(())
}

fn handle_ipc_stream(
    data_dir: &Path,
    ipc_store: &MetaStore,
    mut stream: TcpStream,
    options: &RunOptions,
) -> Result<()> {
    stream
        .set_nonblocking(false)
        .map_err(|_| DaemonError::user("unable to configure daemon ipc stream"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| DaemonError::user("unable to set daemon ipc timeout"))?;
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
        let body = status_json(ipc_store)?;
        return write_http_response(&mut stream, 200, "application/json", &body);
    }

    if request.method == "POST"
        && request.path == "/imports"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_import_command_ipc(data_dir, &request, &mut stream);
    }

    if request.method == "POST"
        && request.path == "/imports/cancel"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_import_cancel_command_ipc(data_dir, &request, &mut stream);
    }

    if request.method == "GET"
        && request.path == "/imports/progress"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_import_progress_stream_ipc(data_dir, &request, &mut stream);
    }

    if request.method == "POST"
        && request.path == "/search"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_search_command_ipc(data_dir, &request, &mut stream, options);
    }

    if request.method == "POST"
        && request.path == "/details"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_detail_command_ipc(data_dir, &request, &mut stream);
    }

    write_http_response(&mut stream, 404, "text/plain", "not found")
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
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(data_dir, &request.headers)? {
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
        Err(IpcCommandError::Internal(error)) => Err(error),
    }
}

fn handle_import_cancel_command_ipc(
    data_dir: &Path,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(data_dir, &request.headers)? {
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
        Err(IpcCommandError::Internal(error)) => Err(error),
    }
}

fn handle_import_progress_stream_ipc(
    data_dir: &Path,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(data_dir, &request.headers)? {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(stream, 401, "application/json", &body);
    }

    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n",
        )
        .map_err(|_| DaemonError::user("unable to write daemon import progress stream"))?;
    for event_index in 0..IMPORT_PROGRESS_STREAM_EVENTS {
        let event = import_progress_stream_event_json(data_dir)?;
        stream
            .write_all(event.as_bytes())
            .and_then(|_| stream.write_all(b"\n"))
            .and_then(|_| stream.flush())
            .map_err(|_| DaemonError::user("unable to write daemon import progress stream"))?;
        if event_index + 1 < IMPORT_PROGRESS_STREAM_EVENTS {
            thread::sleep(Duration::from_millis(IMPORT_PROGRESS_STREAM_INTERVAL_MS));
        }
    }
    Ok(())
}

fn ipc_command_authorized(data_dir: &Path, headers: &[(String, String)]) -> Result<bool> {
    let expected = load_or_create_ipc_auth_token(data_dir)?;
    let Some(header) = header_value(headers, "authorization") else {
        return Ok(false);
    };
    let Some(token) = header.strip_prefix("Bearer ") else {
        return Ok(false);
    };

    Ok(constant_time_eq(
        token.trim().as_bytes(),
        expected.as_bytes(),
    ))
}

fn handle_search_command_ipc(
    data_dir: &Path,
    request: &IpcRequest,
    stream: &mut TcpStream,
    options: &RunOptions,
) -> Result<()> {
    if !ipc_command_authorized(data_dir, &request.headers)? {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(stream, 401, "application/json", &body);
    }

    match execute_search_command(data_dir, &request.body, options) {
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
        Err(IpcCommandError::NotFound(message)) => {
            let body = serde_json::json!({
                "schema_version": "daemon.error.v1",
                "status": "not_found",
                "message": message,
            })
            .to_string();
            write_http_response(stream, 404, "application/json", &body)
        }
        Err(IpcCommandError::Internal(error)) => Err(error),
    }
}

fn handle_detail_command_ipc(
    data_dir: &Path,
    request: &IpcRequest,
    stream: &mut TcpStream,
) -> Result<()> {
    if !ipc_command_authorized(data_dir, &request.headers)? {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(stream, 401, "application/json", &body);
    }

    match execute_detail_command(data_dir, &request.body) {
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
        Err(IpcCommandError::Internal(error)) => Err(error),
    }
}

fn execute_search_command(
    data_dir: &Path,
    body: &[u8],
    options: &RunOptions,
) -> std::result::Result<String, IpcCommandError> {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| IpcCommandError::BadRequest("invalid json"))?;
    let args = parse_search_command(&payload)?;
    let query_started = Instant::now();
    let store = open_store(data_dir).map_err(IpcCommandError::Internal)?;
    let hits = match args.mode {
        DaemonSearchMode::FullText => {
            let Some(index) = FullTextIndex::open_active(&data_dir.join("search-index"))
                .map_err(DaemonError::fulltext)
                .map_err(IpcCommandError::Internal)?
            else {
                return Ok(daemon_search_not_ready_body(args.mode));
            };
            daemon_fulltext_search(&index, &store, &args)?
        }
        DaemonSearchMode::Semantic => {
            let Some(hits) = daemon_semantic_search(data_dir, &store, &args, options)? else {
                return Ok(daemon_search_not_ready_body(args.mode));
            };
            hits
        }
        DaemonSearchMode::Hybrid => {
            let Some(index) = FullTextIndex::open_active(&data_dir.join("search-index"))
                .map_err(DaemonError::fulltext)
                .map_err(IpcCommandError::Internal)?
            else {
                return Ok(daemon_search_not_ready_body(args.mode));
            };
            let fulltext_hits = daemon_fulltext_search(&index, &store, &args)?;
            let Some(vector_hits) = daemon_semantic_search(data_dir, &store, &args, options)?
            else {
                return Ok(daemon_search_not_ready_body(args.mode));
            };
            daemon_fuse_hybrid_hits(fulltext_hits, vector_hits, args.top_k)
        }
    };
    record_daemon_query_observation(&store, args.mode, query_started.elapsed(), hits.len());
    Ok(daemon_search_ok_body(args.mode, &hits))
}

fn daemon_search_not_ready_body(mode: DaemonSearchMode) -> String {
    serde_json::json!({
        "schema_version": "daemon.search.v1",
        "status": "ok",
        "mode": mode.label(),
        "search_index": "not_ready",
        "result_count": 0,
        "results": [],
    })
    .to_string()
}

fn daemon_search_ok_body(mode: DaemonSearchMode, hits: &[DaemonSearchHit]) -> String {
    let results = hits
        .iter()
        .map(|hit| {
            let mut result = serde_json::json!({
                "rank": hit.rank,
                "doc_id": hit.doc_id,
                "version_id": hit.version_id,
                "file_name": hit.file_name,
                "snippet": hit.snippet,
            });
            if let Some(hint) = &hit.soft_dedupe_hint {
                result["soft_dedupe"] = serde_json::json!({
                    "suspected_versions": hint.suspected_versions,
                    "max_confidence": format_confidence(hint.max_confidence),
                    "folded": false,
                });
            }
            result
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "schema_version": "daemon.search.v1",
        "status": "ok",
        "mode": mode.label(),
        "search_index": "available",
        "result_count": results.len(),
        "results": results,
    })
    .to_string()
}

fn parse_search_command(
    payload: &serde_json::Value,
) -> std::result::Result<DaemonSearchArgs, IpcCommandError> {
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
    let filters = parse_search_filters(payload.get("filters"))?;
    Ok(DaemonSearchArgs {
        query,
        mode,
        top_k,
        filters,
    })
}

fn parse_search_filters(
    filters: Option<&serde_json::Value>,
) -> std::result::Result<SearchFilters, IpcCommandError> {
    let Some(filters) = filters else {
        return Ok(SearchFilters::default());
    };
    if filters.is_null() {
        return Ok(SearchFilters::default());
    }
    let Some(object) = filters.as_object() else {
        return Err(IpcCommandError::BadRequest("filters must be an object"));
    };

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
            parsed = parsed.with_years_experience_min(years);
        }
    }
    Ok(parsed)
}

fn daemon_fulltext_search(
    index: &FullTextIndex,
    store: &MetaStore,
    args: &DaemonSearchArgs,
) -> std::result::Result<Vec<DaemonSearchHit>, IpcCommandError> {
    let candidate_limit = args.top_k.saturating_mul(5).clamp(args.top_k, 100);
    let plan = plan_search(&args.query, candidate_limit)
        .map_err(|_| IpcCommandError::BadRequest("query must have searchable terms"))?;
    let allowed_doc_ids = daemon_field_filter_doc_id_prefilter(store, &args.filters)?;
    let query = SearchQuery::new(plan.query_text()).with_limit(plan.limit());
    let hits = match &allowed_doc_ids {
        Some(doc_ids) => index.search_allowed_doc_ids(query, doc_ids),
        None => index.search(query),
    }
    .map_err(DaemonError::fulltext)
    .map_err(IpcCommandError::Internal)?;
    daemon_visible_hits(store, hits, &args.filters, args.top_k)
}

fn daemon_semantic_search(
    data_dir: &Path,
    store: &MetaStore,
    args: &DaemonSearchArgs,
    options: &RunOptions,
) -> std::result::Result<Option<Vec<DaemonSearchHit>>, IpcCommandError> {
    let Some(snapshot_dimension) = daemon_vector_snapshot_dimension(data_dir) else {
        return Ok(None);
    };
    let command = options
        .embedding_command
        .clone()
        .ok_or(IpcCommandError::BadRequest(
            "semantic search blocked: local embedding command not configured",
        ))?;
    let model_id = options
        .embedding_model_id
        .as_deref()
        .ok_or(IpcCommandError::BadRequest(
            "semantic search blocked: embedding model id not configured",
        ))?;
    let dimension = options
        .embedding_dimension
        .ok_or(IpcCommandError::BadRequest(
            "semantic search blocked: embedding dimension not configured",
        ))?;
    if dimension != snapshot_dimension {
        return Err(IpcCommandError::BadRequest(
            "semantic search blocked: embedding dimension does not match vector index",
        ));
    }

    let embedder = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), model_id, dimension)
            .map_err(DaemonError::embedding)
            .map_err(IpcCommandError::Internal)?
            .with_timeout_ms(options.embedding_timeout_ms)
            .map_err(DaemonError::embedding)
            .map_err(IpcCommandError::Internal)?,
    );
    let input = EmbeddingInput::new("query", args.query.as_str());
    let query_vectors = embedder
        .embed_batch(&[input], EmbeddingBudget::new(1, args.query.len().max(1)))
        .map_err(DaemonError::embedding)
        .map_err(IpcCommandError::Internal)?;
    let query_vector = query_vectors
        .into_iter()
        .next()
        .ok_or(IpcCommandError::BadRequest(
            "semantic search query embedding is unavailable",
        ))?;
    let vector_index = PersistentVectorIndex::open(data_dir.join("vector-index"), dimension)
        .map_err(DaemonError::vector)
        .map_err(IpcCommandError::Internal)?;
    let candidate_limit = args.top_k.saturating_mul(5).clamp(args.top_k, 100);
    let allowed_doc_ids = daemon_field_filter_doc_id_prefilter(store, &args.filters)?;
    let vector_hits = vector_index
        .knn_for_model(
            QueryVector::new(query_vector.values().to_vec())
                .map_err(DaemonError::vector)
                .map_err(IpcCommandError::Internal)?,
            candidate_limit,
            model_id,
        )
        .map_err(DaemonError::vector)
        .map_err(IpcCommandError::Internal)?;

    daemon_vector_hits(
        store,
        vector_hits,
        &args.filters,
        allowed_doc_ids.as_ref(),
        args.top_k,
    )
    .map(Some)
}

fn daemon_vector_snapshot_dimension(data_dir: &Path) -> Option<usize> {
    let inspection = inspect_persistent_vector_snapshot(data_dir.join("vector-index"));
    match (inspection.state(), inspection.snapshot()) {
        (
            PersistentVectorSnapshotState::Ready | PersistentVectorSnapshotState::Recovered,
            Some(snapshot),
        ) => Some(snapshot.dimension()),
        _ => None,
    }
}

fn daemon_vector_hits(
    store: &MetaStore,
    hits: Vec<VectorHit>,
    filters: &SearchFilters,
    allowed_doc_ids: Option<&BTreeSet<String>>,
    top_k: usize,
) -> std::result::Result<Vec<DaemonSearchHit>, IpcCommandError> {
    let mut visible = Vec::new();
    let mut seen_candidate_keys = BTreeSet::new();

    for (rank, hit) in hits.into_iter().enumerate() {
        if let Some(allowed_doc_ids) = allowed_doc_ids {
            if !allowed_doc_ids.contains(hit.doc_id()) {
                continue;
            }
        }
        let Some((document, version)) =
            daemon_hydrate_visible_document_version(store, hit.doc_id())?
        else {
            continue;
        };
        if !filters.is_empty()
            && !filters.matches(&daemon_persisted_profile(store, hit.doc_id(), &version)?)
        {
            continue;
        }
        let candidate_key = daemon_candidate_fold_key(&version);
        if !seen_candidate_keys.insert(candidate_key.clone()) {
            continue;
        }

        visible.push(DaemonSearchHit {
            rank: rank + 1,
            score: hit.score(),
            doc_id: document.id.to_string(),
            version_id: version.id.to_string(),
            file_name: redact_contact_values(&document.file_name),
            snippet: "semantic match".to_string(),
            candidate_key,
            soft_dedupe_hint: None,
        });
        if visible.len() == top_k {
            break;
        }
    }

    daemon_attach_soft_dedupe_hints(store, daemon_rerank_search_hits(visible))
}

fn daemon_fuse_hybrid_hits(
    fulltext_hits: Vec<DaemonSearchHit>,
    vector_hits: Vec<DaemonSearchHit>,
    top_k: usize,
) -> Vec<DaemonSearchHit> {
    let mut by_doc = BTreeMap::<String, DaemonSearchHit>::new();
    for hit in vector_hits.iter().chain(fulltext_hits.iter()) {
        by_doc.insert(hit.doc_id.clone(), hit.clone());
    }
    let fused = fuse_hybrid_rrf(
        HybridRecall::new(
            daemon_ranked_hits_from_output(&fulltext_hits),
            daemon_ranked_hits_from_output(&vector_hits),
        ),
        60.0,
        top_k.saturating_mul(5).max(top_k),
    );
    let mut output = Vec::new();
    let mut seen_candidate_keys = BTreeSet::new();
    for ranked in fused {
        let Some(hit) = by_doc.get(ranked.doc_id()) else {
            continue;
        };
        if !seen_candidate_keys.insert(hit.candidate_key.clone()) {
            continue;
        }
        let mut hit = hit.clone();
        hit.rank = output.len() + 1;
        hit.score = ranked.score();
        output.push(hit);
        if output.len() == top_k {
            break;
        }
    }
    output
}

fn daemon_ranked_hits_from_output(hits: &[DaemonSearchHit]) -> Vec<RankedHit> {
    hits.iter()
        .enumerate()
        .map(|(index, hit)| {
            RankedHit::new(hit.doc_id.clone(), index + 1, hit.score)
                .with_candidate_key(hit.candidate_key.clone())
        })
        .collect()
}

fn daemon_field_filter_doc_id_prefilter(
    store: &MetaStore,
    filters: &SearchFilters,
) -> std::result::Result<Option<BTreeSet<String>>, IpcCommandError> {
    if filters.is_empty() {
        return Ok(None);
    }

    let mut allowed_doc_ids = None;
    if let Some(degree_min) = filters.degree_min() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_entity_values(
                    EntityType::Degree,
                    &daemon_degree_filter_values(degree_min),
                    FIELD_CONFIDENCE_THRESHOLD,
                    false,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.names_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_entity_values(
                    EntityType::Name,
                    filters.names_any(),
                    FIELD_CONFIDENCE_THRESHOLD,
                    true,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.school_tiers_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            daemon_school_tier_filter_doc_ids(store, filters.school_tiers_any())
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.schools_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_entity_values(
                    EntityType::School,
                    filters.schools_any(),
                    FIELD_CONFIDENCE_THRESHOLD,
                    true,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.majors_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_entity_values(
                    EntityType::Major,
                    filters.majors_any(),
                    FIELD_CONFIDENCE_THRESHOLD,
                    true,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.certificates_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_entity_values(
                    EntityType::Certificate,
                    filters.certificates_any(),
                    FIELD_CONFIDENCE_THRESHOLD,
                    true,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if let Some(date_range) = filters.date_range_overlaps() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_date_range_overlap(
                    date_range.start_month(),
                    date_range.end_month(),
                    FIELD_CONFIDENCE_THRESHOLD,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.companies_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_entity_values(
                    EntityType::Company,
                    filters.companies_any(),
                    FIELD_CONFIDENCE_THRESHOLD,
                    true,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.titles_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_entity_values(
                    EntityType::Title,
                    filters.titles_any(),
                    FIELD_CONFIDENCE_THRESHOLD,
                    true,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.locations_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_entity_values(
                    EntityType::Location,
                    filters.locations_any(),
                    FIELD_CONFIDENCE_THRESHOLD,
                    true,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.skills_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_entity_values(
                    EntityType::Skill,
                    filters.skills_any(),
                    FIELD_CONFIDENCE_THRESHOLD,
                    true,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if !filters.contact_hashes_any().is_empty() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_contact_hashes(&daemon_contact_hash_filter_values(
                    filters.contact_hashes_any(),
                )?)
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }
    if let Some(years_min) = filters.years_experience_min() {
        daemon_merge_filter_doc_ids(
            &mut allowed_doc_ids,
            store
                .searchable_document_ids_with_numeric_entity_min(
                    EntityType::YearsExperience,
                    years_min,
                    FIELD_CONFIDENCE_THRESHOLD,
                )
                .map_err(DaemonError::store)
                .map_err(IpcCommandError::Internal)?,
        );
    }

    Ok(allowed_doc_ids)
}

fn daemon_contact_hash_filter_values(
    contact_hashes: &[String],
) -> std::result::Result<Vec<ContactHash>, IpcCommandError> {
    contact_hashes
        .iter()
        .map(|contact_hash| {
            ContactHash::from_keyed_digest(contact_hash.clone()).map_err(|_| {
                IpcCommandError::BadRequest("contact_hashes_any values must be contact hashes")
            })
        })
        .collect()
}

fn daemon_school_tier_filter_doc_ids(
    store: &MetaStore,
    school_tiers: &[SchoolTier],
) -> meta_store::Result<Vec<DocumentId>> {
    let known_values = school_tiers
        .iter()
        .filter(|school_tier| **school_tier != SchoolTier::Unknown)
        .map(|school_tier| school_tier.canonical().to_string())
        .collect::<Vec<_>>();
    let mut document_ids = Vec::new();
    if !known_values.is_empty() {
        document_ids.extend(store.searchable_document_ids_with_entity_values(
            EntityType::SchoolTier,
            &known_values,
            FIELD_CONFIDENCE_THRESHOLD,
            false,
        )?);
    }
    if school_tiers.contains(&SchoolTier::Unknown) {
        document_ids.extend(store.searchable_document_ids_without_entity_type(
            EntityType::SchoolTier,
            FIELD_CONFIDENCE_THRESHOLD,
        )?);
    }
    Ok(document_ids)
}

fn daemon_merge_filter_doc_ids(current: &mut Option<BTreeSet<String>>, next: Vec<DocumentId>) {
    let next = next
        .into_iter()
        .map(|document_id| document_id.to_string())
        .collect::<BTreeSet<_>>();
    match current {
        Some(current) => {
            *current = current.intersection(&next).cloned().collect();
        }
        None => *current = Some(next),
    }
}

fn daemon_degree_filter_values(min_degree: DegreeLevel) -> Vec<String> {
    [
        DegreeLevel::HighSchool,
        DegreeLevel::Associate,
        DegreeLevel::Bachelor,
        DegreeLevel::Master,
        DegreeLevel::Doctor,
    ]
    .into_iter()
    .filter(|degree| *degree >= min_degree)
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

fn daemon_visible_hits(
    store: &MetaStore,
    hits: Vec<SearchHit>,
    filters: &SearchFilters,
    top_k: usize,
) -> std::result::Result<Vec<DaemonSearchHit>, IpcCommandError> {
    let mut visible = Vec::new();
    let mut seen_candidate_keys = BTreeSet::new();

    for hit in hits {
        let Some(version) = daemon_hydrate_visible_version(store, &hit)? else {
            continue;
        };
        if !filters.is_empty()
            && !filters.matches(&daemon_persisted_profile(store, &hit.doc_id, &version)?)
        {
            continue;
        }
        let candidate_key = daemon_candidate_fold_key(&version);
        if !seen_candidate_keys.insert(candidate_key.clone()) {
            continue;
        }

        visible.push(DaemonSearchHit {
            rank: visible.len() + 1,
            score: hit.score,
            doc_id: hit.doc_id,
            version_id: hit.version_id,
            file_name: redact_contact_values(&hit.file_name),
            snippet: redact_contact_values(&hit.snippet),
            candidate_key,
            soft_dedupe_hint: None,
        });
        if visible.len() == top_k {
            break;
        }
    }

    daemon_attach_soft_dedupe_hints(store, visible)
}

fn daemon_rerank_search_hits(mut hits: Vec<DaemonSearchHit>) -> Vec<DaemonSearchHit> {
    for (index, hit) in hits.iter_mut().enumerate() {
        hit.rank = index + 1;
    }
    hits
}

fn daemon_attach_soft_dedupe_hints(
    store: &MetaStore,
    mut hits: Vec<DaemonSearchHit>,
) -> std::result::Result<Vec<DaemonSearchHit>, IpcCommandError> {
    let hints = hits
        .iter()
        .map(|hit| daemon_soft_dedupe_hint_for_hit(store, hit))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (hit, hint) in hits.iter_mut().zip(hints) {
        hit.soft_dedupe_hint = hint;
    }
    Ok(hits)
}

fn daemon_soft_dedupe_hint_for_hit(
    store: &MetaStore,
    hit: &DaemonSearchHit,
) -> std::result::Result<Option<DaemonSoftDedupeHint>, IpcCommandError> {
    if hit.candidate_key.starts_with("candidate:") {
        return Ok(None);
    }
    let Some(profile) = daemon_dedupe_profile_for_hit(store, hit)? else {
        return Ok(None);
    };
    let Some(name) = profile.name() else {
        return Ok(None);
    };
    let candidate_doc_ids = store
        .searchable_document_ids_with_entity_values(
            EntityType::Name,
            &[name.to_string()],
            FIELD_CONFIDENCE_THRESHOLD,
            true,
        )
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?;
    let mut suspected_versions = 0_usize;
    let mut max_confidence = 0.0_f32;

    for candidate_doc_id in candidate_doc_ids.into_iter().take(64) {
        if candidate_doc_id.as_str() == hit.doc_id {
            continue;
        }
        let versions = store
            .resume_versions_for_document(&candidate_doc_id)
            .map_err(DaemonError::store)
            .map_err(IpcCommandError::Internal)?;
        for version in versions {
            if version.id.as_str() == hit.version_id
                || version.visibility != ResumeVisibility::Searchable
                || version.candidate_id.is_some()
            {
                continue;
            }
            let other_hit = DaemonSearchHit {
                rank: 0,
                score: 0.0,
                doc_id: version.document_id.to_string(),
                version_id: version.id.to_string(),
                file_name: String::new(),
                snippet: String::new(),
                candidate_key: daemon_candidate_fold_key(&version),
                soft_dedupe_hint: None,
            };
            let Some(other_profile) = daemon_dedupe_profile_for_hit(store, &other_hit)? else {
                continue;
            };
            if let Some(score) = soft_dedupe_score(&profile, &other_profile) {
                suspected_versions += 1;
                max_confidence = max_confidence.max(score.confidence());
            }
        }
    }

    Ok((suspected_versions > 0).then_some(DaemonSoftDedupeHint {
        suspected_versions,
        max_confidence,
    }))
}

fn daemon_dedupe_profile_for_hit(
    store: &MetaStore,
    hit: &DaemonSearchHit,
) -> std::result::Result<Option<DedupeProfile>, IpcCommandError> {
    let Ok(version_id) = ResumeVersionId::from_str(&hit.version_id) else {
        return Ok(None);
    };
    let Some(version) = store
        .resume_version_by_id(&version_id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
    else {
        return Ok(None);
    };
    if version.document_id.as_str() != hit.doc_id || version.candidate_id.is_some() {
        return Ok(None);
    }
    let mentions = store
        .entity_mentions_for_version(&version.id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?;
    let Some(name) = daemon_best_normalized_entity_value(&mentions, EntityType::Name) else {
        return Ok(None);
    };
    let profile = DedupeProfile::new(hit.doc_id.clone())
        .with_name(&name)
        .with_schools(daemon_normalized_entity_values(
            &mentions,
            EntityType::School,
        ))
        .with_companies(daemon_normalized_entity_values(
            &mentions,
            EntityType::Company,
        ))
        .with_skills(daemon_normalized_entity_values(
            &mentions,
            EntityType::Skill,
        ));

    Ok(Some(profile))
}

fn daemon_best_normalized_entity_value(
    mentions: &[EntityMention],
    entity_type: EntityType,
) -> Option<String> {
    mentions
        .iter()
        .filter(|mention| {
            mention.entity_type == entity_type && mention.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|mention| {
            Some((
                mention.normalized_value.as_deref()?.to_string(),
                mention.confidence,
                mention.span_start.unwrap_or(usize::MAX),
            ))
        })
        .max_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| right.0.cmp(&left.0))
        })
        .map(|candidate| candidate.0)
}

fn daemon_normalized_entity_values(
    mentions: &[EntityMention],
    entity_type: EntityType,
) -> Vec<String> {
    mentions
        .iter()
        .filter(|mention| {
            mention.entity_type == entity_type && mention.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|mention| mention.normalized_value.as_deref())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn daemon_hydrate_visible_version(
    store: &MetaStore,
    hit: &SearchHit,
) -> std::result::Result<Option<ResumeVersion>, IpcCommandError> {
    let Ok(document_id) = DocumentId::from_str(&hit.doc_id) else {
        return Ok(None);
    };
    let Some(document) = store
        .document_by_id(&document_id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
    else {
        return Ok(None);
    };
    if document.is_deleted
        || !matches!(
            document.status,
            DocumentStatus::Searchable | DocumentStatus::IndexedPartial
        )
    {
        return Ok(None);
    }

    let Ok(version_id) = ResumeVersionId::from_str(&hit.version_id) else {
        return Ok(None);
    };
    let Some(version) = store
        .resume_version_by_id(&version_id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
    else {
        return Ok(None);
    };
    if version.document_id != document_id || version.visibility != ResumeVisibility::Searchable {
        return Ok(None);
    }

    Ok(Some(version))
}

fn daemon_hydrate_visible_document_version(
    store: &MetaStore,
    doc_id: &str,
) -> std::result::Result<Option<(Document, ResumeVersion)>, IpcCommandError> {
    let Ok(document_id) = DocumentId::from_str(doc_id) else {
        return Ok(None);
    };
    let Some(document) = store
        .document_by_id(&document_id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
    else {
        return Ok(None);
    };
    if document.is_deleted
        || !matches!(
            document.status,
            DocumentStatus::Searchable | DocumentStatus::IndexedPartial
        )
    {
        return Ok(None);
    }
    let Some(version) = store
        .latest_visible_resume_version_for_document(&document_id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
    else {
        return Ok(None);
    };
    if version.visibility != ResumeVisibility::Searchable {
        return Ok(None);
    }
    Ok(Some((document, version)))
}

fn daemon_persisted_profile(
    store: &MetaStore,
    doc_id: &str,
    version: &ResumeVersion,
) -> std::result::Result<ResumeProfile, IpcCommandError> {
    let fields = store
        .entity_mentions_for_version(&version.id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?;
    let names = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::Name && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let degree = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::Degree
                && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| DegreeLevel::parse(field.normalized_value.as_deref()?))
        .max();
    let skills = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::Skill && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let certificates = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::Certificate
                && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let date_ranges = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::DateRange
                && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let schools = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::School
                && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let majors = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::Major && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let companies = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::Company
                && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let titles = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::Title && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let locations = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::Location
                && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let school_tiers = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::SchoolTier
                && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| SchoolTier::parse(field.normalized_value.as_deref()?))
        .collect::<Vec<_>>();
    let years_experience = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::YearsExperience
                && field.confidence >= FIELD_CONFIDENCE_THRESHOLD
        })
        .filter_map(|field| field.normalized_value.as_deref()?.parse::<f32>().ok())
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));

    let mut profile = ResumeProfile::new(doc_id)
        .with_names(names)
        .with_school_tiers(school_tiers)
        .with_schools(schools)
        .with_majors(majors)
        .with_certificates(certificates)
        .with_date_ranges(date_ranges)
        .with_companies(companies)
        .with_titles(titles)
        .with_locations(locations)
        .with_skills(skills);
    if let Some(degree) = degree {
        profile = profile.with_degree(degree);
    }
    if let Some(years_experience) = years_experience {
        profile = profile.with_years_experience(years_experience);
    }
    Ok(profile)
}

fn execute_detail_command(
    data_dir: &Path,
    body: &[u8],
) -> std::result::Result<String, IpcCommandError> {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| IpcCommandError::BadRequest("invalid json"))?;
    let args = parse_detail_command(&payload)?;
    let store = open_store(data_dir).map_err(IpcCommandError::Internal)?;
    let detail = build_daemon_resume_detail(&store, &args.document_id)?
        .ok_or(IpcCommandError::NotFound("detail document was not found"))?;
    let fields = detail
        .fields
        .iter()
        .map(|field| {
            serde_json::json!({
                "type": field.field_type,
                "value": field.value,
                "confidence": field.confidence,
                "evidence": field.evidence,
                "extractor": field.extractor,
            })
        })
        .collect::<Vec<_>>();
    let body = serde_json::json!({
        "schema_version": "daemon.detail.v1",
        "status": "ok",
        "document": {
            "doc_id": detail.doc_id,
            "version_id": detail.version_id,
            "file_name": detail.file_name,
            "extension": detail.extension,
            "document_status": detail.document_status,
            "visibility": detail.visibility,
            "byte_size": detail.byte_size,
            "fields": fields,
            "snippet": detail.snippet,
        }
    });
    Ok(body.to_string())
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

fn parse_detail_command(
    payload: &serde_json::Value,
) -> std::result::Result<DaemonDetailArgs, IpcCommandError> {
    let doc_id = payload
        .get("doc_id")
        .and_then(serde_json::Value::as_str)
        .filter(|doc_id| !doc_id.trim().is_empty())
        .ok_or(IpcCommandError::BadRequest(
            "doc_id must be a non-empty string",
        ))?;
    let document_id = DocumentId::from_str(doc_id)
        .map_err(|_| IpcCommandError::BadRequest("doc_id is invalid"))?;
    Ok(DaemonDetailArgs { document_id })
}

fn build_daemon_resume_detail(
    store: &MetaStore,
    document_id: &DocumentId,
) -> std::result::Result<Option<DaemonResumeDetail>, IpcCommandError> {
    let Some(document) = store
        .document_by_id(document_id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
    else {
        return Ok(None);
    };
    if document.is_deleted || document.status == DocumentStatus::Deleted {
        return Ok(None);
    }
    let Some(version) = select_daemon_detail_version(store, document_id)? else {
        return Ok(None);
    };
    let fields = store
        .entity_mentions_for_version(&version.id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?
        .iter()
        .map(daemon_detail_field_from_mention)
        .collect::<Vec<_>>();
    let snippet = version
        .clean_text
        .as_deref()
        .or(version.raw_text.as_deref())
        .map(|text| redact_short_text(text, 240))
        .unwrap_or_else(|| "none".to_string());

    Ok(Some(DaemonResumeDetail {
        doc_id: document.id.to_string(),
        version_id: version.id.to_string(),
        file_name: redact_short_text(&document.file_name, 160),
        extension: file_extension_label(&document.extension).to_string(),
        document_status: document_status_label(document.status).to_string(),
        visibility: resume_visibility_label(version.visibility).to_string(),
        byte_size: document.byte_size,
        fields,
        snippet,
    }))
}

fn select_daemon_detail_version(
    store: &MetaStore,
    document_id: &DocumentId,
) -> std::result::Result<Option<ResumeVersion>, IpcCommandError> {
    store
        .latest_visible_resume_version_for_document(document_id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)
}

fn daemon_detail_field_from_mention(mention: &EntityMention) -> DaemonResumeDetailField {
    let value = mention
        .normalized_value
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&mention.raw_value);
    DaemonResumeDetailField {
        field_type: entity_type_label(&mention.entity_type),
        value: redact_short_text(value, 120),
        confidence: f64::from(mention.confidence.clamp(0.0, 1.0)),
        evidence: redact_short_text(&mention.raw_value, 120),
        extractor: redact_short_text(&mention.extractor, 80),
    }
}

fn redact_short_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = redact_contact_values(&compact);
    truncate_chars(&redacted, max_chars)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index == max_chars {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    output
}

fn daemon_candidate_fold_key(version: &ResumeVersion) -> String {
    version
        .candidate_id
        .as_ref()
        .map(|candidate_id| format!("candidate:{}", candidate_id.as_str()))
        .unwrap_or_else(|| format!("doc:{}", version.document_id.as_str()))
}

fn format_confidence(value: f32) -> f64 {
    f64::from((value.clamp(0.0, 1.0) * 100.0).round() / 100.0)
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

fn load_or_create_ipc_auth_token(data_dir: &Path) -> Result<String> {
    fs::create_dir_all(data_dir)
        .map_err(|_| DaemonError::user("unable to prepare local metadata directory"))?;
    let path = data_dir.join(IPC_AUTH_TOKEN_FILE);
    match fs::read_to_string(&path) {
        Ok(token) => {
            ensure_ipc_auth_token_permissions(&path)?;
            validate_ipc_auth_token(&token)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_ipc_auth_token(&path),
        Err(_) => Err(DaemonError::user("unable to read daemon ipc auth token")),
    }
}

fn create_ipc_auth_token(path: &Path) -> Result<String> {
    let token = random_hex_token()?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    match options.open(path) {
        Ok(mut file) => {
            file.write_all(token.as_bytes())
                .and_then(|_| file.write_all(b"\n"))
                .map_err(|_| DaemonError::user("unable to write daemon ipc auth token"))?;
            Ok(token)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            ensure_ipc_auth_token_permissions(path)?;
            let token = fs::read_to_string(path)
                .map_err(|_| DaemonError::user("unable to read daemon ipc auth token"))?;
            validate_ipc_auth_token(&token)
        }
        Err(_) => Err(DaemonError::user("unable to create daemon ipc auth token")),
    }
}

#[cfg(unix)]
fn ensure_ipc_auth_token_permissions(path: &Path) -> Result<()> {
    let permissions = fs::metadata(path)
        .map_err(|_| DaemonError::user("unable to inspect daemon ipc auth token"))?
        .permissions();
    if permissions.mode() & 0o077 == 0 {
        return Ok(());
    }

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| DaemonError::user("unable to secure daemon ipc auth token"))?;
    let repaired = fs::metadata(path)
        .map_err(|_| DaemonError::user("unable to inspect daemon ipc auth token"))?
        .permissions();
    if repaired.mode() & 0o077 != 0 {
        return Err(DaemonError::user(
            "daemon ipc auth token permissions are unsafe",
        ));
    }

    Ok(())
}

#[cfg(not(unix))]
fn ensure_ipc_auth_token_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn validate_ipc_auth_token(token: &str) -> Result<String> {
    let token = token.trim();
    if token.len() != IPC_AUTH_TOKEN_BYTES * 2
        || !token.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DaemonError::user("daemon ipc auth token is invalid"));
    }
    Ok(token.to_string())
}

fn random_hex_token() -> Result<String> {
    let mut bytes = [0_u8; IPC_AUTH_TOKEN_BYTES];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| DaemonError::user("unable to create daemon ipc auth token"))?;
    let mut token = String::with_capacity(IPC_AUTH_TOKEN_BYTES * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}")
            .map_err(|_| DaemonError::user("unable to create daemon ipc auth token"))?;
    }
    Ok(token)
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
    filters: SearchFilters,
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
}

#[derive(Clone)]
struct DaemonSearchHit {
    rank: usize,
    score: f32,
    doc_id: String,
    version_id: String,
    file_name: String,
    snippet: String,
    candidate_key: String,
    soft_dedupe_hint: Option<DaemonSoftDedupeHint>,
}

#[derive(Clone)]
struct DaemonSoftDedupeHint {
    suspected_versions: usize,
    max_confidence: f32,
}

struct DaemonDetailArgs {
    document_id: DocumentId,
}

struct DaemonResumeDetail {
    doc_id: String,
    version_id: String,
    file_name: String,
    extension: String,
    document_status: String,
    visibility: String,
    byte_size: u64,
    fields: Vec<DaemonResumeDetailField>,
    snippet: String,
}

struct DaemonResumeDetailField {
    field_type: String,
    value: String,
    confidence: f64,
    evidence: String,
    extractor: String,
}

fn status_json(store: &MetaStore) -> Result<String> {
    retry_ipc_metadata_read(|| status_json_once(store))
}

fn status_json_once(store: &MetaStore) -> Result<String> {
    let summary = store.status_summary().map_err(DaemonError::store)?;
    let latest_import_scan = store
        .latest_import_scan_scope()
        .map_err(DaemonError::store)?
        .map(|scope| latest_import_scan_json(&scope))
        .unwrap_or(serde_json::Value::Null);
    let body = serde_json::json!({
        "schema_version": "daemon.status.v1",
        "status": "ok",
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

fn retry_ipc_metadata_read<T>(mut read: impl FnMut() -> Result<T>) -> Result<T> {
    let mut last_error = None;
    for attempt in 1..=IPC_METADATA_READ_ATTEMPTS {
        match read() {
            Ok(value) => return Ok(value),
            Err(error)
                if error.is_metadata_store_storage_error()
                    && attempt < IPC_METADATA_READ_ATTEMPTS =>
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
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status_code} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .map_err(|_| DaemonError::user("unable to write daemon ipc response"))
}

#[derive(Clone)]
struct RunOptions {
    foreground: bool,
    once: bool,
    ipc_listen: Option<SocketAddr>,
    max_requests: Option<usize>,
    work_imports_once: bool,
    work_imports: bool,
    rescan_completed_imports: bool,
    watch_import_roots: bool,
    import_rescan_min_age_seconds: Option<i64>,
    stale_import_task_seconds: Option<i64>,
    import_retry_backoff_seconds: Option<i64>,
    work_ocr_once: bool,
    work_ocr: bool,
    work_embeddings_once: bool,
    work_embeddings: bool,
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
    embedding_command: Option<PathBuf>,
    embedding_model_id: Option<String>,
    embedding_dimension: Option<usize>,
    embedding_max_docs: usize,
    embedding_max_text_bytes: usize,
    embedding_timeout_ms: u64,
    worker_interval_ms: Option<u64>,
    max_worker_ticks: Option<usize>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            foreground: false,
            once: false,
            ipc_listen: None,
            max_requests: None,
            work_imports_once: false,
            work_imports: false,
            rescan_completed_imports: false,
            watch_import_roots: false,
            import_rescan_min_age_seconds: None,
            stale_import_task_seconds: None,
            import_retry_backoff_seconds: None,
            work_ocr_once: false,
            work_ocr: false,
            work_embeddings_once: false,
            work_embeddings: false,
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
            embedding_command: None,
            embedding_model_id: None,
            embedding_dimension: None,
            embedding_max_docs: DEFAULT_EMBEDDING_MAX_DOCS,
            embedding_max_text_bytes: DEFAULT_EMBEDDING_MAX_TEXT_BYTES,
            embedding_timeout_ms: DEFAULT_EMBEDDING_TIMEOUT_MS,
            worker_interval_ms: None,
            max_worker_ticks: None,
        }
    }
}

impl RunOptions {
    fn has_worker_loop(&self) -> bool {
        self.work_imports || self.work_ocr || self.work_embeddings || self.work_index
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
        .map_err(|_| DaemonError::user("unable to write daemon status"))
}

#[derive(Default)]
struct OcrWorkerSummary {
    paused: bool,
    processed: usize,
    failed: usize,
    cache_writes: usize,
    cache_hits: usize,
}

impl OcrWorkerSummary {
    fn has_activity(&self) -> bool {
        self.paused
            || self.processed > 0
            || self.failed > 0
            || self.cache_writes > 0
            || self.cache_hits > 0
    }
}

fn print_ocr_worker_summary(summary: &OcrWorkerSummary) -> Result<()> {
    println!("ocr worker paused: {}", summary.paused);
    println!("ocr worker processed: {}", summary.processed);
    println!("ocr worker cache writes: {}", summary.cache_writes);
    println!("ocr worker cache hits: {}", summary.cache_hits);
    println!("ocr worker failed: {}", summary.failed);
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::user("unable to write daemon status"))
}

#[derive(Clone, PartialEq, Eq)]
struct EmbeddingWorkerCandidate {
    document_id: DocumentId,
    version_id: ResumeVersionId,
    text: String,
}

impl fmt::Debug for EmbeddingWorkerCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddingWorkerCandidate")
            .field("document_id", &self.document_id)
            .field("version_id", &self.version_id)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

#[derive(Default)]
struct EmbeddingWorkerSummary {
    documents_considered: usize,
    processed: usize,
    vector_writes: usize,
    failed: usize,
}

impl EmbeddingWorkerSummary {
    fn has_activity(&self) -> bool {
        self.documents_considered > 0
            || self.processed > 0
            || self.vector_writes > 0
            || self.failed > 0
    }
}

fn print_embedding_worker_summary(summary: &EmbeddingWorkerSummary) -> Result<()> {
    println!(
        "embedding worker documents considered: {}",
        summary.documents_considered
    );
    println!("embedding worker processed: {}", summary.processed);
    println!("embedding worker vector writes: {}", summary.vector_writes);
    println!("embedding worker failed: {}", summary.failed);
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::user("unable to write daemon status"))
}

#[derive(Default)]
struct IndexWorkerSummary {
    rebuilt: bool,
    indexed_documents: usize,
}

impl IndexWorkerSummary {
    fn has_activity(&self) -> bool {
        self.rebuilt
    }
}

fn print_index_worker_summary(summary: &IndexWorkerSummary) -> Result<()> {
    println!(
        "index worker rebuilt: {}",
        if summary.rebuilt { "yes" } else { "no" }
    );
    println!(
        "index worker indexed documents: {}",
        summary.indexed_documents
    );
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::user("unable to write daemon status"))
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
        .map_err(|_| DaemonError::user("unable to prepare local metadata directory"))?;
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

fn file_extension_label(extension: &FileExtension) -> &str {
    match extension {
        FileExtension::Docx => "docx",
        FileExtension::Pdf => "pdf",
        FileExtension::Doc => "doc",
        FileExtension::Txt => "txt",
        FileExtension::Image => "image",
        FileExtension::Other(_) => "other",
    }
}

fn document_status_label(status: DocumentStatus) -> &'static str {
    match status {
        DocumentStatus::Discovered => "discovered",
        DocumentStatus::Fingerprinted => "fingerprinted",
        DocumentStatus::ParseQueued => "parse_queued",
        DocumentStatus::ParseRunning => "parse_running",
        DocumentStatus::TextExtracted => "text_extracted",
        DocumentStatus::OcrRequired => "ocr_required",
        DocumentStatus::OcrRunning => "ocr_running",
        DocumentStatus::OcrDone => "ocr_done",
        DocumentStatus::TextCleaned => "text_cleaned",
        DocumentStatus::FieldsExtracted => "fields_extracted",
        DocumentStatus::EmbeddingDone => "embedding_done",
        DocumentStatus::IndexedPartial => "indexed_partial",
        DocumentStatus::Searchable => "searchable",
        DocumentStatus::FailedRetryable => "failed_retryable",
        DocumentStatus::FailedPermanent => "failed_permanent",
        DocumentStatus::Deleted => "deleted",
    }
}

fn resume_visibility_label(visibility: ResumeVisibility) -> &'static str {
    match visibility {
        ResumeVisibility::Searchable => "searchable",
        ResumeVisibility::Partial => "partial",
        ResumeVisibility::Hidden => "hidden",
    }
}

fn entity_type_label(entity_type: &EntityType) -> String {
    match entity_type {
        EntityType::Name => "name".to_string(),
        EntityType::Email => "email".to_string(),
        EntityType::Phone => "phone".to_string(),
        EntityType::School => "school".to_string(),
        EntityType::SchoolTier => "school_tier".to_string(),
        EntityType::Degree => "degree".to_string(),
        EntityType::Major => "major".to_string(),
        EntityType::Company => "company".to_string(),
        EntityType::Title => "title".to_string(),
        EntityType::Education => "education".to_string(),
        EntityType::Skills => "skills".to_string(),
        EntityType::Skill => "skill".to_string(),
        EntityType::Certificate => "certificate".to_string(),
        EntityType::Date => "date".to_string(),
        EntityType::DateRange => "date_range".to_string(),
        EntityType::YearsExperience => "years_experience".to_string(),
        EntityType::Location => "location".to_string(),
        EntityType::Other(_) => "other".to_string(),
    }
}

type Result<T> = std::result::Result<T, DaemonError>;

#[derive(Debug)]
struct DaemonError {
    message: String,
    exit_code: i32,
}

impl DaemonError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 2,
        }
    }

    fn user(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
        }
    }

    fn store(error: meta_store::MetaStoreError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn fulltext(_error: index_fulltext::FullTextError) -> Self {
        Self {
            message: "search index operation failed".to_string(),
            exit_code: 1,
        }
    }

    fn import(error: import_pipeline::ImportPipelineError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn ocr(error: ocr_client::OcrError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn embedding(error: embedder::EmbeddingError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn vector(error: index_vector::VectorIndexError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn exit_code(&self) -> i32 {
        self.exit_code
    }

    fn is_metadata_store_storage_error(&self) -> bool {
        self.message == "metadata store operation failed"
    }
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
