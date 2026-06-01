use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use import_pipeline::{
    import_root_with_options, ImportOptions, ImportScanBudgetKind as PipelineImportScanBudgetKind,
    ImportSummary, ScanProfile,
};
use meta_store::{
    ImportScanBudgetKind, ImportScanProfile, ImportScanScope, ImportTaskId, ImportTaskStatus,
    IndexStateStatus, MetaStore, UnixTimestamp,
};

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
            "usage: resume-daemon run --foreground [--once] [--work-imports-once] [--ipc-listen <127.0.0.1:port>] [--max-requests <n>]",
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
            .map_err(|_| DaemonError::user("unable to write daemon status"))?;
    }

    if options.once {
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
    "usage: resume-daemon run --foreground [--once] [--work-imports-once] [--ipc-listen <127.0.0.1:port>] [--max-requests <n>]"
}

fn run_import_worker_once(data_dir: &Path, store: &MetaStore) -> Result<ImportWorkerSummary> {
    let mut worker_summary = ImportWorkerSummary::default();
    let mut attempted = Vec::<ImportTaskId>::new();

    while let Some(task) = store
        .claim_next_import_task_for_worker_excluding(current_timestamp()?, &attempted)
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
        let import_summary = match import_root_with_options(
            data_dir,
            store,
            &task,
            Path::new(&scope.canonical_root_path),
            now,
            import_options,
        ) {
            Ok(import_summary) => import_summary,
            Err(_) => {
                worker_summary.failed += 1;
                continue;
            }
        };

        upsert_scope_summary(store, scope, import_summary, now)?;
        worker_summary.processed += 1;
        worker_summary.searchable_documents += import_summary.searchable_documents;
        worker_summary.ocr_jobs_queued += import_summary.ocr_jobs_queued;
    }

    Ok(worker_summary)
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
    let listener = TcpListener::bind(addr)
        .map_err(|_| DaemonError::user("unable to bind daemon ipc listener"))?;
    let local_addr = listener
        .local_addr()
        .map_err(|_| DaemonError::user("unable to inspect daemon ipc listener"))?;
    println!("ipc status endpoint: http://{local_addr}/status");
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::user("unable to write daemon status"))?;

    let request_limit = max_requests.unwrap_or(usize::MAX);
    for _ in 0..request_limit {
        let (stream, _) = listener
            .accept()
            .map_err(|_| DaemonError::user("unable to accept daemon ipc request"))?;
        handle_ipc_stream(data_dir, stream)?;
    }

    Ok(())
}

fn handle_ipc_stream(data_dir: &Path, mut stream: TcpStream) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| DaemonError::user("unable to set daemon ipc timeout"))?;
    let mut request = Vec::new();
    let mut buffer = [0_u8; 512];

    loop {
        let read = stream
            .read(&mut buffer)
            .map_err(|_| DaemonError::user("unable to read daemon ipc request"))?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() > 8_192 {
            return write_http_response(&mut stream, 413, "text/plain", "request too large");
        }
    }

    let request = String::from_utf8_lossy(&request);
    let first_line = request.lines().next().unwrap_or_default();
    if first_line != "GET /status HTTP/1.1" && first_line != "GET /status HTTP/1.0" {
        return write_http_response(&mut stream, 404, "text/plain", "not found");
    }

    let body = status_json(data_dir)?;
    write_http_response(&mut stream, 200, "application/json", &body)
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
        404 => "Not Found",
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

#[derive(Default)]
struct RunOptions {
    foreground: bool,
    once: bool,
    ipc_listen: Option<SocketAddr>,
    max_requests: Option<usize>,
    work_imports_once: bool,
}

#[derive(Default)]
struct ImportWorkerSummary {
    processed: usize,
    failed: usize,
    searchable_documents: usize,
    ocr_jobs_queued: usize,
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

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for DaemonError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
