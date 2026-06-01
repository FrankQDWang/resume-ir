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

use import_pipeline::{
    import_root_with_options, ImportOptions, ImportScanBudgetKind as PipelineImportScanBudgetKind,
    ImportSummary, ScanProfile,
};
use index_fulltext::{redact_contact_values, FullTextIndex, SearchHit, SearchQuery};
use meta_store::{
    DocumentId, DocumentStatus, EntityType, ImportRootKind, ImportRootPreset, ImportScanBudgetKind,
    ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus,
    IndexStateStatus, MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
};
use rank_fusion::{DegreeLevel, ResumeProfile, SearchFilters};
use search_planner::plan_search;

const IMPORT_RETRY_BACKOFF_SECONDS: i64 = 60;
const IMPORT_TASK_HEARTBEAT_SECONDS: u64 = 30;
const STALE_IMPORT_TASK_SECONDS: i64 = 15 * 60;
const IPC_AUTH_TOKEN_FILE: &str = "ipc.auth";
const IPC_AUTH_TOKEN_BYTES: usize = 32;
const IPC_MAX_REQUEST_BYTES: usize = 64 * 1024;

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
        return Err(DaemonError::usage(
            "usage: resume-daemon run --foreground [--once] [--work-imports-once|--work-imports] [--worker-interval-ms <n>] [--max-worker-ticks <n>] [--ipc-listen <127.0.0.1:port>] [--max-requests <n>]",
        ));
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
    if (options.worker_interval_ms.is_some() || options.max_worker_ticks.is_some())
        && !options.work_imports
    {
        return Err(DaemonError::usage(
            "usage: worker loop options require --work-imports",
        ));
    }
    if options.work_imports && options.ipc_listen.is_some() && options.max_worker_ticks.is_some() {
        return Err(DaemonError::usage(
            "usage: --max-worker-ticks cannot be combined with --ipc-listen",
        ));
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

    if options.once {
        return Ok(());
    }
    if options.work_imports && options.ipc_listen.is_some() {
        run_import_worker_with_ipc(data_dir, &options)?;
        return Ok(());
    }
    if options.work_imports {
        run_import_worker_loop(data_dir, &store, &options, None)?;
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
    "usage: resume-daemon run --foreground [--once] [--work-imports-once|--work-imports] [--worker-interval-ms <n>] [--max-worker-ticks <n>] [--ipc-listen <127.0.0.1:port>] [--max-requests <n>]"
}

fn run_import_worker_with_ipc(data_dir: &Path, options: &RunOptions) -> Result<()> {
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
            run_import_worker_loop(&worker_data_dir, &store, &worker_options, Some(worker_stop))
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
        .map_err(|_| DaemonError::user("import worker thread panicked"))?;
    ipc_result
}

fn run_import_worker_loop(
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

#[derive(Clone, Default)]
struct RunOptions {
    foreground: bool,
    once: bool,
    ipc_listen: Option<SocketAddr>,
    max_requests: Option<usize>,
    work_imports_once: bool,
    work_imports: bool,
    worker_interval_ms: Option<u64>,
    max_worker_ticks: Option<usize>,
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

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
