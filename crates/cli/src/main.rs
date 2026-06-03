use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use embedder::{
    Embedder, EmbeddingBudget, EmbeddingInput, LocalEmbeddingCommandEmbedder,
    LocalEmbeddingCommandSpec,
};
use import_pipeline::{
    import_root_with_options, index_ocr_text, rebuild_full_text_index, ImportOptions,
    ImportScanBudgetKind as PipelineImportScanBudgetKind, ImportSummary, ScanProfile,
};
use index_fulltext::{
    inspect_snapshot_root, redact_contact_values, FullTextIndex, SearchHit, SearchQuery,
    SnapshotReadTarget, SnapshotRootState,
};
use index_vector::{
    inspect_persistent_vector_snapshot, PersistentVectorIndex, PersistentVectorSnapshotInspection,
    PersistentVectorSnapshotState, QueryVector, VectorDocument, VectorHit, VectorIndex,
};
use meta_store::{
    Document, DocumentId, DocumentStatus, EntityMention, EntityType, FileExtension,
    ImportRootKind as StoreImportRootKind, ImportRootPreset as StoreImportRootPreset,
    ImportScanBudgetKind as StoreImportScanBudgetKind, ImportScanProfile as StoreImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus, IndexStateStatus, IngestJobKind,
    IngestJobStatus, MetaStore, OcrPageCacheEntry, OcrPageCacheKey, ResumeVersion, ResumeVersionId,
    ResumeVisibility, UnixTimestamp, WorkerTaskKind,
};
use ocr_client::{
    CancellationToken, LocalOcrCommandClient, LocalOcrCommandSpec, OcrClient, OcrOptions,
    OcrPageRequest, OcrWorkerBudget, RenderedPage,
};
use privacy::inspect_contact_hash_key;
use rank_fusion::{
    fuse_hybrid_rrf, DegreeLevel, HybridRecall, RankedHit, ResumeProfile, SearchFilters,
};
use search_planner::plan_search;
use sectionizer::Sectionizer;

const LOCAL_DISCOVERY_ROOTS_ENV: &str = "RESUME_IR_LOCAL_DISCOVERY_ROOTS";
const LOCAL_DISCOVERY_DEFAULT_MAX_FILES: usize = 10_000;
const IPC_ENDPOINT_FILE: &str = "ipc.endpoints.json";
const IPC_AUTH_TOKEN_FILE: &str = "ipc.auth";
const IPC_ENDPOINT_SCHEMA_VERSION: &str = "resume-ir.daemon-ipc.v1";

fn main() {
    if let Err(error) = run() {
        eprintln!("resume-cli: {error}");
        std::process::exit(error.exit_code());
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();

    if args == ["--identity"] {
        println!("resume-cli");
        return Ok(());
    }

    let data_dir = take_data_dir(&mut args)?;
    let Some(command) = args.first().map(String::as_str) else {
        return Err(CliError::usage(
            "expected command: status, import, search, detail, delete, cancel, pause, resume, ocr-worker, embed-worker, doctor, or export-diagnostics",
        ));
    };

    match command {
        "status" => status_command(&data_dir, &args[1..]),
        "import" => import_command(&data_dir, &args[1..]),
        "search" => search_command(&data_dir, &args[1..]),
        "detail" => detail_command(&data_dir, &args[1..]),
        "delete" => delete_command(&data_dir, &args[1..]),
        "cancel" => cancel_command(&data_dir, &args[1..]),
        "pause" => task_control_command(&data_dir, &args[1..], true),
        "resume" => task_control_command(&data_dir, &args[1..], false),
        "ocr-worker" => ocr_worker_command(&data_dir, &args[1..]),
        "embed-worker" => embed_worker_command(&data_dir, &args[1..]),
        "doctor" => {
            if args.len() != 1 {
                return Err(CliError::usage("usage: resume-cli doctor"));
            }
            doctor_command(&data_dir)
        }
        "export-diagnostics" => export_diagnostics_command(&data_dir, &args[1..]),
        _ => Err(CliError::usage(
            "expected command: status, import, search, detail, delete, cancel, pause, resume, ocr-worker, embed-worker, doctor, or export-diagnostics",
        )),
    }
}

fn take_data_dir(args: &mut Vec<String>) -> Result<PathBuf> {
    if args.first().map(String::as_str) != Some("--data-dir") {
        return Ok(PathBuf::from("local-data"));
    }

    if args.len() < 2 {
        return Err(CliError::usage(
            "usage: resume-cli --data-dir <path> <command>",
        ));
    }

    let path = PathBuf::from(args.remove(1));
    args.remove(0);
    Ok(path)
}

fn status_command(data_dir: &Path, args: &[String]) -> Result<()> {
    if let Some(endpoint) = parse_status_ipc_arg(data_dir, args)? {
        return status_ipc_command(&endpoint);
    }

    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(CliError::store)?;
    let ocr_task = store
        .worker_task_control(WorkerTaskKind::Ocr)
        .map_err(CliError::store)?;
    let index_diagnostic = inspect_search_index(data_dir);
    let vector_diagnostic = inspect_vector_index(data_dir);

    println!("resume-ir status");
    println!("indexed documents: {}", summary.indexed_documents);
    println!("searchable documents: {}", summary.searchable_documents);
    println!("partial documents: {}", summary.partial_documents);
    println!("failed retryable: {}", summary.failed_retryable);
    println!("failed permanent: {}", summary.failed_permanent);
    println!("recovery queue: {}", summary.recovery_queue_depth);
    println!("ocr queue: {}", summary.ocr_queue_depth);
    println!("ocr jobs queued: {}", summary.ocr_jobs_queued);
    println!("ocr task: {}", worker_task_status_label(ocr_task.paused));
    println!("embedding queue: {}", summary.embedding_queue_depth);
    println!("entity mentions: {}", summary.entity_mentions);
    println!("import tasks queued: {}", summary.import_tasks_queued);
    println!(
        "import tasks recoverable: {}",
        summary.import_tasks_recoverable
    );
    println!("import tasks cancelled: {}", summary.import_tasks_cancelled);
    println!("import scan scopes: {}", summary.import_scan_scopes);
    println!("import scan errors: {}", summary.import_scan_errors);
    println!("active profile: balanced");
    println!("index health: {}", index_health_label(summary.index_health));
    println!(
        "last snapshot: {}",
        summary.last_snapshot_id.as_deref().unwrap_or("none")
    );
    println!("search index: {}", index_diagnostic.index_label());
    println!("vector index: {}", vector_diagnostic.index_label());
    println!("vector index vectors: {}", vector_diagnostic.vector_count());
    println!(
        "vector index tombstones: {}",
        vector_diagnostic.deleted_count()
    );

    Ok(())
}

fn parse_status_ipc_arg(data_dir: &Path, args: &[String]) -> Result<Option<IpcStatusEndpoint>> {
    if args.is_empty() {
        return Ok(None);
    }
    if args.len() != 2 || args.first().map(String::as_str) != Some("--ipc") {
        return Err(CliError::usage(status_usage()));
    }
    if args[1] == "auto" {
        return discover_status_ipc_endpoint(data_dir).map(Some);
    }

    parse_status_ipc_endpoint(&args[1]).map(Some)
}

fn status_usage() -> &'static str {
    "usage: resume-cli status [--ipc <auto|http://127.0.0.1:port/status>]"
}

fn parse_status_ipc_endpoint(value: &str) -> Result<IpcStatusEndpoint> {
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| CliError::usage(status_usage()))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| CliError::usage(status_usage()))?;
    if path != "status" {
        return Err(CliError::usage(status_usage()));
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| CliError::usage(status_usage()))?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage("status ipc endpoint must be loopback"));
    }

    Ok(IpcStatusEndpoint { addr })
}

fn status_ipc_command(endpoint: &IpcStatusEndpoint) -> Result<()> {
    let body = request_status_ipc_body(endpoint)?;
    render_ipc_status(&body);
    Ok(())
}

