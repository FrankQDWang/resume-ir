use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use import_pipeline::{
    import_root_with_options, rebuild_full_text_index, ImportOptions, ImportSummary, ScanProfile,
};
use index_fulltext::{
    inspect_snapshot_root, FullTextIndex, SearchHit, SearchQuery, SnapshotReadTarget,
    SnapshotRootState,
};
use meta_store::{
    DocumentId, DocumentStatus, EntityType, ImportRootKind as StoreImportRootKind,
    ImportRootPreset as StoreImportRootPreset, ImportScanProfile as StoreImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus, IndexStateStatus, IngestJobKind,
    IngestJobStatus, MetaStore, OcrPageCacheEntry, OcrPageCacheKey, ResumeVersion, ResumeVersionId,
    ResumeVisibility, UnixTimestamp, WorkerTaskKind,
};
use ocr_client::{
    CancellationToken, LocalOcrCommandClient, LocalOcrCommandSpec, OcrClient, OcrOptions,
    OcrPageRequest, OcrWorkerBudget, RenderedPage,
};
use privacy::inspect_contact_hash_key;
use rank_fusion::{DegreeLevel, ResumeProfile, SearchFilters};
use search_planner::plan_search;

const LOCAL_DISCOVERY_ROOTS_ENV: &str = "RESUME_IR_LOCAL_DISCOVERY_ROOTS";

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
            "expected command: status, import, search, delete, pause, resume, ocr-worker, doctor, or export-diagnostics",
        ));
    };

    match command {
        "status" => status_command(&data_dir, &args[1..]),
        "import" => import_command(&data_dir, &args[1..]),
        "search" => search_command(&data_dir, &args[1..]),
        "delete" => delete_command(&data_dir, &args[1..]),
        "pause" => task_control_command(&data_dir, &args[1..], true),
        "resume" => task_control_command(&data_dir, &args[1..], false),
        "ocr-worker" => ocr_worker_command(&data_dir, &args[1..]),
        "doctor" => {
            if args.len() != 1 {
                return Err(CliError::usage("usage: resume-cli doctor"));
            }
            doctor_command(&data_dir)
        }
        "export-diagnostics" => export_diagnostics_command(&data_dir, &args[1..]),
        _ => Err(CliError::usage(
            "expected command: status, import, search, delete, pause, resume, ocr-worker, doctor, or export-diagnostics",
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
    if let Some(endpoint) = parse_status_ipc_arg(args)? {
        return status_ipc_command(&endpoint);
    }

    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(CliError::store)?;
    let ocr_task = store
        .worker_task_control(WorkerTaskKind::Ocr)
        .map_err(CliError::store)?;
    let index_diagnostic = inspect_search_index(data_dir);

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
    println!("import scan scopes: {}", summary.import_scan_scopes);
    println!("active profile: balanced");
    println!("index health: {}", index_health_label(summary.index_health));
    println!(
        "last snapshot: {}",
        summary.last_snapshot_id.as_deref().unwrap_or("none")
    );
    println!("search index: {}", index_diagnostic.index_label());

    Ok(())
}

fn parse_status_ipc_arg(args: &[String]) -> Result<Option<IpcStatusEndpoint>> {
    if args.is_empty() {
        return Ok(None);
    }
    if args.len() != 2 || args.first().map(String::as_str) != Some("--ipc") {
        return Err(CliError::usage(status_usage()));
    }

    parse_status_ipc_endpoint(&args[1]).map(Some)
}

fn status_usage() -> &'static str {
    "usage: resume-cli status [--ipc <http://127.0.0.1:port/status>]"
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
    render_ipc_status(&body);
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
        "import scan scopes: {}",
        json_u64(body, "import_scan_scopes")
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

fn import_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let import_args = parse_import_args(args)?;
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
            &ImportSummary::default(),
            now,
        )?;
        let root_summary = import_root_with_options(
            data_dir,
            &store,
            task,
            &root.canonical,
            now,
            ImportOptions {
                scan_profile: import_args.profile,
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

    Ok(())
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ImportRootSelection {
    Explicit(Vec<PathBuf>),
    Preset(RootPreset),
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
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
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
            _ => return Err(import_usage()),
        }
        index += 1;
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

    Ok(ImportArgs {
        root_selection,
        profile: profile.unwrap_or(default_profile),
    })
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

fn import_usage() -> CliError {
    CliError::usage(
        "usage: resume-cli import (--root <path> [--root <path> ...] | --root-preset local-discovery) [--profile explicit|discovery]",
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

    let Some(index) =
        FullTextIndex::open_active(&data_dir.join("search-index")).map_err(CliError::fulltext)?
    else {
        println!("search index not available yet");
        println!("results: 0");
        return Ok(());
    };

    let candidate_limit = search_args
        .top_k
        .saturating_mul(5)
        .clamp(search_args.top_k, 100);
    let plan = plan_search(&search_args.query, candidate_limit)
        .map_err(|_| CliError::user("search query is empty"))?;
    let hits = index
        .search(SearchQuery::new(plan.query_text()).with_limit(plan.limit()))
        .map_err(CliError::fulltext)?;
    let store = open_store(data_dir)?;
    let hits = if search_args.filters.is_empty() {
        visible_hits(&store, hits, search_args.top_k)?
    } else {
        filter_hits(&store, hits, &search_args.filters, search_args.top_k)?
    };

    println!("results: {}", hits.len());
    for hit in hits {
        println!("rank: {}", hit.rank);
        println!("doc_id: {}", hit.doc_id);
        println!("version_id: {}", hit.version_id);
        println!("file_name: {}", hit.file_name);
        println!("snippet: {}", hit.snippet);
    }

    Ok(())
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
    _data_dir: &Path,
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
        let _ = entry;
        document.status = DocumentStatus::OcrDone;
        document.updated_at = now;
        store.upsert_document(&document).map_err(CliError::store)?;
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
            document.status = DocumentStatus::OcrDone;
            document.updated_at = now;
            store.upsert_document(&document).map_err(CliError::store)?;
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

fn doctor_command(data_dir: &Path) -> Result<()> {
    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(CliError::store)?;
    let index_diagnostic = inspect_search_index(data_dir);
    let contact_key = inspect_contact_hash_key(data_dir).map_err(CliError::privacy)?;

    println!("resume-ir doctor");
    println!("metadata: ok");
    println!("indexed documents: {}", summary.indexed_documents);
    println!("searchable documents: {}", summary.searchable_documents);
    println!("ocr queue: {}", summary.ocr_queue_depth);
    println!("ocr jobs queued: {}", summary.ocr_jobs_queued);
    println!("entity mentions: {}", summary.entity_mentions);
    println!("import scan scopes: {}", summary.import_scan_scopes);
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
        "    \"recovery_queue_depth\": {}",
        summary.recovery_queue_depth
    );
    println!("  }},");
    println!(
        "  \"search_index_state\": \"{}\",",
        index_diagnostic.state_label()
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
    let mut index = 1_usize;

    while index < args.len() {
        match args[index].as_str() {
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

    Ok(SearchArgs {
        query: query.clone(),
        top_k,
        filters,
    })
}

fn search_usage() -> &'static str {
    "usage: resume-cli search <query> [--degree <level>] [--skills-any <skill[,skill...]>] [--years-experience-min <years>] [--top-k <n>]"
}

fn visible_hits(store: &MetaStore, hits: Vec<SearchHit>, top_k: usize) -> Result<Vec<SearchHit>> {
    let mut visible = Vec::new();
    let mut seen_candidate_keys = BTreeSet::new();

    for hit in hits {
        let Some(version) = hydrate_visible_version(store, &hit)? else {
            continue;
        };
        if !seen_candidate_keys.insert(candidate_fold_key(&version)) {
            continue;
        }

        let mut hit = hit;
        hit.rank = visible.len() + 1;
        visible.push(hit);
        if visible.len() == top_k {
            break;
        }
    }

    Ok(visible)
}

fn filter_hits(
    store: &MetaStore,
    hits: Vec<SearchHit>,
    filters: &SearchFilters,
    top_k: usize,
) -> Result<Vec<SearchHit>> {
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
        if !seen_candidate_keys.insert(candidate_fold_key(&version)) {
            continue;
        }

        let mut hit = hit;
        hit.rank = filtered.len() + 1;
        filtered.push(hit);
        if filtered.len() == top_k {
            break;
        }
    }

    Ok(filtered)
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

#[derive(Clone)]
struct SearchArgs {
    query: String,
    top_k: usize,
    filters: SearchFilters,
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

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
