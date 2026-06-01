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
    Document, DocumentId, DocumentStatus, EntityType, ImportRootKind as StoreImportRootKind,
    ImportRootPreset as StoreImportRootPreset, ImportScanBudgetKind as StoreImportScanBudgetKind,
    ImportScanProfile as StoreImportScanProfile, ImportScanScope, ImportTask, ImportTaskId,
    ImportTaskStatus, IndexStateStatus, IngestJobKind, IngestJobStatus, MetaStore,
    OcrPageCacheEntry, OcrPageCacheKey, ResumeVersion, ResumeVersionId, ResumeVisibility,
    UnixTimestamp, WorkerTaskKind,
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

const LOCAL_DISCOVERY_ROOTS_ENV: &str = "RESUME_IR_LOCAL_DISCOVERY_ROOTS";
const LOCAL_DISCOVERY_DEFAULT_MAX_FILES: usize = 10_000;

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
            "expected command: status, import, search, delete, pause, resume, ocr-worker, embed-worker, doctor, or export-diagnostics",
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
        "embed-worker" => embed_worker_command(&data_dir, &args[1..]),
        "doctor" => {
            if args.len() != 1 {
                return Err(CliError::usage("usage: resume-cli doctor"));
            }
            doctor_command(&data_dir)
        }
        "export-diagnostics" => export_diagnostics_command(&data_dir, &args[1..]),
        _ => Err(CliError::usage(
            "expected command: status, import, search, delete, pause, resume, ocr-worker, embed-worker, doctor, or export-diagnostics",
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
    let mut max_files = None;
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

fn parse_positive_usize(value: &str) -> Result<usize> {
    let parsed = value.parse::<usize>().map_err(|_| import_usage())?;
    if parsed == 0 {
        return Err(import_usage());
    }
    Ok(parsed)
}

fn import_usage() -> CliError {
    CliError::usage(
        "usage: resume-cli import (--root <path> [--root <path> ...] | --root-preset local-discovery) [--profile explicit|discovery] [--max-files <count>]",
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
    let inputs = candidates
        .iter()
        .map(|candidate| {
            EmbeddingInput::new(candidate.version_id.as_str(), candidate.text.as_str())
        })
        .collect::<Vec<_>>();
    let vectors = embedder
        .embed_batch(
            &inputs,
            EmbeddingBudget::new(worker_args.max_docs, worker_args.max_text_bytes),
        )
        .map_err(CliError::embedding)?;
    let vector_documents = vectors
        .into_iter()
        .zip(candidates.iter())
        .map(|(vector, candidate)| {
            VectorDocument::new(
                format!("{}:{}", vector.model_id(), vector.id()),
                candidate.document_id.as_str(),
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
    println!("documents embedded: {}", inputs.len());
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
    let mut index = 1_usize;

    while index < args.len() {
        match args[index].as_str() {
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
    })
}

fn search_usage() -> &'static str {
    "usage: resume-cli search <query> [--mode fulltext|semantic|hybrid] [--embedding-command <path>] [--model-id <id>] [--dimension <n>] [--vector-top-k <n>] [--embedding-timeout-ms <ms>] [--degree <level>] [--skills-any <skill[,skill...]>] [--years-experience-min <years>] [--top-k <n>]"
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
        .knn(
            QueryVector::new(query_vector.values().to_vec()).map_err(CliError::vector)?,
            vector_limit,
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