fn request_status_ipc_body(endpoint: &IpcStatusEndpoint) -> Result<serde_json::Value> {
    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon status ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon status ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon status ipc"))?;
    let request = format!(
        "GET /status HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        endpoint.addr
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon status ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon status ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon status ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200 ") && !status_line.starts_with("HTTP/1.0 200 ") {
        return Err(CliError::user("daemon status ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon status ipc returned invalid json"))?;
    Ok(body)
}

fn verify_auto_ipc_status(endpoint: &IpcStatusEndpoint) -> Result<()> {
    let body = request_status_ipc_body(endpoint)
        .map_err(|_| CliError::user("daemon ipc auto-discovery is stale"))?;
    if json_str(&body, "schema_version") != Some("daemon.status.v1")
        || json_str(&body, "status") != Some("ok")
    {
        return Err(CliError::user("daemon ipc auto-discovery is stale"));
    }
    Ok(())
}

fn render_ipc_status(body: &serde_json::Value) {
    println!("resume-ir status");
    println!("indexed documents: {}", json_u64(body, "indexed_documents"));
    println!(
        "searchable documents: {}",
        json_u64(body, "searchable_documents")
    );
    println!("partial documents: {}", json_u64(body, "partial_documents"));
    println!("failed retryable: {}", json_u64(body, "failed_retryable"));
    println!("failed permanent: {}", json_u64(body, "failed_permanent"));
    println!("recovery queue: {}", json_u64(body, "recovery_queue_depth"));
    println!("ocr queue: {}", json_u64(body, "ocr_queue_depth"));
    println!("ocr jobs queued: {}", json_u64(body, "ocr_jobs_queued"));
    println!(
        "embedding queue: {}",
        json_u64(body, "embedding_queue_depth")
    );
    println!("entity mentions: {}", json_u64(body, "entity_mentions"));
    println!(
        "import tasks queued: {}",
        json_u64(body, "import_tasks_queued")
    );
    println!(
        "import tasks recoverable: {}",
        json_u64(body, "import_tasks_recoverable")
    );
    println!(
        "import tasks cancelled: {}",
        json_u64(body, "import_tasks_cancelled")
    );
    println!(
        "import scan scopes: {}",
        json_u64(body, "import_scan_scopes")
    );
    println!(
        "import scan errors: {}",
        json_u64(body, "import_scan_errors")
    );
    println!(
        "active profile: {}",
        json_str(body, "active_profile").unwrap_or("unknown")
    );
    println!(
        "index health: {}",
        json_str(body, "index_health").unwrap_or("unknown")
    );
    let snapshot_label = if json_bool(body, "snapshot_present") {
        "present"
    } else {
        "none"
    };
    println!("last snapshot: {snapshot_label}");
    println!("search index: daemon ipc (full-text state reported by daemon)");
}

fn json_u64(body: &serde_json::Value, key: &str) -> u64 {
    body.get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

fn json_str<'a>(body: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    body.get(key).and_then(serde_json::Value::as_str)
}

fn json_bool(body: &serde_json::Value, key: &str) -> bool {
    body.get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

struct IpcStatusEndpoint {
    addr: SocketAddr,
}

struct IpcImportEndpoint {
    addr: SocketAddr,
}

#[derive(Clone)]
struct IpcSearchEndpoint {
    addr: SocketAddr,
}

#[derive(Clone)]
struct IpcDetailEndpoint {
    addr: SocketAddr,
}

fn auto_ipc_token_file(data_dir: &Path) -> PathBuf {
    data_dir.join(IPC_AUTH_TOKEN_FILE)
}

fn discover_status_ipc_endpoint(data_dir: &Path) -> Result<IpcStatusEndpoint> {
    parse_status_ipc_endpoint(&discover_ipc_url(data_dir, "status")?)
}

fn discover_import_ipc_endpoint(data_dir: &Path) -> Result<IpcImportEndpoint> {
    parse_import_ipc_endpoint(&discover_ipc_url(data_dir, "imports")?)
}

fn discover_search_ipc_endpoint(data_dir: &Path) -> Result<IpcSearchEndpoint> {
    parse_search_ipc_endpoint(&discover_ipc_url(data_dir, "search")?)
}

fn discover_detail_ipc_endpoint(data_dir: &Path) -> Result<IpcDetailEndpoint> {
    parse_detail_ipc_endpoint(&discover_ipc_url(data_dir, "details")?)
}

fn ensure_auto_ipc_same_daemon(status_addr: SocketAddr, command_addr: SocketAddr) -> Result<()> {
    if status_addr != command_addr {
        return Err(CliError::user("daemon ipc auto-discovery is invalid"));
    }
    Ok(())
}

fn discover_ipc_url(data_dir: &Path, key: &str) -> Result<String> {
    let manifest = fs::read_to_string(data_dir.join(IPC_ENDPOINT_FILE))
        .map_err(|_| CliError::user("daemon ipc auto-discovery is unavailable"))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest)
        .map_err(|_| CliError::user("daemon ipc auto-discovery is invalid"))?;
    if json_str(&manifest, "schema_version") != Some(IPC_ENDPOINT_SCHEMA_VERSION) {
        return Err(CliError::user("daemon ipc auto-discovery is invalid"));
    }
    json_str(&manifest, key)
        .map(str::to_string)
        .ok_or_else(|| CliError::user("daemon ipc auto-discovery is invalid"))
}

fn import_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let import_args = parse_import_args(args)?;
    if import_args.ipc_auto {
        let status_endpoint = discover_status_ipc_endpoint(data_dir)?;
        let endpoint = discover_import_ipc_endpoint(data_dir)?;
        ensure_auto_ipc_same_daemon(status_endpoint.addr, endpoint.addr)?;
        verify_auto_ipc_status(&status_endpoint)?;
        let token_file = auto_ipc_token_file(data_dir);
        return import_ipc_command_with_token_file(&endpoint, &token_file, &import_args);
    }
    if let Some(endpoint) = &import_args.ipc_endpoint {
        return import_ipc_command(endpoint, &import_args);
    }

    let requested_roots = expand_import_root_selection(&import_args.root_selection)?;
    let roots = canonical_import_roots(&requested_roots)?;

    let store = open_store(data_dir)?;
    let now = current_timestamp()?;
    let mut tasks = Vec::new();
    let mut new_tasks = Vec::new();

    for root in &roots {
        let canonical_root_path = path_string(&root.canonical);
        let requested_root_path = path_string(&root.requested);
        let task = match pending_import_task(&store, &canonical_root_path, &requested_root_path)? {
            Some(task) if task.status == ImportTaskStatus::Running => {
                return Err(CliError::user("import task is already running"));
            }
            Some(task) => task,
            None => {
                let task = ImportTask {
                    id: new_import_task_id()?,
                    root_path: canonical_root_path,
                    status: ImportTaskStatus::Queued,
                    queued_at: now,
                    started_at: None,
                    finished_at: None,
                    updated_at: now,
                };
                new_tasks.push(task.clone());
                task
            }
        };
        tasks.push(task);
    }

    for task in &new_tasks {
        store.insert_import_task(task).map_err(CliError::store)?;
    }

    let mut summary = ImportSummary::default();
    for (task, root) in tasks.iter().zip(roots.iter()) {
        upsert_import_scan_scope(
            &store,
            task,
            root,
            &import_args,
            &initial_import_summary(&import_args),
            now,
        )?;
    }

    if import_args.enqueue {
        let task_ids = tasks
            .iter()
            .map(|task| task.id.to_string())
            .collect::<Vec<_>>()
            .join(",");

        println!("import task submitted");
        if tasks.len() == 1 {
            println!("task id: {}", tasks[0].id);
        } else {
            println!("task ids: {task_ids}");
        }
        println!("status: queued");
        println!("scan profile: {}", import_args.profile.label());
        println!("roots queued: {}", roots.len());
        println!(
            "scan file limit: {}",
            import_args
                .max_files
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string())
        );
        return Ok(());
    }

    for (task, root) in tasks.iter().zip(roots.iter()) {
        let root_summary = import_root_with_options(
            data_dir,
            &store,
            task,
            &root.canonical,
            now,
            ImportOptions {
                scan_profile: import_args.profile,
                max_files: import_args.max_files,
            },
        )
        .map_err(CliError::import)?;
        upsert_import_scan_scope(&store, task, root, &import_args, &root_summary, now)?;
        merge_import_summary(&mut summary, root_summary);
    }

    let task_ids = tasks
        .iter()
        .map(|task| task.id.to_string())
        .collect::<Vec<_>>()
        .join(",");

    println!("import task submitted");
    if tasks.len() == 1 {
        println!("task id: {}", tasks[0].id);
    } else {
        println!("task ids: {task_ids}");
    }
    println!("status: completed");
    println!("scan profile: {}", import_args.profile.label());
    println!("roots scanned: {}", roots.len());
    println!("files discovered: {}", summary.files_discovered);
    println!("searchable documents: {}", summary.searchable_documents);
    println!("ocr required documents: {}", summary.ocr_required_documents);
    println!("ocr jobs queued: {}", summary.ocr_jobs_queued);
    println!("failed documents: {}", summary.failed_documents);
    println!("deleted documents: {}", summary.deleted_documents);
    println!("scan errors: {}", summary.scan_errors);
    println!(
        "scan budget exhausted: {}",
        if summary.scan_budget.is_some_and(|budget| budget.exhausted) {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "scan file limit: {}",
        import_args
            .max_files
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    );

    Ok(())
}

fn import_ipc_command(endpoint: &IpcImportEndpoint, import_args: &ImportArgs) -> Result<()> {
    let token_file = import_args
        .ipc_token_file
        .as_ref()
        .ok_or_else(import_usage)?;
    import_ipc_command_with_token_file(endpoint, token_file, import_args)
}

fn import_ipc_command_with_token_file(
    endpoint: &IpcImportEndpoint,
    token_file: &Path,
    import_args: &ImportArgs,
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .map_err(|_| CliError::user("unable to read daemon import ipc token"))?;
    let token = validate_daemon_ipc_token(&token, "daemon import ipc token is invalid")?;

    let roots = expand_import_root_selection(&import_args.root_selection)?;
    let root_values = roots
        .iter()
        .map(|root| serde_json::Value::String(path_string(root)))
        .collect::<Vec<_>>();
    let root_preset = import_args.root_selection.preset_label();
    let body = serde_json::json!({
        "roots": root_values,
        "root_preset": root_preset,
        "profile": import_args.profile.label(),
        "max_files": import_args.max_files,
    })
    .to_string();

    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon import ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon import ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon import ipc"))?;
    let request = format!(
        "POST /imports HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.addr,
        token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon import ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon import ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon import ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 202 ") && !status_line.starts_with("HTTP/1.0 202 ") {
        return Err(CliError::user("daemon import ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon import ipc returned invalid json"))?;
    render_import_ipc_result(&body);
    Ok(())
}

fn validate_daemon_ipc_token<'a>(token: &'a str, invalid_message: &'static str) -> Result<&'a str> {
    let token = token.trim();
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::user(invalid_message));
    }
    Ok(token)
}

