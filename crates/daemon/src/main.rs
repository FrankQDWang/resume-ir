use std::collections::BTreeSet;
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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use embedder::{
    Embedder, EmbeddingBudget, EmbeddingInput, LocalEmbeddingCommandEmbedder,
    LocalEmbeddingCommandSpec,
};
use import_pipeline::{
    import_root_with_options, index_ocr_text, ImportOptions,
    ImportScanBudgetKind as PipelineImportScanBudgetKind, ImportSummary, ScanProfile,
};
use index_fulltext::{redact_contact_values, FullTextIndex, SearchHit, SearchQuery};
use index_vector::{PersistentVectorIndex, VectorDocument, VectorIndex};
use meta_store::{
    DocumentId, DocumentStatus, EntityMention, EntityType, FileExtension, ImportRootKind,
    ImportRootPreset, ImportScanBudgetKind, ImportScanProfile, ImportScanScope, ImportTask,
    ImportTaskId, ImportTaskStatus, IndexStateStatus, IngestJob, IngestJobKind, IngestJobStatus,
    MetaStore, OcrPageCacheEntry, OcrPageCacheKey, OcrPageCacheStatus, ResumeVersion,
    ResumeVersionId, ResumeVisibility, UnixTimestamp, WorkerTaskKind,
};
use ocr_client::{
    CancellationToken, LocalOcrCommandClient, LocalOcrCommandSpec, OcrClient, OcrOptions,
    OcrPageRequest, OcrWorkerBudget, RenderedPage,
};
use rank_fusion::{DegreeLevel, ResumeProfile, SearchFilters};
use search_planner::plan_search;

