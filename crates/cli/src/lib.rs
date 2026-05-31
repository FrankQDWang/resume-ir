//! Command-line interface skeleton for local resume indexing.

use index_fulltext::{FullTextError, FullTextIndexReader};
use meta_store::MetadataStore;
use search_planner::SearchOptions;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Returns the crate name for smoke tests and workspace metadata.
#[must_use]
pub fn crate_name() -> &'static str {
    "resume-cli"
}

/// Runs the CLI with explicit arguments and output sink.
pub fn run_with_args<I, S, W>(args: I, output: &mut W) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    W: Write,
{
    let options = CliOptions::parse(args)?;
    match options.command {
        Command::Status => {
            let store = open_store(&options.data_dir)?;
            let status = store
                .status()
                .map_err(|error| error.user_message().to_string())?;
            writeln!(output, "metadata schema: {}", status.schema_version)
                .map_err(|error| error.to_string())?;
            writeln!(
                output,
                "visible documents: {}",
                status.visible_document_count
            )
            .map_err(|error| error.to_string())?;
            writeln!(
                output,
                "queued imports: {}",
                status.queued_import_task_count
            )
            .map_err(|error| error.to_string())?;
            writeln!(output, "index states: {}", status.index_state_count)
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        Command::Import { root } => {
            if !root.is_dir() {
                return Err("Import root must be an existing directory.".to_string());
            }
            let store = open_store(&options.data_dir)?;
            let task_id = store
                .enqueue_import_root(&root)
                .map_err(|error| error.user_message().to_string())?;
            writeln!(output, "queued import task: {task_id}").map_err(|error| error.to_string())
        }
        Command::Search { query } => {
            let trimmed = query.trim();
            if trimmed.is_empty() {
                return Err("Search query must not be empty.".to_string());
            }
            let index_dir = fulltext_index_dir(&options.data_dir);
            let reader = match FullTextIndexReader::open_existing(&index_dir) {
                Ok(reader) => reader,
                Err(FullTextError::MissingIndex) => {
                    let store = open_store(&options.data_dir)?;
                    let status = store
                        .status()
                        .map_err(|error| error.user_message().to_string())?;
                    writeln!(
                        output,
                        "search index is not available yet; indexed states: {}",
                        status.index_state_count
                    )
                    .map_err(|error| error.to_string())?;
                    return Ok(());
                }
                Err(error) => return Err(error.to_string()),
            };
            let hits = reader
                .search(trimmed, SearchOptions::default())
                .map_err(|error| error.to_string())?;
            if hits.is_empty() {
                writeln!(output, "no search results").map_err(|error| error.to_string())?;
                return Ok(());
            }
            for hit in hits {
                writeln!(
                    output,
                    "rank={} doc_id={} file_name={} snippet={}",
                    hit.rank,
                    hit.doc_id,
                    hit.file_name,
                    single_line(&hit.snippet)
                )
                .map_err(|error| error.to_string())?;
            }
            Ok(())
        }
    }
}

struct CliOptions {
    data_dir: PathBuf,
    command: Command,
}

enum Command {
    Status,
    Import { root: PathBuf },
    Search { query: String },
}

impl CliOptions {
    fn parse<I, S>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut data_dir = PathBuf::from("local-data");
        let mut command_parts = Vec::new();
        let mut args = args.into_iter();
        let _program = args.next();

        while let Some(arg) = args.next() {
            match arg.as_ref() {
                "--data-dir" => {
                    let Some(value) = args.next() else {
                        return Err("Missing value for --data-dir.".to_string());
                    };
                    data_dir = PathBuf::from(value.as_ref());
                }
                value => command_parts.push(value.to_string()),
            }
        }

        let command = parse_command(&command_parts)?;
        Ok(Self { data_dir, command })
    }
}

fn parse_command(parts: &[String]) -> Result<Command, String> {
    let Some(command) = parts.first() else {
        return Err("Usage: resume-cli [--data-dir <path>] <status|import|search>".to_string());
    };
    match command.as_str() {
        "status" if parts.len() == 1 => Ok(Command::Status),
        "import" => parse_import_command(parts),
        "search" if parts.len() >= 2 => Ok(Command::Search {
            query: parts[1..].join(" "),
        }),
        "search" => Err("Usage: resume-cli search <query>".to_string()),
        _ => Err("Unknown command. Use status, import, or search.".to_string()),
    }
}