fn render_import_ipc_result(body: &serde_json::Value) {
    let task_ids = body
        .get("task_ids")
        .and_then(serde_json::Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let roots_queued = json_u64(body, "accepted_roots");
    let scan_file_limit = body
        .get("scan_file_limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string());

    println!("import task submitted");
    match task_ids.as_slice() {
        [task_id] => println!("task id: {task_id}"),
        [] => println!("task ids: none"),
        ids => println!("task ids: {}", ids.join(",")),
    }
    println!("status: queued");
    println!(
        "scan profile: {}",
        json_str(body, "scan_profile").unwrap_or("unknown")
    );
    println!("roots queued: {roots_queued}");
    println!("scan file limit: {scan_file_limit}");
}

fn merge_import_summary(total: &mut ImportSummary, next: ImportSummary) {
    total.files_discovered += next.files_discovered;
    total.scan_errors += next.scan_errors;
    total.ignored_entries += next.ignored_entries;
    total.searchable_documents += next.searchable_documents;
    total.ocr_required_documents += next.ocr_required_documents;
    total.ocr_jobs_queued += next.ocr_jobs_queued;
    total.failed_documents += next.failed_documents;
    total.deleted_documents += next.deleted_documents;
    if next.scan_budget.is_some()
        && (total.scan_budget.is_none() || next.scan_budget.is_some_and(|budget| budget.exhausted))
    {
        total.scan_budget = next.scan_budget;
    }
}

fn initial_import_summary(import_args: &ImportArgs) -> ImportSummary {
    ImportSummary {
        scan_budget: import_args
            .max_files
            .map(|limit| import_pipeline::ImportScanBudget {
                kind: PipelineImportScanBudgetKind::Files,
                limit,
                observed: 0,
                exhausted: false,
            }),
        ..ImportSummary::default()
    }
}

fn upsert_import_scan_scope(
    store: &MetaStore,
    task: &ImportTask,
    root: &CanonicalImportRoot,
    import_args: &ImportArgs,
    summary: &ImportSummary,
    updated_at: UnixTimestamp,
) -> Result<()> {
    let (root_kind, root_preset) = import_scan_scope_root(&import_args.root_selection);
    store
        .upsert_import_scan_scope(&ImportScanScope {
            import_task_id: task.id.clone(),
            root_kind,
            root_preset,
            scan_profile: import_scan_profile(import_args.profile),
            requested_root_path: path_string(&root.requested),
            canonical_root_path: path_string(&root.canonical),
            files_discovered: usize_to_u64(summary.files_discovered)?,
            ignored_entries: usize_to_u64(summary.ignored_entries)?,
            scan_errors: usize_to_u64(summary.scan_errors)?,
            searchable_documents: usize_to_u64(summary.searchable_documents)?,
            ocr_required_documents: usize_to_u64(summary.ocr_required_documents)?,
            ocr_jobs_queued: usize_to_u64(summary.ocr_jobs_queued)?,
            failed_documents: usize_to_u64(summary.failed_documents)?,
            deleted_documents: usize_to_u64(summary.deleted_documents)?,
            scan_budget_kind: summary
                .scan_budget
                .map(|budget| import_scan_budget_kind(budget.kind)),
            scan_budget_limit: summary
                .scan_budget
                .map(|budget| usize_to_u64(budget.limit))
                .transpose()?,
            scan_budget_observed: summary
                .scan_budget
                .map(|budget| usize_to_u64(budget.observed))
                .transpose()?,
            scan_budget_exhausted: summary.scan_budget.is_some_and(|budget| budget.exhausted),
            updated_at,
        })
        .map_err(CliError::store)
}

fn import_scan_scope_root(
    selection: &ImportRootSelection,
) -> (StoreImportRootKind, Option<StoreImportRootPreset>) {
    match selection {
        ImportRootSelection::Explicit(_) => (StoreImportRootKind::Explicit, None),
        ImportRootSelection::Preset(RootPreset::LocalDiscovery) => (
            StoreImportRootKind::Preset,
            Some(StoreImportRootPreset::LocalDiscovery),
        ),
    }
}

fn import_scan_profile(profile: ScanProfile) -> StoreImportScanProfile {
    match profile {
        ScanProfile::Explicit => StoreImportScanProfile::Explicit,
        ScanProfile::Discovery => StoreImportScanProfile::Discovery,
    }
}

fn import_scan_budget_kind(kind: PipelineImportScanBudgetKind) -> StoreImportScanBudgetKind {
    match kind {
        PipelineImportScanBudgetKind::Files => StoreImportScanBudgetKind::Files,
    }
}

fn usize_to_u64(value: usize) -> Result<u64> {
    u64::try_from(value).map_err(|_| CliError::user("import summary count is too large"))
}

#[derive(Clone)]
struct CanonicalImportRoot {
    requested: PathBuf,
    canonical: PathBuf,
}

fn path_string(path: &Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

struct ImportArgs {
    root_selection: ImportRootSelection,
    profile: ScanProfile,
    max_files: Option<usize>,
    enqueue: bool,
    ipc_auto: bool,
    ipc_endpoint: Option<IpcImportEndpoint>,
    ipc_token_file: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ImportRootSelection {
    Explicit(Vec<PathBuf>),
    Preset(RootPreset),
}

impl ImportRootSelection {
    fn preset_label(&self) -> Option<&'static str> {
        match self {
            Self::Explicit(_) => None,
            Self::Preset(RootPreset::LocalDiscovery) => Some("local-discovery"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootPreset {
    LocalDiscovery,
}

impl RootPreset {
    fn default_profile(self) -> ScanProfile {
        match self {
            Self::LocalDiscovery => ScanProfile::Discovery,
        }
    }
}

fn parse_import_args(args: &[String]) -> Result<ImportArgs> {
    let mut roots = Vec::<PathBuf>::new();
    let mut root_preset = None;
    let mut profile = None;
    let mut profile_seen = false;
    let mut max_files = None;
    let mut enqueue = false;
    let mut ipc_auto = false;
    let mut ipc_endpoint = None;
    let mut ipc_token_file = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--enqueue" => {
                if enqueue {
                    return Err(import_usage());
                }
                enqueue = true;
            }
            "--root" => {
                if root_preset.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                let root = PathBuf::from(value);
                if roots.iter().any(|existing| existing == &root) {
                    return Err(import_usage());
                }
                roots.push(root);
            }
            "--root-preset" => {
                if root_preset.is_some() || !roots.is_empty() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                root_preset = Some(parse_root_preset(value)?);
            }
            "--profile" => {
                if profile_seen {
                    return Err(import_usage());
                }
                profile_seen = true;
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                profile = Some(parse_scan_profile(value)?);
            }
            "--max-files" => {
                if max_files.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                max_files = Some(parse_positive_usize(value)?);
            }
            "--ipc" => {
                if ipc_auto || ipc_endpoint.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                if value == "auto" {
                    ipc_auto = true;
                } else {
                    ipc_endpoint = Some(parse_import_ipc_endpoint(value)?);
                }
            }
            "--ipc-token-file" => {
                if ipc_token_file.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                ipc_token_file = Some(PathBuf::from(value));
            }
            _ => return Err(import_usage()),
        }
        index += 1;
    }
    if ipc_auto && ipc_token_file.is_some() {
        return Err(import_usage());
    }
    if !ipc_auto && ipc_endpoint.is_some() != ipc_token_file.is_some() {
        return Err(import_usage());
    }

    let (root_selection, default_profile) = if !roots.is_empty() {
        (ImportRootSelection::Explicit(roots), ScanProfile::Explicit)
    } else if let Some(root_preset) = root_preset {
        (
            ImportRootSelection::Preset(root_preset),
            root_preset.default_profile(),
        )
    } else {
        return Err(import_usage());
    };

    let max_files = match (&root_selection, max_files) {
        (ImportRootSelection::Preset(RootPreset::LocalDiscovery), None) => {
            Some(LOCAL_DISCOVERY_DEFAULT_MAX_FILES)
        }
        (_, max_files) => max_files,
    };

    Ok(ImportArgs {
        root_selection,
        profile: profile.unwrap_or(default_profile),
        max_files,
        enqueue,
        ipc_auto,
        ipc_endpoint,
        ipc_token_file,
    })
}

fn parse_import_ipc_endpoint(value: &str) -> Result<IpcImportEndpoint> {
    let rest = value.strip_prefix("http://").ok_or_else(import_usage)?;
    let (authority, path) = rest.split_once('/').ok_or_else(import_usage)?;
    if path != "imports" && path != "status" {
        return Err(import_usage());
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| import_usage())?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage("import ipc endpoint must be loopback"));
    }

    Ok(IpcImportEndpoint { addr })
}

fn parse_root_preset(value: &str) -> Result<RootPreset> {
    match value {
        "local-discovery" => Ok(RootPreset::LocalDiscovery),
        _ => Err(import_usage()),
    }
}

fn parse_scan_profile(value: &str) -> Result<ScanProfile> {
    match value {
        "explicit" => Ok(ScanProfile::Explicit),
        "discovery" => Ok(ScanProfile::Discovery),
        _ => Err(import_usage()),
    }
}

fn parse_positive_usize(value: &str) -> Result<usize> {
    let parsed = value.parse::<usize>().map_err(|_| import_usage())?;
    if parsed == 0 {
        return Err(import_usage());
    }
    Ok(parsed)
}

fn import_usage() -> CliError {
    CliError::usage(
        "usage: resume-cli import [--enqueue] [--ipc auto|<http://127.0.0.1:port/imports|/status> --ipc-token-file <path>] (--root <path> [--root <path> ...] | --root-preset local-discovery) [--profile explicit|discovery] [--max-files <count>]",
    )
}

fn expand_import_root_selection(selection: &ImportRootSelection) -> Result<Vec<PathBuf>> {
    match selection {
        ImportRootSelection::Explicit(roots) => Ok(roots.clone()),
        ImportRootSelection::Preset(RootPreset::LocalDiscovery) => local_discovery_roots(),
    }
}

fn local_discovery_roots() -> Result<Vec<PathBuf>> {
    let roots = std::env::var_os(LOCAL_DISCOVERY_ROOTS_ENV)
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(platform_local_discovery_roots);

    if roots.is_empty() {
        return Err(CliError::user(
            "local discovery import roots are unavailable",
        ));
    }

    Ok(roots)
}

#[cfg(not(windows))]
fn platform_local_discovery_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

#[cfg(windows)]
fn platform_local_discovery_roots() -> Vec<PathBuf> {
    (b'A'..=b'Z')
        .map(|drive| PathBuf::from(format!("{}:\\", drive as char)))
        .filter(|root| {
            fs::metadata(root)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        })
        .collect()
}

fn canonical_import_roots(requested_roots: &[PathBuf]) -> Result<Vec<CanonicalImportRoot>> {
    let mut roots = requested_roots
        .iter()
        .map(|requested_root| {
            let metadata = fs::metadata(requested_root)
                .map_err(|_| CliError::user("import root must exist and be a directory"))?;
            if !metadata.is_dir() {
                return Err(CliError::user("import root must exist and be a directory"));
            }
            let canonical = fs::canonicalize(requested_root)
                .map_err(|_| CliError::user("import root must exist and be a directory"))?;
            Ok(CanonicalImportRoot {
                requested: requested_root.clone(),
                canonical,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    roots.sort_by(|left, right| left.canonical.cmp(&right.canonical));
    for window in roots.windows(2) {
        let [left, right] = window else {
            continue;
        };
        if left.canonical == right.canonical || right.canonical.starts_with(&left.canonical) {
            return Err(CliError::user(
                "import roots must be distinct and non-overlapping",
            ));
        }
    }

    Ok(roots)
}

fn pending_import_task(
    store: &MetaStore,
    canonical_root_path: &str,
    requested_root_path: &str,
) -> Result<Option<ImportTask>> {
    if let Some(task) = store
        .pending_import_task_by_root(canonical_root_path)
        .map_err(CliError::store)?
    {
        return Ok(Some(task));
    }

    if requested_root_path == canonical_root_path {
        return Ok(None);
    }

    store
        .pending_import_task_by_root(requested_root_path)
        .map_err(CliError::store)
}

fn search_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let search_args = parse_search_args(args)?;
    if search_args.ipc_auto {
        let status_endpoint = discover_status_ipc_endpoint(data_dir)?;
        let endpoint = discover_search_ipc_endpoint(data_dir)?;
        ensure_auto_ipc_same_daemon(status_endpoint.addr, endpoint.addr)?;
        verify_auto_ipc_status(&status_endpoint)?;
        let token_file = auto_ipc_token_file(data_dir);
        return search_ipc_command_with_token_file(&endpoint, &token_file, &search_args);
    }
    if let Some(endpoint) = &search_args.ipc_endpoint {
        return search_ipc_command(endpoint, &search_args);
    }

    let candidate_limit = search_args
        .top_k
        .saturating_mul(5)
        .clamp(search_args.top_k, 100);

    let hits = match search_args.mode {
        SearchMode::FullText => {
            let Some(index) = FullTextIndex::open_active(&data_dir.join("search-index"))
                .map_err(CliError::fulltext)?
            else {
                println!("search index not available yet");
                println!("results: 0");
                return Ok(());
            };
            let store = open_store(data_dir)?;
            let fulltext_hits = run_fulltext_search(&index, &store, &search_args, candidate_limit)?;
            fulltext_hits.into_iter().take(search_args.top_k).collect()
        }
        SearchMode::Semantic => {
            let store = open_store(data_dir)?;
            run_semantic_search(data_dir, &store, &search_args, candidate_limit)?
        }
        SearchMode::Hybrid => {
            let Some(index) = FullTextIndex::open_active(&data_dir.join("search-index"))
                .map_err(CliError::fulltext)?
            else {
                return Err(CliError::user(
                    "hybrid search unavailable: full-text index is not ready",
                ));
            };
            let store = open_store(data_dir)?;
            let fulltext_hits = run_fulltext_search(&index, &store, &search_args, candidate_limit)?;
            let vector_hits = run_semantic_search(data_dir, &store, &search_args, candidate_limit)?;
            fuse_hybrid_output_hits(fulltext_hits, vector_hits, search_args.top_k)
        }
    };

    print_search_hits(hits);

    Ok(())
}

fn search_ipc_command(endpoint: &IpcSearchEndpoint, search_args: &SearchArgs) -> Result<()> {
    let token_file = search_args
        .ipc_token_file
        .as_ref()
        .ok_or_else(|| CliError::usage(search_usage()))?;
    search_ipc_command_with_token_file(endpoint, token_file, search_args)
}

fn search_ipc_command_with_token_file(
    endpoint: &IpcSearchEndpoint,
    token_file: &Path,
    search_args: &SearchArgs,
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .map_err(|_| CliError::user("unable to read daemon search ipc token"))?;
    let token = validate_daemon_ipc_token(&token, "daemon search ipc token is invalid")?;
    let body = search_ipc_request_body(search_args);

    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon search ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon search ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon search ipc"))?;
    let request = format!(
        "POST /search HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.addr,
        token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon search ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon search ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon search ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200 ") && !status_line.starts_with("HTTP/1.0 200 ") {
        return Err(CliError::user("daemon search ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon search ipc returned invalid json"))?;
    render_search_ipc_result(&body)?;
    Ok(())
}

fn search_ipc_request_body(search_args: &SearchArgs) -> String {
    serde_json::json!({
        "query": search_args.query.as_str(),
        "mode": search_args.mode.label(),
        "top_k": search_args.top_k,
        "filters": search_filters_json(&search_args.filters),
    })
    .to_string()
}

fn search_filters_json(filters: &SearchFilters) -> serde_json::Value {
    serde_json::json!({
        "degree_min": filters.degree_min().map(DegreeLevel::canonical),
        "skills_any": filters.skills_any(),
        "years_experience_min": filters.years_experience_min(),
    })
}

fn render_search_ipc_result(body: &serde_json::Value) -> Result<()> {
    if json_str(body, "schema_version") != Some("daemon.search.v1")
        || json_str(body, "status") != Some("ok")
    {
        return Err(CliError::user(
            "daemon search ipc returned invalid protocol",
        ));
    }
    if json_str(body, "search_index") == Some("not_ready") {
        println!("search index not available yet");
    }
    let results = body
        .get("results")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
    println!("results: {}", results.len());
    for result in results {
        let rank = result
            .get("rank")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
        let doc_id = json_str(result, "doc_id")
            .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
        let version_id = json_str(result, "version_id")
            .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
        let file_name = json_str(result, "file_name")
            .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
        let snippet = json_str(result, "snippet")
            .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
        println!("rank: {rank}");
        println!("doc_id: {doc_id}");
        println!("version_id: {version_id}");
        println!("file_name: {}", redact_contact_values(file_name));
        println!("snippet: {}", redact_contact_values(snippet));
    }
    Ok(())
}

fn run_fulltext_search(
    index: &FullTextIndex,
    store: &MetaStore,
    search_args: &SearchArgs,
    candidate_limit: usize,
) -> Result<Vec<SearchOutputHit>> {
    let plan = plan_search(&search_args.query, candidate_limit)
        .map_err(|_| CliError::user("search query is empty"))?;
    let hits = index
        .search(SearchQuery::new(plan.query_text()).with_limit(plan.limit()))
        .map_err(CliError::fulltext)?;

    if search_args.filters.is_empty() {
        visible_hits(store, hits, candidate_limit)
    } else {
        filter_hits(store, hits, &search_args.filters, candidate_limit)
    }
}

fn print_search_hits(hits: Vec<SearchOutputHit>) {
    println!("results: {}", hits.len());
    for hit in hits {
        println!("rank: {}", hit.rank);
        println!("doc_id: {}", hit.doc_id);
        println!("version_id: {}", hit.version_id);
        println!("file_name: {}", hit.file_name);
        println!("snippet: {}", hit.snippet);
    }
}

fn detail_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let detail_args = parse_detail_args(args)?;
    if detail_args.ipc_auto {
        let status_endpoint = discover_status_ipc_endpoint(data_dir)?;
        let endpoint = discover_detail_ipc_endpoint(data_dir)?;
        ensure_auto_ipc_same_daemon(status_endpoint.addr, endpoint.addr)?;
        verify_auto_ipc_status(&status_endpoint)?;
        let token_file = auto_ipc_token_file(data_dir);
        return detail_ipc_command_with_token_file(&endpoint, &token_file, &detail_args);
    }
    if let Some(endpoint) = &detail_args.ipc_endpoint {
        return detail_ipc_command(endpoint, &detail_args);
    }

    let document_id = DocumentId::from_str(&detail_args.doc_id)
        .map_err(|_| CliError::user("detail doc id is invalid"))?;
    let store = open_store(data_dir)?;
    let detail = build_resume_detail(&store, &document_id)?
        .ok_or_else(|| CliError::user("detail document was not found"))?;
    print_resume_detail(&detail);
    Ok(())
}

fn detail_ipc_command(endpoint: &IpcDetailEndpoint, detail_args: &DetailArgs) -> Result<()> {
    let token_file = detail_args
        .ipc_token_file
        .as_ref()
        .ok_or_else(|| CliError::usage(detail_usage()))?;
    detail_ipc_command_with_token_file(endpoint, token_file, detail_args)
}

fn detail_ipc_command_with_token_file(
    endpoint: &IpcDetailEndpoint,
    token_file: &Path,
    detail_args: &DetailArgs,
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .map_err(|_| CliError::user("unable to read daemon detail ipc token"))?;
    let token = validate_daemon_ipc_token(&token, "daemon detail ipc token is invalid")?;
    let body = serde_json::json!({
        "doc_id": detail_args.doc_id.as_str(),
    })
    .to_string();

    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon detail ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon detail ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon detail ipc"))?;
    let request = format!(
        "POST /details HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.addr,
        token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon detail ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon detail ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon detail ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200 ") && !status_line.starts_with("HTTP/1.0 200 ") {
        return Err(CliError::user("daemon detail ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon detail ipc returned invalid json"))?;
    render_detail_ipc_result(&body, detail_args.doc_id.as_str())?;
    Ok(())
}

fn render_detail_ipc_result(body: &serde_json::Value, expected_doc_id: &str) -> Result<()> {
    if json_str(body, "schema_version") != Some("daemon.detail.v1")
        || json_str(body, "status") != Some("ok")
    {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }
    let document = body
        .get("document")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let doc_id = json_str(document, "doc_id")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if doc_id != expected_doc_id || DocumentId::from_str(doc_id).is_err() {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }
    let version_id = json_str(document, "version_id")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if ResumeVersionId::from_str(version_id).is_err() {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }
    let file_name = json_str(document, "file_name")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let extension = json_str(document, "extension")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if !is_valid_detail_extension_label(extension) {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }
    let document_status = json_str(document, "document_status")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if !is_valid_detail_document_status_label(document_status) {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }
    let visibility = json_str(document, "visibility")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if !matches!(visibility, "searchable" | "partial") {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }
    let byte_size = document
        .get("byte_size")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let snippet = json_str(document, "snippet")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let fields = document
        .get("fields")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;

    let fields = fields
        .iter()
        .map(parse_detail_ipc_field)
        .collect::<Result<Vec<_>>>()?;
    let detail = ResumeDetail {
        doc_id: doc_id.to_string(),
        version_id: version_id.to_string(),
        file_name: redact_short_text(file_name, 160),
        extension: extension.to_string(),
        document_status: document_status.to_string(),
        visibility: visibility.to_string(),
        byte_size,
        fields,
        snippet: redact_short_text(snippet, 240),
    };
    print_resume_detail(&detail);
    Ok(())
}

fn parse_detail_ipc_field(value: &serde_json::Value) -> Result<ResumeDetailField> {
    let field_type = json_str(value, "type")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if !is_valid_detail_field_type_label(field_type) {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }
    let field_value = json_str(value, "value")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let evidence = json_str(value, "evidence")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let extractor = json_str(value, "extractor")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let confidence = value
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }

    Ok(ResumeDetailField {
        field_type: field_type.to_string(),
        value: redact_short_text(field_value, 120),
        confidence,
        evidence: redact_short_text(evidence, 120),
        extractor: redact_short_text(extractor, 80),
    })
}

fn is_valid_detail_extension_label(value: &str) -> bool {
    matches!(value, "docx" | "pdf" | "doc" | "txt" | "image" | "other")
}

fn is_valid_detail_document_status_label(value: &str) -> bool {
    matches!(
        value,
        "discovered"
            | "fingerprinted"
            | "parse_queued"
            | "parse_running"
            | "text_extracted"
            | "ocr_required"
            | "ocr_running"
            | "ocr_done"
            | "text_cleaned"
            | "fields_extracted"
            | "embedding_done"
            | "indexed_partial"
            | "searchable"
            | "failed_retryable"
            | "failed_permanent"
    )
}

fn is_valid_detail_field_type_label(value: &str) -> bool {
    matches!(
        value,
        "name"
            | "email"
            | "phone"
            | "school"
            | "degree"
            | "company"
            | "title"
            | "education"
            | "skills"
            | "skill"
            | "certificate"
            | "date"
            | "date_range"
            | "years_experience"
            | "location"
            | "other"
    )
}

fn build_resume_detail(
    store: &MetaStore,
    document_id: &DocumentId,
) -> Result<Option<ResumeDetail>> {
    let Some(document) = store.document_by_id(document_id).map_err(CliError::store)? else {
        return Ok(None);
    };
    if document.is_deleted || document.status == DocumentStatus::Deleted {
        return Ok(None);
    }
    let Some(version) = select_detail_version(store, document_id)? else {
        return Ok(None);
    };
    let fields = store
        .entity_mentions_for_version(&version.id)
        .map_err(CliError::store)?
        .iter()
        .map(resume_detail_field_from_mention)
        .collect::<Vec<_>>();
    let snippet = version
        .clean_text
        .as_deref()
        .or(version.raw_text.as_deref())
        .map(|text| redact_short_text(text, 240))
        .unwrap_or_else(|| "none".to_string());

    Ok(Some(ResumeDetail {
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

fn select_detail_version(
    store: &MetaStore,
    document_id: &DocumentId,
) -> Result<Option<ResumeVersion>> {
    store
        .latest_visible_resume_version_for_document(document_id)
        .map_err(CliError::store)
}

fn resume_detail_field_from_mention(mention: &EntityMention) -> ResumeDetailField {
    let value = mention
        .normalized_value
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&mention.raw_value);
    ResumeDetailField {
        field_type: entity_type_label(&mention.entity_type),
        value: redact_short_text(value, 120),
        confidence: f64::from(mention.confidence.clamp(0.0, 1.0)),
        evidence: redact_short_text(&mention.raw_value, 120),
        extractor: redact_short_text(&mention.extractor, 80),
    }
}

fn print_resume_detail(detail: &ResumeDetail) {
    println!("resume detail");
    println!("doc_id: {}", detail.doc_id);
    println!("version_id: {}", detail.version_id);
    println!("file_name: {}", detail.file_name);
    println!("extension: {}", detail.extension);
    println!("document status: {}", detail.document_status);
    println!("visibility: {}", detail.visibility);
    println!("byte_size: {}", detail.byte_size);
    println!("fields: {}", detail.fields.len());
    for field in &detail.fields {
        println!(
            "field: {} | value: {} | confidence: {:.2} | evidence: {} | extractor: {}",
            field.field_type, field.value, field.confidence, field.evidence, field.extractor
        );
    }
    println!("snippet: {}", detail.snippet);
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

fn delete_command(data_dir: &Path, args: &[String]) -> Result<()> {
    if args.len() != 2 || args.first().map(String::as_str) != Some("--doc-id") {
        return Err(CliError::usage(
            "usage: resume-cli delete --doc-id <doc_id>",
        ));
    }

    let document_id =
        DocumentId::from_str(&args[1]).map_err(|_| CliError::user("delete doc id is invalid"))?;
    let store = open_store(data_dir)?;
    let now = current_timestamp()?;
    let Some(deleted_document) = store
        .mark_document_deleted(&document_id, now)
        .map_err(CliError::store)?
    else {
        return Err(CliError::user("delete document was not found"));
    };
    let rebuild = rebuild_full_text_index(data_dir, &store, now).map_err(CliError::import)?;

    println!("delete completed");
    println!("doc_id: {}", deleted_document.id);
    println!("status: deleted");
    println!("index rebuilt: true");
    println!("indexed documents: {}", rebuild.indexed_documents);

    Ok(())
}

fn task_control_command(data_dir: &Path, args: &[String], paused: bool) -> Result<()> {
    let task = parse_worker_task_control_args(args)?;
    let store = open_store(data_dir)?;
    let now = current_timestamp()?;
    store
        .set_worker_task_paused(task, paused, now)
        .map_err(CliError::store)?;

    println!("task: {}", worker_task_label(task));
    println!("status: {}", worker_task_status_label(paused));

    Ok(())
}

fn cancel_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let task_id = parse_cancel_import_args(args)?;
    let store = open_store(data_dir)?;
    let now = current_timestamp()?;
    store
        .cancel_import_task(&task_id, now)
        .map_err(CliError::store)?;

    println!("import task cancelled");
    println!("task id: {task_id}");
    println!("status: cancelled");

    Ok(())
}

fn parse_cancel_import_args(args: &[String]) -> Result<ImportTaskId> {
    if args.len() != 3
        || args.first().map(String::as_str) != Some("import")
        || args.get(1).map(String::as_str) != Some("--task-id")
    {
        return Err(cancel_usage());
    }

    ImportTaskId::from_str(&args[2]).map_err(|_| cancel_usage())
}

fn cancel_usage() -> CliError {
    CliError::usage("usage: resume-cli cancel import --task-id <id>")
}

fn parse_worker_task_control_args(args: &[String]) -> Result<WorkerTaskKind> {
    if args.len() != 2 || args.first().map(String::as_str) != Some("--task") {
        return Err(task_control_usage());
    }

    parse_worker_task_kind(&args[1])
}

fn parse_worker_task_kind(value: &str) -> Result<WorkerTaskKind> {
    match value {
        "ocr" => Ok(WorkerTaskKind::Ocr),
        _ => Err(task_control_usage()),
    }
}

fn worker_task_label(task: WorkerTaskKind) -> &'static str {
    match task {
        WorkerTaskKind::Ocr => "ocr",
    }
}

fn worker_task_status_label(paused: bool) -> &'static str {
    if paused {
        "paused"
    } else {
        "running"
    }
}

fn task_control_usage() -> CliError {
    CliError::usage("usage: resume-cli pause --task ocr OR resume --task ocr")
}

fn ocr_worker_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let worker_args = parse_ocr_worker_args(args)?;
    let store = open_store(data_dir)?;
    let now = current_timestamp()?;
    if store
        .worker_task_control(WorkerTaskKind::Ocr)
        .map_err(CliError::store)?
        .paused
    {
        println!("ocr worker: paused");
        println!("documents processed: 0");
        println!("cache writes: 0");
        println!("cache hits: 0");
        return Ok(());
    }

    let Some(command) = worker_args.command.clone() else {
        return Err(CliError::user(
            "ocr worker blocked: local OCR command not configured",
        ));
    };

    let Some(job) = store
        .claim_next_job_by_kind(IngestJobKind::OcrDocument, now)
        .map_err(CliError::store)?
    else {
        println!("ocr worker: idle");
        println!("documents processed: 0");
        println!("cache writes: 0");
        return Ok(());
    };

    let result = run_claimed_ocr_job(data_dir, &store, &job, &worker_args, command, now);
    match result {
        Ok(summary) => {
            println!("ocr worker: completed");
            println!("documents processed: {}", summary.documents_processed);
            println!("cache writes: {}", summary.cache_writes);
            println!("cache hits: {}", summary.cache_hits);
            Ok(())
        }
        Err(error) => {
            let _ = store.update_job_status(&job.id, IngestJobStatus::FailedRetryable, now);
            Err(error)
        }
    }
}

fn run_claimed_ocr_job(
    data_dir: &Path,
    store: &MetaStore,
    job: &meta_store::IngestJob,
    worker_args: &OcrWorkerArgs,
    command: PathBuf,
    now: UnixTimestamp,
) -> Result<OcrWorkerSummary> {
    let Some(mut document) = store
        .document_by_id(&job.document_id)
        .map_err(CliError::store)?
    else {
        store
            .update_job_status(&job.id, IngestJobStatus::FailedPermanent, now)
            .map_err(CliError::store)?;
        return Err(CliError::user("ocr worker job document was not found"));
    };
    let content_hash = document
        .content_hash
        .clone()
        .ok_or_else(|| CliError::user("ocr worker document is missing content hash"))?;
    let cache_key = OcrPageCacheKey::new(
        content_hash,
        1,
        worker_args.render_dpi,
        worker_args.lang.as_str(),
        worker_args.profile.as_str(),
    )
    .map_err(CliError::store)?;

    if let Some(entry) = store
        .ocr_page_cache_entry(&cache_key)
        .map_err(CliError::store)?
        .filter(|entry| entry.status() == meta_store::OcrPageCacheStatus::Succeeded)
    {
        if let Some(text) = entry.text() {
            let _ = index_ocr_text(data_dir, store, &document.id, text, entry.confidence(), now)
                .map_err(CliError::import)?;
        } else {
            document.status = DocumentStatus::OcrDone;
            document.updated_at = now;
            store.upsert_document(&document).map_err(CliError::store)?;
        }
        store
            .update_job_status(&job.id, IngestJobStatus::Completed, now)
            .map_err(CliError::store)?;
        return Ok(OcrWorkerSummary {
            documents_processed: 1,
            cache_writes: 0,
            cache_hits: 1,
        });
    }

    let bytes = fs::read(&document.normalized_path)
        .map_err(|_| CliError::user("ocr worker could not read document bytes"))?;
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(
            command,
            Vec::<String>::new(),
            worker_args.engine_profile.as_str(),
        )
        .map_err(CliError::ocr)?,
    );
    let request = OcrPageRequest::new(
        RenderedPage::new(1, worker_args.render_dpi, bytes).map_err(CliError::ocr)?,
        OcrOptions::new(worker_args.lang.as_str(), worker_args.profile.as_str())
            .map_err(CliError::ocr)?,
    )
    .map_err(CliError::ocr)?;

    match client.recognize_page(
        request,
        OcrWorkerBudget::new(worker_args.page_timeout_ms).map_err(CliError::ocr)?,
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
            .map_err(CliError::store)?;
            store
                .upsert_ocr_page_cache_entry(&entry)
                .map_err(CliError::store)?;
            let _ = index_ocr_text(
                data_dir,
                store,
                &document.id,
                page.text(),
                Some(page.confidence()),
                now,
            )
            .map_err(CliError::import)?;
            store
                .update_job_status(&job.id, IngestJobStatus::Completed, now)
                .map_err(CliError::store)?;
            Ok(OcrWorkerSummary {
                documents_processed: 1,
                cache_writes: 1,
                cache_hits: 0,
            })
        }
        Err(error) => {
            let entry =
                OcrPageCacheEntry::failed_retryable(cache_key, format!("{:?}", error.kind()), now)
                    .map_err(CliError::store)?;
            store
                .upsert_ocr_page_cache_entry(&entry)
                .map_err(CliError::store)?;
            store
                .update_job_status(&job.id, IngestJobStatus::FailedRetryable, now)
                .map_err(CliError::store)?;
            Err(CliError::user(
                "ocr worker blocked: local OCR command failed or unavailable",
            ))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OcrWorkerSummary {
    documents_processed: usize,
    cache_writes: usize,
    cache_hits: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OcrWorkerArgs {
    command: Option<PathBuf>,
    engine_profile: String,
    lang: String,
    profile: String,
    render_dpi: u32,
    page_timeout_ms: u64,
}

fn parse_ocr_worker_args(args: &[String]) -> Result<OcrWorkerArgs> {
    let mut seen_once = false;
    let mut command = None;
    let mut engine_profile = "local-command".to_string();
    let mut lang = "eng".to_string();
    let mut profile = "balanced".to_string();
    let mut render_dpi = 300_u32;
    let mut page_timeout_ms = 30_000_u64;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--once" => {
                if seen_once {
                    return Err(ocr_worker_usage());
                }
                seen_once = true;
                index += 1;
            }
            "--command" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if command.is_some() {
                    return Err(ocr_worker_usage());
                }
                command = Some(PathBuf::from(value));
                index += 1;
            }
            "--engine-profile" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if value.trim().is_empty() {
                    return Err(ocr_worker_usage());
                }
                engine_profile = value.clone();
                index += 1;
            }
            "--lang" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if value.trim().is_empty() {
                    return Err(ocr_worker_usage());
                }
                lang = value.clone();
                index += 1;
            }
            "--profile" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if value.trim().is_empty() {
                    return Err(ocr_worker_usage());
                }
                profile = value.clone();
                index += 1;
            }
            "--render-dpi" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                render_dpi = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(ocr_worker_usage)?;
                index += 1;
            }
            "--page-timeout-ms" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                page_timeout_ms = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(ocr_worker_usage)?;
                index += 1;
            }
            _ => return Err(ocr_worker_usage()),
        }
    }

    if !seen_once {
        return Err(ocr_worker_usage());
    }

    Ok(OcrWorkerArgs {
        command,
        engine_profile,
        lang,
        profile,
        render_dpi,
        page_timeout_ms,
    })
}