const IMPORT_RETRY_BACKOFF_SECONDS: i64 = 60;
const IMPORT_TASK_HEARTBEAT_SECONDS: u64 = 30;
const STALE_IMPORT_TASK_SECONDS: i64 = 15 * 60;
const IPC_AUTH_TOKEN_FILE: &str = "ipc.auth";
const IPC_AUTH_TOKEN_BYTES: usize = 32;
const IPC_MAX_REQUEST_BYTES: usize = 64 * 1024;
const DEFAULT_OCR_ENGINE_PROFILE: &str = "local-command";
const DEFAULT_OCR_LANG: &str = "eng";
const DEFAULT_OCR_PROFILE: &str = "balanced";
const DEFAULT_OCR_RENDER_DPI: u32 = 300;
const DEFAULT_OCR_PAGE_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_EMBEDDING_MAX_DOCS: usize = 64;
const DEFAULT_EMBEDDING_MAX_TEXT_BYTES: usize = 1_000_000;
const DEFAULT_EMBEDDING_TIMEOUT_MS: u64 = 30_000;

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
    if (options.worker_interval_ms.is_some() || options.max_worker_ticks.is_some())
        && !(options.work_imports || options.work_ocr || options.work_embeddings)
    {
        return Err(DaemonError::usage(
            "usage: worker loop options require --work-imports, --work-ocr, or --work-embeddings",
        ));
    }
    if (options.work_imports || options.work_ocr || options.work_embeddings)
        && options.ipc_listen.is_some()
        && options.max_worker_ticks.is_some()
    {
        return Err(DaemonError::usage(
            "usage: --max-worker-ticks cannot be combined with --ipc-listen",
        ));
    }
    if (options.work_ocr_once || options.work_ocr) && options.ocr_command.is_none() {
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

    if options.once {
        return Ok(());
    }
    if (options.work_imports || options.work_ocr || options.work_embeddings)
        && options.ipc_listen.is_some()
    {
        run_worker_with_ipc(data_dir, &options)?;
        return Ok(());
    }
    if options.work_imports || options.work_ocr || options.work_embeddings {
        run_worker_loop(data_dir, &store, &options, None)?;
        return Ok(());
    }
    if let Some(ipc_addr) = options.ipc_listen {
        serve_ipc(data_dir, ipc_addr, options.max_requests)?;
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

    Ok(options)
}

fn run_usage() -> &'static str {
    "usage: resume-daemon run --foreground [--once] [--work-imports-once|--work-imports] [--work-ocr-once|--work-ocr] [--work-embeddings-once|--work-embeddings] [--ocr-command <path>] [--ocr-engine-profile <name>] [--ocr-lang <lang>] [--ocr-profile <profile>] [--ocr-render-dpi <dpi>] [--ocr-page-timeout-ms <ms>] [--embedding-command <path>] [--embedding-model-id <id>] [--embedding-dimension <n>] [--embedding-max-docs <n>] [--embedding-max-text-bytes <bytes>] [--embedding-timeout-ms <ms>] [--worker-interval-ms <n>] [--max-worker-ticks <n>] [--ipc-listen <127.0.0.1:port>] [--max-requests <n>]"
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
        &listener,
        options.max_requests,
        &worker_result_receiver,
    );
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
                stale_recovered: recover_stale_import_tasks(store, now)?,
                ..ImportWorkerSummary::default()
            };
            import_summary.extend(run_import_worker_once_with_retry_due(
                data_dir,
                store,
                timestamp_minus_seconds(now, IMPORT_RETRY_BACKOFF_SECONDS),
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
                worker_summary.failed += 1;
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

    let Some(command) = options.ocr_command.clone() else {
        return Err(DaemonError::user(
            "ocr worker blocked: local OCR command not configured",
        ));
    };

    let Some(job) = store
        .claim_next_job_by_kind(IngestJobKind::OcrDocument, now)
        .map_err(DaemonError::store)?
    else {
        return Ok(OcrWorkerSummary::default());
    };

    run_claimed_ocr_job(data_dir, store, &job, options, command, now)
}

fn run_claimed_ocr_job(
    data_dir: &Path,
    store: &MetaStore,
    job: &IngestJob,
    options: &RunOptions,
    command: PathBuf,
    now: UnixTimestamp,
) -> Result<OcrWorkerSummary> {
    let Some(mut document) = store
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
    let cache_key = OcrPageCacheKey::new(
        content_hash,
        1,
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
        if let Some(text) = entry.text() {
            if let Err(error) =
                index_ocr_text(data_dir, store, &document.id, text, entry.confidence(), now)
            {
                let _ = mark_ocr_job_failed_retryable(store, job, now);
                return Err(DaemonError::import(error));
            }
        } else {
            document.status = DocumentStatus::OcrDone;
            document.updated_at = now;
            store
                .upsert_document(&document)
                .map_err(DaemonError::store)?;
        }
        store
            .update_job_status(&job.id, IngestJobStatus::Completed, now)
            .map_err(DaemonError::store)?;
        return Ok(OcrWorkerSummary {
            processed: 1,
            cache_hits: 1,
            ..OcrWorkerSummary::default()
        });
    }

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
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(
            command,
            Vec::<String>::new(),
            options.ocr_engine_profile.as_str(),
        )
        .map_err(DaemonError::ocr)?,
    );
    let request = OcrPageRequest::new(
        RenderedPage::new(1, options.ocr_render_dpi, bytes).map_err(DaemonError::ocr)?,
        OcrOptions::new(options.ocr_lang.as_str(), options.ocr_profile.as_str())
            .map_err(DaemonError::ocr)?,
    )
    .map_err(DaemonError::ocr)?;

    match client.recognize_page(
        request,
        OcrWorkerBudget::new(options.ocr_page_timeout_ms).map_err(DaemonError::ocr)?,
        &CancellationToken::new(),
    ) {
        Ok(page) => {
            let entry = OcrPageCacheEntry::succeeded(
                cache_key,
                page.text(),
                page.confidence(),
                page.engine_profile(),
                page.duration_ms(),
                now,
            )
            .map_err(DaemonError::store)?;
            store
                .upsert_ocr_page_cache_entry(&entry)
                .map_err(DaemonError::store)?;
            if let Err(error) = index_ocr_text(
                data_dir,
                store,
                &document.id,
                page.text(),
                Some(page.confidence()),
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
                cache_writes: 1,
                ..OcrWorkerSummary::default()
            })
        }
        Err(error) => {
            let entry =
                OcrPageCacheEntry::failed_retryable(cache_key, format!("{:?}", error.kind()), now)
                    .map_err(DaemonError::store)?;
            store
                .upsert_ocr_page_cache_entry(&entry)
                .map_err(DaemonError::store)?;
            mark_ocr_job_failed_retryable(store, job, now)?;
            Ok(OcrWorkerSummary {
                failed: 1,
                ..OcrWorkerSummary::default()
            })
        }
    }
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
    enqueue_embedding_jobs_for_candidates(store, options.embedding_max_docs, now)?;
    let jobs = claim_embedding_jobs(store, options.embedding_max_docs, now)?;
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
    let inputs = candidates
        .iter()
        .map(|(_, candidate)| {
            EmbeddingInput::new(candidate.version_id.as_str(), candidate.text.as_str())
        })
        .collect::<Vec<_>>();
    let vectors = match embedder.embed_batch(
        &inputs,
        EmbeddingBudget::new(options.embedding_max_docs, options.embedding_max_text_bytes),
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
        .zip(candidates.iter())
        .map(|(vector, (_, candidate))| {
            VectorDocument::new(
                format!("{}:{}", vector.model_id(), vector.id()),
                candidate.document_id.as_str(),
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
        processed: inputs.len(),
        vector_writes,
        failed: 0,
    })
}

fn enqueue_embedding_jobs_for_candidates(
    store: &MetaStore,
    max_docs: usize,
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
                .enqueue_embedding_job_for_resume_version(&document.id, &version.id, now)
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
    now: UnixTimestamp,
) -> Result<Vec<IngestJob>> {
    let mut jobs = Vec::new();
    while jobs.len() < max_docs {
        let Some(job) = store
            .claim_next_embedding_job(now)
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

fn recover_stale_import_tasks(store: &MetaStore, now: UnixTimestamp) -> Result<usize> {
    store
        .recover_stale_running_import_tasks(
            now,
            timestamp_minus_seconds(now, STALE_IMPORT_TASK_SECONDS),
        )
        .map_err(DaemonError::store)
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

fn serve_ipc(data_dir: &Path, addr: SocketAddr, max_requests: Option<usize>) -> Result<()> {
    let listener = bind_ipc_listener(data_dir, addr)?;
    serve_ipc_listener(data_dir, &listener, max_requests)
}

fn bind_ipc_listener(data_dir: &Path, addr: SocketAddr) -> Result<TcpListener> {
    let _ = load_or_create_ipc_auth_token(data_dir)?;
    let listener = TcpListener::bind(addr)
        .map_err(|_| DaemonError::user("unable to bind daemon ipc listener"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|_| DaemonError::user("unable to inspect daemon ipc listener"))?;
    println!("ipc status endpoint: http://{local_addr}/status");
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::user("unable to write daemon status"))?;
    Ok(listener)
}

fn serve_ipc_listener(
    data_dir: &Path,
    listener: &TcpListener,
    max_requests: Option<usize>,
) -> Result<()> {
    let request_limit = max_requests.unwrap_or(usize::MAX);
    for _ in 0..request_limit {
        let (stream, _) = listener
            .accept()
            .map_err(|_| DaemonError::user("unable to accept daemon ipc request"))?;
        handle_ipc_stream(data_dir, stream)?;
    }

    Ok(())
}

fn serve_ipc_listener_with_worker_monitor(
    data_dir: &Path,
    listener: &TcpListener,
    max_requests: Option<usize>,
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
                handle_ipc_stream(data_dir, stream)?;
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

fn handle_ipc_stream(data_dir: &Path, mut stream: TcpStream) -> Result<()> {
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
        let body = status_json(data_dir)?;
        return write_http_response(&mut stream, 200, "application/json", &body);
    }

    if request.method == "POST"
        && request.path == "/imports"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_import_command_ipc(data_dir, &request, &mut stream);
    }

    if request.method == "POST"
        && request.path == "/search"
        && (request.version == "HTTP/1.1" || request.version == "HTTP/1.0")
    {
        return handle_search_command_ipc(data_dir, &request, &mut stream);
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
) -> Result<()> {
    if !ipc_command_authorized(data_dir, &request.headers)? {
        let body = serde_json::json!({
            "schema_version": "daemon.error.v1",
            "status": "unauthorized",
        })
        .to_string();
        return write_http_response(stream, 401, "application/json", &body);
    }

    match execute_search_command(data_dir, &request.body) {
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
) -> std::result::Result<String, IpcCommandError> {
    let payload = serde_json::from_slice::<serde_json::Value>(body)
        .map_err(|_| IpcCommandError::BadRequest("invalid json"))?;
    let args = parse_search_command(&payload)?;
    if args.mode != "fulltext" {
        return Err(IpcCommandError::BadRequest(
            "daemon search ipc supports fulltext mode only",
        ));
    }

    let Some(index) = FullTextIndex::open_active(&data_dir.join("search-index"))
        .map_err(DaemonError::fulltext)
        .map_err(IpcCommandError::Internal)?
    else {
        let body = serde_json::json!({
            "schema_version": "daemon.search.v1",
            "status": "ok",
            "mode": "fulltext",
            "search_index": "not_ready",
            "result_count": 0,
            "results": [],
        });
        return Ok(body.to_string());
    };
    let store = open_store(data_dir).map_err(IpcCommandError::Internal)?;
    let hits = daemon_fulltext_search(&index, &store, &args)?;
    let results = hits
        .iter()
        .map(|hit| {
            serde_json::json!({
                "rank": hit.rank,
                "doc_id": hit.doc_id,
                "version_id": hit.version_id,
                "file_name": hit.file_name,
                "snippet": hit.snippet,
            })
        })
        .collect::<Vec<_>>();
    let body = serde_json::json!({
        "schema_version": "daemon.search.v1",
        "status": "ok",
        "mode": "fulltext",
        "search_index": "available",
        "result_count": results.len(),
        "results": results,
    });
    Ok(body.to_string())
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
        .unwrap_or("fulltext")
        .to_string();
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
    let hits = index
        .search(SearchQuery::new(plan.query_text()).with_limit(plan.limit()))
        .map_err(DaemonError::fulltext)
        .map_err(IpcCommandError::Internal)?;
    daemon_visible_hits(store, hits, &args.filters, args.top_k)
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
        if !seen_candidate_keys.insert(candidate_key) {
            continue;
        }

        visible.push(DaemonSearchHit {
            rank: visible.len() + 1,
            doc_id: hit.doc_id,
            version_id: hit.version_id,
            file_name: redact_contact_values(&hit.file_name),
            snippet: redact_contact_values(&hit.snippet),
        });
        if visible.len() == top_k {
            break;
        }
    }

    Ok(visible)
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

fn daemon_persisted_profile(
    store: &MetaStore,
    doc_id: &str,
    version: &ResumeVersion,
) -> std::result::Result<ResumeProfile, IpcCommandError> {
    let fields = store
        .entity_mentions_for_version(&version.id)
        .map_err(DaemonError::store)
        .map_err(IpcCommandError::Internal)?;
    let degree = fields
        .iter()
        .filter(|field| field.entity_type == EntityType::Degree && field.confidence >= 0.75)
        .filter_map(|field| DegreeLevel::parse(field.normalized_value.as_deref()?))
        .max();
    let skills = fields
        .iter()
        .filter(|field| field.entity_type == EntityType::Skill && field.confidence >= 0.75)
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let years_experience = fields
        .iter()
        .filter(|field| {
            field.entity_type == EntityType::YearsExperience && field.confidence >= 0.75
        })
        .filter_map(|field| field.normalized_value.as_deref()?.parse::<f32>().ok())
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));

    let mut profile = ResumeProfile::new(doc_id).with_skills(skills);
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
    mode: String,
    top_k: usize,
    filters: SearchFilters,
}

struct DaemonSearchHit {
    rank: usize,
    doc_id: String,
    version_id: String,
    file_name: String,
    snippet: String,
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

fn status_json(data_dir: &Path) -> Result<String> {
    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(DaemonError::store)?;
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
        "embedding_queue_depth": summary.embedding_queue_depth,
        "entity_mentions": summary.entity_mentions,
        "import_tasks_queued": summary.import_tasks_queued,
        "import_tasks_recoverable": summary.import_tasks_recoverable,
        "import_scan_scopes": summary.import_scan_scopes,
        "import_scan_errors": summary.import_scan_errors,
        "active_profile": "balanced",
        "index_health": index_health_label(summary.index_health),
        "snapshot_present": summary.last_snapshot_id.is_some(),
    });
    Ok(body.to_string())
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
    work_ocr_once: bool,
    work_ocr: bool,
    work_embeddings_once: bool,
    work_embeddings: bool,
    ocr_command: Option<PathBuf>,
    ocr_engine_profile: String,
    ocr_lang: String,
    ocr_profile: String,
    ocr_render_dpi: u32,
    ocr_page_timeout_ms: u64,
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
            work_ocr_once: false,
            work_ocr: false,
            work_embeddings_once: false,
            work_embeddings: false,
            ocr_command: None,
            ocr_engine_profile: DEFAULT_OCR_ENGINE_PROFILE.to_string(),
            ocr_lang: DEFAULT_OCR_LANG.to_string(),
            ocr_profile: DEFAULT_OCR_PROFILE.to_string(),
            ocr_render_dpi: DEFAULT_OCR_RENDER_DPI,
            ocr_page_timeout_ms: DEFAULT_OCR_PAGE_TIMEOUT_MS,
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

#[derive(Default)]
struct ImportWorkerSummary {
    stale_recovered: usize,
    processed: usize,
    failed: usize,
    searchable_documents: usize,
    ocr_jobs_queued: usize,
}

impl ImportWorkerSummary {
    fn has_activity(&self) -> bool {
        self.stale_recovered > 0
            || self.processed > 0
            || self.failed > 0
            || self.searchable_documents > 0
            || self.ocr_jobs_queued > 0
    }

    fn extend(&mut self, other: Self) {
        self.stale_recovered += other.stale_recovered;
        self.processed += other.processed;
        self.failed += other.failed;
        self.searchable_documents += other.searchable_documents;
        self.ocr_jobs_queued += other.ocr_jobs_queued;
    }
}

fn print_import_worker_summary(import_summary: &ImportWorkerSummary) -> Result<()> {
    println!(
        "import worker recovered stale running: {}",
        import_summary.stale_recovered
    );
    println!("import worker processed: {}", import_summary.processed);
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

struct ImportTaskHeartbeat {
    stop: Arc<AtomicBool>,
}

impl ImportTaskHeartbeat {
    fn start(data_dir: &Path, task_id: ImportTaskId) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let metadata_path = data_dir.join("metadata.sqlite3");

        let _ = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(IMPORT_TASK_HEARTBEAT_SECONDS));
                if thread_stop.load(Ordering::Relaxed) {
                    return;
                }

                let Ok(now) = current_timestamp() else {
                    continue;
                };
                let Ok(store) = MetaStore::open(&metadata_path) else {
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
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).map_err(DaemonError::store)?;
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
        EntityType::Degree => "degree".to_string(),
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
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