fn parse_import_command(parts: &[String]) -> Result<Command, String> {
    if parts.len() != 3 || parts[1] != "--root" {
        return Err("Usage: resume-cli import --root <path>".to_string());
    }
    Ok(Command::Import {
        root: PathBuf::from(&parts[2]),
    })
}

fn open_store(data_dir: &Path) -> Result<MetadataStore, String> {
    fs::create_dir_all(data_dir)
        .map_err(|error| format!("Could not create local data directory: {error}"))?;
    let store = MetadataStore::open(data_dir.join("metadata.sqlite"))
        .map_err(|error| error.user_message().to_string())?;
    store
        .run_migrations()
        .map_err(|error| error.user_message().to_string())?;
    Ok(store)
}

fn fulltext_index_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("indexes").join("fulltext")
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::run_with_args;
    use index_fulltext::{FullTextIndexWriter, IndexDocument};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn status_initializes_store_and_prints_counts() -> Result<(), String> {
        let data_dir = unique_data_dir("status")?;
        let mut output = Vec::new();

        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "status"],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("metadata schema: 2"));
        assert!(text.contains("visible documents: 0"));
        assert!(text.contains("queued imports: 0"));
        assert!(data_dir.join("metadata.sqlite").is_file());
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn import_rejects_missing_root_with_user_readable_error() -> Result<(), String> {
        let data_dir = unique_data_dir("missing-root")?;
        let missing_root = data_dir.join("missing");
        let mut output = Vec::new();

        let error = run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                missing_root.as_str(),
            ],
            &mut output,
        )
        .err()
        .ok_or_else(|| "missing root should have failed".to_string())?;

        assert!(error.contains("Import root must be an existing directory"));
        assert!(!error.contains(missing_root.as_str()));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn import_queues_existing_root_without_printing_path() -> Result<(), String> {
        let data_dir = unique_data_dir("import")?;
        let import_root = data_dir.join("root");
        fs::create_dir_all(&import_root).map_err(|error| error.to_string())?;
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("queued import task: 1"));
        assert!(!text.contains(import_root.as_str()));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn search_reports_unavailable_index_without_results() -> Result<(), String> {
        let data_dir = unique_data_dir("search")?;
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "Java",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("search index is not available yet"));
        assert!(!text.contains("Java"));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn search_reads_existing_fulltext_index_and_prints_ranked_results() -> Result<(), String> {
        let data_dir = unique_data_dir("search-index")?;
        let index_dir = data_dir.join("indexes").join("fulltext");
        let mut writer = FullTextIndexWriter::open_or_create(index_dir.as_ref())
            .map_err(|error| format!("could not create synthetic full-text test index: {error}"))?;
        writer
            .add_document(IndexDocument {
                doc_id: "doc-cli".to_string(),
                version_id: "ver-cli".to_string(),
                file_name: "synthetic-cli.pdf".to_string(),
                clean_text: "Synthetic Java 支付 project experience text".to_string(),
                section_type: "experience".to_string(),
                is_deleted: false,
            })
            .map_err(|error| error.to_string())?;
        writer.commit().map_err(|error| error.to_string())?;
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "Java 支付",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("rank=1"));
        assert!(text.contains("doc_id=doc-cli"));
        assert!(text.contains("file_name=synthetic-cli.pdf"));
        assert!(text.contains("snippet="));
        assert!(!text.contains(index_dir.as_str()));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    fn unique_data_dir(label: &str) -> Result<TestPath, String> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("resume-cli-{label}-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        Ok(TestPath(path))
    }

    struct TestPath(std::path::PathBuf);

    impl TestPath {
        fn join(&self, path: &str) -> Self {
            Self(self.0.join(path))
        }

        fn as_str(&self) -> &str {
            self.0.to_str().map_or("", std::convert::identity)
        }

        fn is_file(&self) -> bool {
            self.0.is_file()
        }
    }

    impl AsRef<std::path::Path> for TestPath {
        fn as_ref(&self) -> &std::path::Path {
            &self.0
        }
    }
}