fn ocr_worker_usage() -> CliError {
    CliError::usage(
        "usage: resume-cli ocr-worker --once [--command <path>] [--engine-profile <name>] [--lang <lang>] [--profile <profile>] [--render-dpi <dpi>] [--page-timeout-ms <ms>]",
    )
}

fn embed_worker_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let worker_args = parse_embed_worker_args(args)?;
    let Some(command) = worker_args.command.clone() else {
        return Err(CliError::user(
            "embedding worker blocked: local embedding command not configured",
        ));
    };
    let model_id = worker_args
        .model_id
        .as_deref()
        .ok_or_else(embed_worker_usage)?;
    let dimension = worker_args.dimension.ok_or_else(embed_worker_usage)?;
    let store = open_store(data_dir)?;
    let candidates = embedding_candidates(&store, worker_args.max_docs)?;
    let documents_considered = candidates.len();

    if candidates.is_empty() {
        let vector_diagnostic = inspect_vector_index(data_dir);
        println!("embedding worker: completed");
        println!("model id: {model_id}");
        println!("dimension: {dimension}");
        println!("documents considered: 0");
        println!("documents embedded: 0");
        println!("vector index: {}", vector_diagnostic.index_label());
        return Ok(());
    }

    let embedder = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), model_id, dimension)
            .map_err(CliError::embedding)?
            .with_timeout_ms(worker_args.timeout_ms)
            .map_err(CliError::embedding)?,
    );
    let vector_inputs = embedding_inputs_for_candidates(&candidates);
    let inputs = vector_inputs
        .iter()
        .map(|input| EmbeddingInput::new(input.input_id.as_str(), input.text.as_str()))
        .collect::<Vec<_>>();
    let vectors = embedder
        .embed_batch(
            &inputs,
            EmbeddingBudget::new(inputs.len(), worker_args.max_text_bytes),
        )
        .map_err(CliError::embedding)?;
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
            .map_err(CliError::vector)
        })
        .collect::<Result<Vec<_>>>()?;
    let index = PersistentVectorIndex::open(data_dir.join("vector-index"), dimension)
        .map_err(CliError::vector)?;
    index.upsert(vector_documents).map_err(CliError::vector)?;

    let vector_diagnostic = inspect_vector_index(data_dir);
    println!("embedding worker: completed");
    println!("model id: {model_id}");
    println!("dimension: {dimension}");
    println!("documents considered: {documents_considered}");
    println!("documents embedded: {}", candidates.len());
    println!("vector inputs: {}", inputs.len());
    println!("vector index: {}", vector_diagnostic.index_label());

    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
