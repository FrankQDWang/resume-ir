use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use extractor_rules::{extract_strong_fields, FieldType};
use import_pipeline::import_root;
use index_fulltext::{FullTextIndex, SearchHit, SearchQuery};
use meta_store::{
    DocumentId, DocumentStatus, ImportTask, ImportTaskId, ImportTaskStatus, IndexStateStatus,
    MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
};
use rank_fusion::{DegreeLevel, ResumeProfile, SearchFilters};
use search_planner::plan_search;

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
            "expected command: status, import, or search",
        ));
    };

    match command {
        "status" => {
            if args.len() != 1 {
                return Err(CliError::usage("usage: resume-cli status"));
            }
            status_command(&data_dir)
        }
        "import" => import_command(&data_dir, &args[1..]),
        "search" => search_command(&data_dir, &args[1..]),
        _ => Err(CliError::usage(
            "expected command: status, import, or search",
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

fn status_command(data_dir: &Path) -> Result<()> {
    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(CliError::store)?;

    println!("resume-ir status");
    println!("indexed documents: {}", summary.indexed_documents);
    println!("searchable documents: {}", summary.searchable_documents);
    println!("partial documents: {}", summary.partial_documents);
    println!("failed retryable: {}", summary.failed_retryable);
    println!("failed permanent: {}", summary.failed_permanent);
    println!("recovery queue: {}", summary.recovery_queue_depth);
    println!("ocr queue: {}", summary.ocr_queue_depth);
    println!("embedding queue: {}", summary.embedding_queue_depth);
    println!("import tasks queued: {}", summary.import_tasks_queued);
    println!(
        "import tasks recoverable: {}",
        summary.import_tasks_recoverable
    );
    println!("active profile: balanced");
    println!("index health: {}", index_health_label(summary.index_health));
    println!(
        "last snapshot: {}",
        summary.last_snapshot_id.as_deref().unwrap_or("none")
    );
    if data_dir.join("search-index").join("meta.json").exists() {
        println!("search index: available (full-text)");
    } else {
        println!("search index: unavailable (no full-text index snapshot)");
    }

    Ok(())
}

fn import_command(data_dir: &Path, args: &[String]) -> Result<()> {
    if args.len() != 2 || args.first().map(String::as_str) != Some("--root") {
        return Err(CliError::usage("usage: resume-cli import --root <path>"));
    }

    let requested_root = PathBuf::from(&args[1]);
    let requested_root_path = requested_root.as_os_str().to_string_lossy().into_owned();
    let metadata = fs::metadata(&requested_root)
        .map_err(|_| CliError::user("import root must exist and be a directory"))?;
    if !metadata.is_dir() {
        return Err(CliError::user("import root must exist and be a directory"));
    }
    let root = fs::canonicalize(&requested_root)
        .map_err(|_| CliError::user("import root must exist and be a directory"))?;

    let store = open_store(data_dir)?;
    let now = current_timestamp()?;
    let root_path = root.as_os_str().to_string_lossy().into_owned();
    let task = match pending_import_task(&store, &root_path, &requested_root_path)? {
        Some(task) if task.status == ImportTaskStatus::Running => {
            return Err(CliError::user("import task is already running"));
        }
        Some(task) => task,
        None => {
            let task = ImportTask {
                id: new_import_task_id()?,
                root_path,
                status: ImportTaskStatus::Queued,
                queued_at: now,
                started_at: None,
                finished_at: None,
                updated_at: now,
            };
            store.insert_import_task(&task).map_err(CliError::store)?;
            task
        }
    };

    let summary = import_root(data_dir, &store, &task, &root, now).map_err(CliError::import)?;

    println!("import task submitted");
    println!("task id: {}", task.id);
    println!("status: completed");
    println!("files discovered: {}", summary.files_discovered);
    println!("searchable documents: {}", summary.searchable_documents);
    println!("ocr required documents: {}", summary.ocr_required_documents);
    println!("failed documents: {}", summary.failed_documents);
    println!("scan errors: {}", summary.scan_errors);

    Ok(())
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

    let index_dir = data_dir.join("search-index");
    if !index_dir.join("meta.json").exists() {
        println!("search index not available yet");
        println!("results: 0");
        return Ok(());
    }

    let candidate_limit = if search_args.filters.is_empty() {
        search_args.top_k
    } else {
        search_args
            .top_k
            .saturating_mul(5)
            .clamp(search_args.top_k, 100)
    };
    let plan = plan_search(&search_args.query, candidate_limit)
        .map_err(|_| CliError::user("search query is empty"))?;
    let index = FullTextIndex::open(&index_dir).map_err(CliError::fulltext)?;
    let hits = index
        .search(SearchQuery::new(plan.query_text()).with_limit(plan.limit()))
        .map_err(CliError::fulltext)?;
    let hits = if search_args.filters.is_empty() {
        limit_and_rerank_hits(hits, search_args.top_k)
    } else {
        let store = open_store(data_dir)?;
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

fn limit_and_rerank_hits(hits: Vec<SearchHit>, top_k: usize) -> Vec<SearchHit> {
    hits.into_iter()
        .take(top_k)
        .enumerate()
        .map(|(index, mut hit)| {
            hit.rank = index + 1;
            hit
        })
        .collect()
}

fn filter_hits(
    store: &MetaStore,
    hits: Vec<SearchHit>,
    filters: &SearchFilters,
    top_k: usize,
) -> Result<Vec<SearchHit>> {
    let mut filtered = Vec::new();

    for hit in hits {
        let Some(version) = hydrate_visible_version(store, &hit)? else {
            continue;
        };
        let Some(clean_text) = version.clean_text.as_deref() else {
            continue;
        };
        let profile = extracted_profile(&hit.doc_id, clean_text);
        if !filters.matches(&profile) {
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

fn extracted_profile(doc_id: &str, clean_text: &str) -> ResumeProfile {
    let fields = extract_strong_fields(clean_text);
    let degree = fields
        .iter()
        .filter(|field| field.field_type == FieldType::Degree && field.confidence >= 0.75)
        .filter_map(|field| DegreeLevel::parse(field.normalized_value.as_deref()?))
        .max();
    let skills = fields
        .iter()
        .filter(|field| field.field_type == FieldType::Skill && field.confidence >= 0.75)
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    let years_experience = fields
        .iter()
        .filter(|field| field.field_type == FieldType::YearsExperience && field.confidence >= 0.75)
        .filter_map(|field| field.normalized_value.as_deref()?.parse::<f32>().ok())
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));

    let mut profile = ResumeProfile::new(doc_id).with_skills(skills);
    if let Some(degree) = degree {
        profile = profile.with_degree(degree);
    }
    if let Some(years_experience) = years_experience {
        profile = profile.with_years_experience(years_experience);
    }
    profile
}

#[derive(Clone)]
struct SearchArgs {
    query: String,
    top_k: usize,
    filters: SearchFilters,
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

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
