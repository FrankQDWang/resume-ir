use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, SearchQuery};
use meta_store::{
    ImportTask, ImportTaskId, ImportTaskStatus, IndexStateStatus, MetaStore, UnixTimestamp,
};
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

    let root = PathBuf::from(&args[1]);
    let metadata = fs::metadata(&root)
        .map_err(|_| CliError::user("import root must exist and be a directory"))?;
    if !metadata.is_dir() {
        return Err(CliError::user("import root must exist and be a directory"));
    }

    let store = open_store(data_dir)?;
    let now = current_timestamp()?;
    let task = ImportTask {
        id: new_import_task_id()?,
        root_path: root.as_os_str().to_string_lossy().into_owned(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };

    store.insert_import_task(&task).map_err(CliError::store)?;

    println!("import task submitted");
    println!("task id: {}", task.id);
    println!("status: queued");

    Ok(())
}

fn search_command(data_dir: &Path, args: &[String]) -> Result<()> {
    if args.len() != 1 {
        return Err(CliError::usage("usage: resume-cli search <query>"));
    }

    let index_dir = data_dir.join("search-index");
    if !index_dir.join("meta.json").exists() {
        println!("search index not available yet");
        println!("results: 0");
        return Ok(());
    }

    let plan = plan_search(&args[0], 10).map_err(|_| CliError::user("search query is empty"))?;
    let index = FullTextIndex::open(&index_dir).map_err(CliError::fulltext)?;
    let hits = index
        .search(SearchQuery::new(plan.query_text()).with_limit(plan.limit()))
        .map_err(CliError::fulltext)?;

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

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}