struct EmbedWorkerCandidate {
    document_id: DocumentId,
    version_id: ResumeVersionId,
    text: String,
}

impl fmt::Debug for EmbedWorkerCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbedWorkerCandidate")
            .field("document_id", &self.document_id)
            .field("version_id", &self.version_id)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

fn embedding_candidates(store: &MetaStore, max_docs: usize) -> Result<Vec<EmbedWorkerCandidate>> {
    let mut candidates = Vec::new();
    for document in store.visible_documents().map_err(CliError::store)? {
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
            .map_err(CliError::store)?
        {
            if version.visibility != ResumeVisibility::Searchable {
                continue;
            }
            let Some(text) = version
                .clean_text
                .as_deref()
                .or(version.raw_text.as_deref())
                .map(str::trim)
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            candidates.push(EmbedWorkerCandidate {
                document_id: document.id.clone(),
                version_id: version.id,
                text: text.to_string(),
            });
            if candidates.len() == max_docs {
                return Ok(candidates);
            }
        }
    }

    Ok(candidates)
}

#[derive(Clone, PartialEq, Eq)]
struct EmbedWorkerInput {
    document_id: DocumentId,
    input_id: String,
    text: String,
}

impl fmt::Debug for EmbedWorkerInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbedWorkerInput")
            .field("document_id", &self.document_id)
            .field("input_id", &self.input_id)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

fn embedding_inputs_for_candidates(candidates: &[EmbedWorkerCandidate]) -> Vec<EmbedWorkerInput> {
    let sectionizer = Sectionizer::default();
    candidates
        .iter()
        .flat_map(|candidate| embedding_inputs_for_candidate(candidate, &sectionizer))
        .collect()
}

fn embedding_inputs_for_candidate(
    candidate: &EmbedWorkerCandidate,
    sectionizer: &Sectionizer,
) -> Vec<EmbedWorkerInput> {
    let mut inputs = vec![EmbedWorkerInput {
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

            Some(EmbedWorkerInput {
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

#[derive(Clone, PartialEq, Eq)]
struct EmbedWorkerArgs {
    command: Option<PathBuf>,
    model_id: Option<String>,
    dimension: Option<usize>,
    max_docs: usize,
    max_text_bytes: usize,
    timeout_ms: u64,
}

impl fmt::Debug for EmbedWorkerArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbedWorkerArgs")
            .field("command_configured", &self.command.is_some())
            .field("command", &self.command.as_ref().map(|_| "<redacted>"))
            .field("model_id", &self.model_id)
            .field("dimension", &self.dimension)
            .field("max_docs", &self.max_docs)
            .field("max_text_bytes", &self.max_text_bytes)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

fn parse_embed_worker_args(args: &[String]) -> Result<EmbedWorkerArgs> {
    let mut seen_once = false;
    let mut command = None;
    let mut model_id = None;
    let mut dimension = None;
    let mut max_docs = 64_usize;
    let mut max_text_bytes = 1_000_000_usize;
    let mut timeout_ms = 30_000_u64;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--once" => {
                if seen_once {
                    return Err(embed_worker_usage());
                }
                seen_once = true;
                index += 1;
            }
            "--command" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(embed_worker_usage());
                };
                if command.is_some() {
                    return Err(embed_worker_usage());
                }
                command = Some(PathBuf::from(value));
                index += 1;
            }
            "--model-id" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(embed_worker_usage());
                };
                if model_id.is_some() || !valid_cli_identifier(value) {
                    return Err(embed_worker_usage());
                }
                model_id = Some(value.clone());
                index += 1;
            }
            "--dimension" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(embed_worker_usage());
                };
                if dimension.is_some() {
                    return Err(embed_worker_usage());
                }
                dimension = Some(
                    value
                        .parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(embed_worker_usage)?,
                );
                index += 1;
            }
            "--max-docs" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(embed_worker_usage());
                };
                max_docs = value
                    .parse::<usize>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(embed_worker_usage)?;
                index += 1;
            }
            "--max-text-bytes" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(embed_worker_usage());
                };
                max_text_bytes = value
                    .parse::<usize>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(embed_worker_usage)?;
                index += 1;
            }
            "--timeout-ms" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(embed_worker_usage());
                };
                timeout_ms = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(embed_worker_usage)?;
                index += 1;
            }
            _ => return Err(embed_worker_usage()),
        }
    }

    if !seen_once {
        return Err(embed_worker_usage());
    }

    Ok(EmbedWorkerArgs {
        command,
        model_id,
        dimension,
        max_docs,
        max_text_bytes,
        timeout_ms,
    })
}

fn valid_cli_identifier(value: &str) -> bool {
    !value.trim().is_empty()
        && !value.contains('\n')
        && !value.contains('\r')
        && !value.contains('\t')
}

fn embed_worker_usage() -> CliError {
    CliError::usage(
        "usage: resume-cli embed-worker --once [--command <path>] [--model-id <id>] [--dimension <n>] [--max-docs <n>] [--max-text-bytes <bytes>] [--timeout-ms <ms>]",
    )
}

fn doctor_command(data_dir: &Path) -> Result<()> {
    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(CliError::store)?;
    let index_diagnostic = inspect_search_index(data_dir);
    let vector_diagnostic = inspect_vector_index(data_dir);
    let contact_key = inspect_contact_hash_key(data_dir).map_err(CliError::privacy)?;

    println!("resume-ir doctor");
    println!("metadata: ok");
    println!("indexed documents: {}", summary.indexed_documents);
    println!("searchable documents: {}", summary.searchable_documents);
    println!("ocr queue: {}", summary.ocr_queue_depth);
    println!("ocr jobs queued: {}", summary.ocr_jobs_queued);
    println!("entity mentions: {}", summary.entity_mentions);
    println!("import scan scopes: {}", summary.import_scan_scopes);
    println!("import scan errors: {}", summary.import_scan_errors);
    println!("recovery queue: {}", summary.recovery_queue_depth);
    println!("index health: {}", index_health_label(summary.index_health));
    println!(
        "last snapshot: {}",
        if summary.last_snapshot_id.is_some() {
            "present"
        } else {
            "none"
        }
    );
    println!("search index: {}", index_diagnostic.index_label());
    println!("vector index: {}", vector_diagnostic.index_label());
    println!("vector index vectors: {}", vector_diagnostic.vector_count());
    println!(
        "vector index tombstones: {}",
        vector_diagnostic.deleted_count()
    );
    println!(
        "search index read target: {}",
        index_diagnostic.read_target_label()
    );
    println!("query smoke: {}", index_diagnostic.query_smoke_label());
    println!(
        "snapshot fallback: {}",
        index_diagnostic.snapshot_fallback_label()
    );
    println!("staging orphans: {}", index_diagnostic.staging_orphans());
    println!("contact hash key: {}", contact_key.state().label());
    println!("fault simulations: available");
    println!("fault simulation hooks: daemon_restart,index_snapshot_corrupt,disk_space_low");
    println!("diagnostics redaction: available");

    Ok(())
}

fn export_diagnostics_command(data_dir: &Path, args: &[String]) -> Result<()> {
    if args != ["--redact"] {
        return Err(CliError::usage(
            "usage: resume-cli export-diagnostics --redact",
        ));
    }

    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(CliError::store)?;
    let index_diagnostic = inspect_search_index(data_dir);
    let vector_diagnostic = inspect_vector_index(data_dir);
    let contact_key = inspect_contact_hash_key(data_dir).map_err(CliError::privacy)?;

    println!("{{");
    println!("  \"schema_version\": \"diagnostics.v1\",");
    println!("  \"redacted\": true,");
    println!("  \"raw_paths\": \"<redacted>\",");
    println!("  \"raw_queries\": \"<redacted>\",");
    println!("  \"raw_resume_text\": \"<redacted>\",");
    println!("  \"metadata\": {{");
    println!("    \"indexed_documents\": {},", summary.indexed_documents);
    println!(
        "    \"searchable_documents\": {},",
        summary.searchable_documents
    );
    println!("    \"ocr_queue_depth\": {},", summary.ocr_queue_depth);
    println!("    \"ocr_jobs_queued\": {},", summary.ocr_jobs_queued);
    println!("    \"entity_mentions\": {},", summary.entity_mentions);
    println!(
        "    \"import_scan_scopes\": {},",
        summary.import_scan_scopes
    );
    println!(
        "    \"import_scan_errors\": {},",
        summary.import_scan_errors
    );
    println!(
        "    \"recovery_queue_depth\": {}",
        summary.recovery_queue_depth
    );
    println!("  }},");
    println!(
        "  \"search_index_state\": \"{}\",",
        index_diagnostic.state_label()
    );
    println!(
        "  \"vector_index_state\": \"{}\",",
        vector_diagnostic.state_label()
    );
    println!(
        "  \"vector_index_vectors\": {},",
        vector_diagnostic.vector_count()
    );
    println!(
        "  \"vector_index_tombstones\": {},",
        vector_diagnostic.deleted_count()
    );
    println!(
        "  \"search_index_read_target\": \"{}\",",
        index_diagnostic.read_target_label()
    );
    println!(
        "  \"index_health\": \"{}\",",
        index_health_label(summary.index_health)
    );
    println!(
        "  \"last_snapshot\": \"{}\",",
        if summary.last_snapshot_id.is_some() {
            "present"
        } else {
            "none"
        }
    );
    println!(
        "  \"staging_orphans\": {},",
        index_diagnostic.staging_orphans()
    );
    println!(
        "  \"snapshot_fallback\": \"{}\",",
        index_diagnostic.snapshot_fallback_label()
    );
    println!(
        "  \"query_smoke\": \"{}\",",
        index_diagnostic.query_smoke_json_label()
    );
    println!(
        "  \"contact_hash_key\": \"{}\",",
        contact_key.state().label()
    );
    println!("  \"fault_simulations\": [");
    println!("    \"daemon_restart\",");
    println!("    \"index_snapshot_corrupt\",");
    println!("    \"disk_space_low\"");
    println!("  ],");
    println!("  \"scope\": \"redacted skeleton; no raw resume text, paths, or queries included\"");
    println!("}}");

    Ok(())
}

fn parse_search_args(args: &[String]) -> Result<SearchArgs> {
    let Some(query) = args.first() else {
        return Err(CliError::usage(search_usage()));
    };

    let mut top_k = 10_usize;
    let mut filters = SearchFilters::default();
    let mut mode = SearchMode::FullText;
    let mut embedding_command = None;
    let mut model_id = None;
    let mut dimension = None;
    let mut vector_top_k = None;
    let mut embedding_timeout_ms = 30_000_u64;
    let mut ipc_auto = false;
    let mut ipc_endpoint = None;
    let mut ipc_token_file = None;
    let mut index = 1_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--ipc" => {
                if ipc_auto || ipc_endpoint.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                if value == "auto" {
                    ipc_auto = true;
                } else {
                    ipc_endpoint = Some(parse_search_ipc_endpoint(value)?);
                }
                index += 2;
            }
            "--ipc-token-file" => {
                if ipc_token_file.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                ipc_token_file = Some(PathBuf::from(value));
                index += 2;
            }
            "--mode" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                mode = SearchMode::parse(value).ok_or_else(|| CliError::usage(search_usage()))?;
                index += 2;
            }
            "--embedding-command" => {
                if embedding_command.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                embedding_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--model-id" => {
                if model_id.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(search_usage()));
                }
                model_id = Some(value.clone());
                index += 2;
            }
            "--dimension" => {
                if dimension.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                dimension = Some(parse_positive_usize(value)?);
                index += 2;
            }
            "--vector-top-k" => {
                if vector_top_k.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                vector_top_k = Some(parse_positive_usize(value)?.min(1000));
                index += 2;
            }
            "--embedding-timeout-ms" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                embedding_timeout_ms = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| CliError::usage(search_usage()))?;
                index += 2;
            }
            "--degree" | "--degree-min" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let degree = DegreeLevel::parse(value)
                    .ok_or_else(|| CliError::user("search degree filter is invalid"))?;
                filters = filters.with_degree_min(degree);
                index += 2;
            }
            "--skills-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                filters = filters.with_skills_any(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|skill| !skill.is_empty()),
                );
                index += 2;
            }
            "--years-experience-min" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let years = value
                    .parse::<f32>()
                    .ok()
                    .filter(|years| years.is_finite() && *years >= 0.0)
                    .ok_or_else(|| CliError::user("search years filter is invalid"))?;
                filters = filters.with_years_experience_min(years);
                index += 2;
            }
            "--top-k" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                top_k = value
                    .parse::<usize>()
                    .ok()
                    .filter(|value| *value > 0)
                    .map(|value| value.min(100))
                    .ok_or_else(|| CliError::user("search top-k is invalid"))?;
                index += 2;
            }
            _ => return Err(CliError::usage(search_usage())),
        }
    }
    if ipc_auto && ipc_token_file.is_some() {
        return Err(CliError::usage(search_usage()));
    }
    if !ipc_auto && ipc_endpoint.is_some() != ipc_token_file.is_some() {
        return Err(CliError::usage(search_usage()));
    }

    Ok(SearchArgs {
        query: query.clone(),
        top_k,
        filters,
        mode,
        embedding_command,
        model_id,
        dimension,
        vector_top_k,
        embedding_timeout_ms,
        ipc_auto,
        ipc_endpoint,
        ipc_token_file,
    })
}

fn search_usage() -> &'static str {
    "usage: resume-cli search <query> [--ipc auto|<http://127.0.0.1:port/search|/status> --ipc-token-file <path>] [--mode fulltext|semantic|hybrid] [--embedding-command <path>] [--model-id <id>] [--dimension <n>] [--vector-top-k <n>] [--embedding-timeout-ms <ms>] [--degree <level>] [--skills-any <skill[,skill...]>] [--years-experience-min <years>] [--top-k <n>]"
}

fn parse_search_ipc_endpoint(value: &str) -> Result<IpcSearchEndpoint> {
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| CliError::usage(search_usage()))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| CliError::usage(search_usage()))?;
    if path != "search" && path != "status" {
        return Err(CliError::usage(search_usage()));
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| CliError::usage(search_usage()))?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage("search ipc endpoint must be loopback"));
    }

    Ok(IpcSearchEndpoint { addr })
}

fn parse_detail_args(args: &[String]) -> Result<DetailArgs> {
    let mut doc_id = None;
    let mut ipc_auto = false;
    let mut ipc_endpoint = None;
    let mut ipc_token_file = None;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--doc-id" => {
                if doc_id.is_some() {
                    return Err(CliError::usage(detail_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(detail_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(detail_usage()));
                }
                doc_id = Some(value.clone());
                index += 2;
            }
            "--ipc" => {
                if ipc_auto || ipc_endpoint.is_some() {
                    return Err(CliError::usage(detail_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(detail_usage()));
                };
                if value == "auto" {
                    ipc_auto = true;
                } else {
                    ipc_endpoint = Some(parse_detail_ipc_endpoint(value)?);
                }
                index += 2;
            }
            "--ipc-token-file" => {
                if ipc_token_file.is_some() {
                    return Err(CliError::usage(detail_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(detail_usage()));
                };
                ipc_token_file = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err(CliError::usage(detail_usage())),
        }
    }

    if ipc_auto && ipc_token_file.is_some() {
        return Err(CliError::usage(detail_usage()));
    }
    if !ipc_auto && ipc_endpoint.is_some() != ipc_token_file.is_some() {
        return Err(CliError::usage(detail_usage()));
    }

    Ok(DetailArgs {
        doc_id: doc_id.ok_or_else(|| CliError::usage(detail_usage()))?,
        ipc_auto,
        ipc_endpoint,
        ipc_token_file,
    })
}

fn detail_usage() -> &'static str {
    "usage: resume-cli detail --doc-id <doc_id> [--ipc auto|<http://127.0.0.1:port/details|/status> --ipc-token-file <path>]"
}

fn parse_detail_ipc_endpoint(value: &str) -> Result<IpcDetailEndpoint> {
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| CliError::usage(detail_usage()))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| CliError::usage(detail_usage()))?;
    if path != "details" && path != "status" {
        return Err(CliError::usage(detail_usage()));
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| CliError::usage(detail_usage()))?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage("detail ipc endpoint must be loopback"));
    }

    Ok(IpcDetailEndpoint { addr })
}

fn run_semantic_search(
    data_dir: &Path,
    store: &MetaStore,
    search_args: &SearchArgs,
    candidate_limit: usize,
) -> Result<Vec<SearchOutputHit>> {
    let command = search_args.embedding_command.clone().ok_or_else(|| {
        CliError::user("semantic search blocked: local embedding command not configured")
    })?;
    let model_id = search_args.model_id.as_deref().ok_or_else(|| {
        CliError::user("semantic search blocked: embedding model id not configured")
    })?;
    let snapshot_dimension = vector_snapshot_dimension(data_dir)?;
    let dimension = search_args.dimension.unwrap_or(snapshot_dimension);
    let embedder = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), model_id, dimension)
            .map_err(CliError::embedding)?
            .with_timeout_ms(search_args.embedding_timeout_ms)
            .map_err(CliError::embedding)?,
    );
    let input = EmbeddingInput::new("query", search_args.query.as_str());
    let query_vectors = embedder
        .embed_batch(
            &[input],
            EmbeddingBudget::new(1, search_args.query.len().max(1)),
        )
        .map_err(CliError::embedding)?;
    let query_vector = query_vectors
        .into_iter()
        .next()
        .ok_or_else(|| CliError::user("semantic search query embedding is unavailable"))?;
    let vector_index = PersistentVectorIndex::open(data_dir.join("vector-index"), dimension)
        .map_err(CliError::vector)?;
    let vector_limit = search_args.vector_top_k.unwrap_or(candidate_limit);
    let vector_hits = vector_index
        .knn_for_model(
            QueryVector::new(query_vector.values().to_vec()).map_err(CliError::vector)?,
            vector_limit,
            model_id,
        )
        .map_err(CliError::vector)?;

    vector_output_hits(store, vector_hits, &search_args.filters, search_args.top_k)
}

fn vector_snapshot_dimension(data_dir: &Path) -> Result<usize> {
    let inspection = inspect_persistent_vector_snapshot(data_dir.join("vector-index"));
    match (inspection.state(), inspection.snapshot()) {
        (PersistentVectorSnapshotState::Ready, Some(snapshot)) => Ok(snapshot.dimension()),
        (PersistentVectorSnapshotState::Missing, _) => Err(CliError::user(
            "semantic search unavailable: vector index is missing",
        )),
        (PersistentVectorSnapshotState::Corrupt, _) => Err(CliError::user(
            "semantic search unavailable: vector index is corrupt",
        )),
        (PersistentVectorSnapshotState::Unreadable, _) => Err(CliError::user(
            "semantic search unavailable: vector index is unreadable",
        )),
        _ => Err(CliError::user(
            "semantic search unavailable: vector index is not ready",
        )),
    }
}

fn vector_output_hits(
    store: &MetaStore,
    hits: Vec<VectorHit>,
    filters: &SearchFilters,
    top_k: usize,
) -> Result<Vec<SearchOutputHit>> {
    let mut visible = Vec::new();
    let mut seen_candidate_keys = BTreeSet::new();

    for (rank, hit) in hits.into_iter().enumerate() {
        let Some((document, version)) = hydrate_visible_document_version(store, hit.doc_id())?
        else {
            continue;
        };
        if !filters.is_empty()
            && !filters.matches(&persisted_profile(store, hit.doc_id(), &version)?)
        {
            continue;
        }

        let candidate_key = candidate_fold_key(&version);
        if !seen_candidate_keys.insert(candidate_key.clone()) {
            continue;
        }

        visible.push(SearchOutputHit {
            rank: rank + 1,
            score: hit.score(),
            doc_id: document.id.to_string(),
            version_id: version.id.to_string(),
            file_name: redact_contact_values(&document.file_name),
            snippet: "semantic match".to_string(),
            candidate_key,
        });
        if visible.len() == top_k {
            break;
        }
    }

    Ok(rerank_output_hits(visible))
}

fn fuse_hybrid_output_hits(
    fulltext_hits: Vec<SearchOutputHit>,
    vector_hits: Vec<SearchOutputHit>,
    top_k: usize,
) -> Vec<SearchOutputHit> {
    let mut by_doc = BTreeMap::<String, SearchOutputHit>::new();
    for hit in vector_hits.iter().chain(fulltext_hits.iter()) {
        by_doc.insert(hit.doc_id.clone(), hit.clone());
    }
    let fulltext_ranked = ranked_hits_from_output(&fulltext_hits);
    let vector_ranked = ranked_hits_from_output(&vector_hits);
    let fused = fuse_hybrid_rrf(
        HybridRecall::new(fulltext_ranked, vector_ranked),
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

fn ranked_hits_from_output(hits: &[SearchOutputHit]) -> Vec<RankedHit> {
    hits.iter()
        .enumerate()
        .map(|(index, hit)| {
            RankedHit::new(hit.doc_id.clone(), index + 1, hit.score)
                .with_candidate_key(hit.candidate_key.clone())
        })
        .collect()
}

fn visible_hits(
    store: &MetaStore,
    hits: Vec<SearchHit>,
    top_k: usize,
) -> Result<Vec<SearchOutputHit>> {
    let mut visible = Vec::new();
    let mut seen_candidate_keys = BTreeSet::new();

    for hit in hits {
        let Some(version) = hydrate_visible_version(store, &hit)? else {
            continue;
        };
        let candidate_key = candidate_fold_key(&version);
        if !seen_candidate_keys.insert(candidate_key.clone()) {
            continue;
        }

        visible.push(SearchOutputHit::from_fulltext(hit, candidate_key));
        if visible.len() == top_k {
            break;
        }
    }

    Ok(rerank_output_hits(visible))
}

fn filter_hits(
    store: &MetaStore,
    hits: Vec<SearchHit>,
    filters: &SearchFilters,
    top_k: usize,
) -> Result<Vec<SearchOutputHit>> {
    let mut filtered = Vec::new();
    let mut seen_candidate_keys = BTreeSet::new();

    for hit in hits {
        let Some(version) = hydrate_visible_version(store, &hit)? else {
            continue;
        };
        let profile = persisted_profile(store, &hit.doc_id, &version)?;
        if !filters.matches(&profile) {
            continue;
        }
        let candidate_key = candidate_fold_key(&version);
        if !seen_candidate_keys.insert(candidate_key.clone()) {
            continue;
        }

        filtered.push(SearchOutputHit::from_fulltext(hit, candidate_key));
        if filtered.len() == top_k {
            break;
        }
    }

    Ok(rerank_output_hits(filtered))
}

fn rerank_output_hits(mut hits: Vec<SearchOutputHit>) -> Vec<SearchOutputHit> {
    for (index, hit) in hits.iter_mut().enumerate() {
        hit.rank = index + 1;
    }
    hits
}

fn candidate_fold_key(version: &ResumeVersion) -> String {
    version
        .candidate_id
        .as_ref()
        .map(|candidate_id| format!("candidate:{}", candidate_id.as_str()))
        .unwrap_or_else(|| format!("doc:{}", version.document_id.as_str()))
}

fn hydrate_visible_version(store: &MetaStore, hit: &SearchHit) -> Result<Option<ResumeVersion>> {
    let Ok(document_id) = DocumentId::from_str(&hit.doc_id) else {
        return Ok(None);
    };
    let Some(document) = store
        .document_by_id(&document_id)
        .map_err(CliError::store)?
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
        .map_err(CliError::store)?
    else {
        return Ok(None);
    };
    if version.document_id != document_id {
        return Ok(None);
    }
    if version.visibility != ResumeVisibility::Searchable {
        return Ok(None);
    }

    Ok(Some(version))
}

fn persisted_profile(
    store: &MetaStore,
    doc_id: &str,
    version: &ResumeVersion,
) -> Result<ResumeProfile> {
    let fields = store
        .entity_mentions_for_version(&version.id)
        .map_err(CliError::store)?;
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

fn hydrate_visible_document_version(
    store: &MetaStore,
    doc_id: &str,
) -> Result<Option<(Document, ResumeVersion)>> {
    let Ok(document_id) = DocumentId::from_str(doc_id) else {
        return Ok(None);
    };
    let Some(document) = store
        .document_by_id(&document_id)
        .map_err(CliError::store)?
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

    let version = store
        .resume_versions_for_document(&document_id)
        .map_err(CliError::store)?
        .into_iter()
        .find(|version| version.visibility == ResumeVisibility::Searchable);

    Ok(version.map(|version| (document, version)))
}

#[derive(Clone)]
struct SearchOutputHit {
    rank: usize,
    score: f32,
    doc_id: String,
    version_id: String,
    file_name: String,
    snippet: String,
    candidate_key: String,
}

impl SearchOutputHit {
    fn from_fulltext(hit: SearchHit, candidate_key: String) -> Self {
        Self {
            rank: hit.rank,
            score: hit.score,
            doc_id: hit.doc_id,
            version_id: hit.version_id,
            file_name: hit.file_name,
            snippet: hit.snippet,
            candidate_key,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchMode {
    FullText,
    Semantic,
    Hybrid,
}

impl SearchMode {
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
struct SearchArgs {
    query: String,
    top_k: usize,
    filters: SearchFilters,
    mode: SearchMode,
    embedding_command: Option<PathBuf>,
    model_id: Option<String>,
    dimension: Option<usize>,
    vector_top_k: Option<usize>,
    embedding_timeout_ms: u64,
    ipc_auto: bool,
    ipc_endpoint: Option<IpcSearchEndpoint>,
    ipc_token_file: Option<PathBuf>,
}

struct DetailArgs {
    doc_id: String,
    ipc_auto: bool,
    ipc_endpoint: Option<IpcDetailEndpoint>,
    ipc_token_file: Option<PathBuf>,
}

struct ResumeDetail {
    doc_id: String,
    version_id: String,
    file_name: String,
    extension: String,
    document_status: String,
    visibility: String,
    byte_size: u64,
    fields: Vec<ResumeDetailField>,
    snippet: String,
}

struct ResumeDetailField {
    field_type: String,
    value: String,
    confidence: f64,
    evidence: String,
    extractor: String,
}

fn inspect_search_index(data_dir: &Path) -> SearchIndexDiagnostic {
    let index_root = data_dir.join("search-index");
    let inspection = match inspect_snapshot_root(&index_root) {
        Ok(inspection) => inspection,
        Err(_) => {
            return SearchIndexDiagnostic::Corrupt {
                read_target: None,
                fallback_used: false,
                staging_orphans: 0,
            };
        }
    };

    match inspection.state() {
        SnapshotRootState::Missing => {
            return SearchIndexDiagnostic::Unavailable {
                staging_orphans: inspection.staging_orphans(),
            };
        }
        SnapshotRootState::Corrupt | SnapshotRootState::ActiveMissing => {
            return SearchIndexDiagnostic::Corrupt {
                read_target: inspection.read_target(),
                fallback_used: inspection.fallback_snapshot().is_some(),
                staging_orphans: inspection.staging_orphans(),
            };
        }
        SnapshotRootState::Ready | SnapshotRootState::Recovered => {}
    }

    let fallback_used = inspection.fallback_snapshot().is_some();
    let Ok(Some(index)) = FullTextIndex::open_active(&index_root) else {
        return SearchIndexDiagnostic::Corrupt {
            read_target: inspection.read_target(),
            fallback_used,
            staging_orphans: inspection.staging_orphans(),
        };
    };

    let started_at = Instant::now();
    match index.search(SearchQuery::new("diagnostic").with_limit(1)) {
        Ok(hits) => SearchIndexDiagnostic::Available {
            elapsed_ms: started_at.elapsed().as_millis(),
            results: hits.len(),
            read_target: inspection.read_target(),
            fallback_used,
            staging_orphans: inspection.staging_orphans(),
        },
        Err(_) => SearchIndexDiagnostic::Corrupt {
            read_target: inspection.read_target(),
            fallback_used,
            staging_orphans: inspection.staging_orphans(),
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchIndexDiagnostic {
    Unavailable {
        staging_orphans: usize,
    },
    Corrupt {
        read_target: Option<SnapshotReadTarget>,
        fallback_used: bool,
        staging_orphans: usize,
    },
    Available {
        elapsed_ms: u128,
        results: usize,
        read_target: Option<SnapshotReadTarget>,
        fallback_used: bool,
        staging_orphans: usize,
    },
}

impl SearchIndexDiagnostic {
    fn index_label(self) -> String {
        match self {
            Self::Unavailable { .. } => "unavailable".to_string(),
            Self::Corrupt { fallback_used, .. } if fallback_used => {
                "recovered (full-text snapshot)".to_string()
            }
            Self::Corrupt { .. } => "corrupt".to_string(),
            Self::Available {
                fallback_used: true,
                ..
            } => "recovered (full-text snapshot)".to_string(),
            Self::Available {
                read_target: Some(SnapshotReadTarget::PublishedSnapshot),
                ..
            } => "available (full-text snapshot)".to_string(),
            Self::Available { .. } => "available (full-text)".to_string(),
        }
    }

    fn state_label(self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "unavailable",
            Self::Corrupt { fallback_used, .. } | Self::Available { fallback_used, .. }
                if fallback_used =>
            {
                "recovered"
            }
            Self::Corrupt { .. } => "corrupt",
            Self::Available { .. } => "available",
        }
    }

    fn read_target_label(self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "none",
            Self::Corrupt { read_target, .. } | Self::Available { read_target, .. } => {
                read_target.map(SnapshotReadTarget::label).unwrap_or("none")
            }
        }
    }

    fn snapshot_fallback_label(self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "none",
            Self::Corrupt { fallback_used, .. } | Self::Available { fallback_used, .. } => {
                if fallback_used {
                    "used"
                } else {
                    "none"
                }
            }
        }
    }

    fn staging_orphans(self) -> usize {
        match self {
            Self::Unavailable { staging_orphans }
            | Self::Corrupt {
                staging_orphans, ..
            }
            | Self::Available {
                staging_orphans, ..
            } => staging_orphans,
        }
    }

    fn query_smoke_label(self) -> String {
        match self {
            Self::Unavailable { .. } => "skipped (no full-text index)".to_string(),
            Self::Corrupt { .. } => "skipped (index unavailable)".to_string(),
            Self::Available {
                elapsed_ms,
                results,
                ..
            } => {
                format!("ok (elapsed_ms={elapsed_ms}, results={results})")
            }
        }
    }

    fn query_smoke_json_label(self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "skipped_no_fulltext_index",
            Self::Corrupt { .. } => "skipped_index_unavailable",
            Self::Available { .. } => "ok",
        }
    }
}

fn inspect_vector_index(data_dir: &Path) -> VectorIndexDiagnostic {
    VectorIndexDiagnostic {
        inspection: inspect_persistent_vector_snapshot(data_dir.join("vector-index")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VectorIndexDiagnostic {
    inspection: PersistentVectorSnapshotInspection,
}

impl VectorIndexDiagnostic {
    fn index_label(self) -> &'static str {
        match self.inspection.state() {
            PersistentVectorSnapshotState::Missing => "unavailable",
            PersistentVectorSnapshotState::Ready => "available (vector snapshot)",
            PersistentVectorSnapshotState::Corrupt => "corrupt",
            PersistentVectorSnapshotState::Unreadable => "unreadable",
        }
    }

    fn state_label(self) -> &'static str {
        match self.inspection.state() {
            PersistentVectorSnapshotState::Missing => "unavailable",
            PersistentVectorSnapshotState::Ready => "available",
            PersistentVectorSnapshotState::Corrupt => "corrupt",
            PersistentVectorSnapshotState::Unreadable => "unreadable",
        }
    }

    fn vector_count(self) -> usize {
        self.inspection
            .snapshot()
            .map(|snapshot| snapshot.vector_count())
            .unwrap_or(0)
    }

    fn deleted_count(self) -> usize {
        self.inspection
            .snapshot()
            .map(|snapshot| snapshot.deleted_count())
            .unwrap_or(0)
    }
}

fn open_store(data_dir: &Path) -> Result<MetaStore> {
    fs::create_dir_all(data_dir)
        .map_err(|_| CliError::user("unable to prepare local metadata directory"))?;
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).map_err(CliError::store)?;
    store.run_migrations().map_err(CliError::store)?;
    Ok(store)
}

fn current_timestamp() -> Result<UnixTimestamp> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliError::user("system clock is before the Unix epoch"))?
        .as_secs();
    let seconds = i64::try_from(seconds).map_err(|_| CliError::user("system clock is invalid"))?;
    Ok(UnixTimestamp::from_unix_seconds(seconds))
}

fn new_import_task_id() -> Result<ImportTaskId> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliError::user("system clock is before the Unix epoch"))?;
    let nanos = duration.as_nanos().to_string();
    let pid = std::process::id().to_string();

    Ok(ImportTaskId::from_non_secret_parts(&[
        "s4-import-task",
        &nanos,
        &pid,
    ]))
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

type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug)]
struct CliError {
    message: String,
    exit_code: i32,
}

impl CliError {
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

    fn fulltext(error: index_fulltext::FullTextError) -> Self {
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

    fn import(error: import_pipeline::ImportPipelineError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn privacy(error: privacy::PrivacyError) -> Self {
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

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_worker_debug_output_redacts_candidate_text_and_command_path() {
        let candidate = EmbedWorkerCandidate {
            document_id: DocumentId::from_non_secret_parts(&["debug-doc"]),
            version_id: ResumeVersionId::from_non_secret_parts(&["debug-version"]),
            text: "PRIVATE resume text".to_string(),
        };
        let candidate_debug = format!("{candidate:?}");
        assert!(!candidate_debug.contains("PRIVATE"));
        assert!(candidate_debug.contains("text_bytes"));

        let args = EmbedWorkerArgs {
            command: Some(PathBuf::from("/private/local/embed-command")),
            model_id: Some("local-model".to_string()),
            dimension: Some(4),
            max_docs: 8,
            max_text_bytes: 1000,
            timeout_ms: 5000,
        };
        let args_debug = format!("{args:?}");
        assert!(!args_debug.contains("/private/local/embed-command"));
        assert!(args_debug.contains("command_configured"));
        assert!(args_debug.contains("<redacted>"));
    }
}
