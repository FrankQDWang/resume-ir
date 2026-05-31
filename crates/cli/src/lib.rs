//! Command-line interface skeleton for local resume indexing.

use core_domain::{Document, DocumentExtension, DocumentId};
use extractor_rules::extract_strong_entities;
use fs_crawler::{Crawler, DiscoveredFile, SupportedExtension};
use index_fulltext::{
    FullTextError, FullTextIndexReader, FullTextIndexWriter, IndexDocument, SearchHit,
};
use meta_store::{JobState, MetadataStore, ParsedResumeRecord};
use parser_common::{ParseInput, Parser, SupportLevel};
use parser_docx::DocxParser;
use parser_pdf::PdfParser;
use rank_fusion::{DegreeLevel, FieldEvidence, FieldFilters, FieldSummary};
use search_planner::SearchOptions;
use sectionizer::sectionize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const CLI_USAGE: &str =
    "Usage: resume-cli [--data-dir <path>] <status|import|search|delete|doctor|export-diagnostics|benchmark>";
const DIAGNOSTIC_QUERY_TEXT: &str = "diagnostic-smoke-token";
const LARGE_CORPUS_THRESHOLD: usize = 100_000;
const MAX_SYNTHETIC_BENCHMARK_COUNT: usize = 1_000_000;

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
            write_status_counts(output, &status)
        }
        Command::Import { root } => {
            if !root.is_dir() {
                return Err("Import root must be an existing directory.".to_string());
            }
            let store = open_store(&options.data_dir)?;
            let task_id = store
                .enqueue_import_root(&root)
                .map_err(|error| error.user_message().to_string())?;
            match run_smoke_import(&store, &options.data_dir, &root) {
                Ok(summary) => {
                    store
                        .update_import_task_state(task_id, JobState::Completed)
                        .map_err(|error| error.user_message().to_string())?;
                    writeln!(output, "queued import task: {task_id}")
                        .map_err(|error| error.to_string())?;
                    writeln!(
                        output,
                        "discovered documents: {}",
                        summary.discovered_documents
                    )
                    .map_err(|error| error.to_string())?;
                    writeln!(
                        output,
                        "searchable documents: {}",
                        summary.searchable_documents
                    )
                    .map_err(|error| error.to_string())?;
                    writeln!(
                        output,
                        "ocr required documents: {}",
                        summary.ocr_required_documents
                    )
                    .map_err(|error| error.to_string())?;
                    writeln!(output, "skipped documents: {}", summary.skipped_documents)
                        .map_err(|error| error.to_string())
                }
                Err(error) => {
                    let _ = store.update_import_task_state(task_id, JobState::Failed);
                    Err(error)
                }
            }
        }
        Command::Search {
            query,
            filters,
            top_k,
        } => {
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
            let search_options = SearchOptions {
                top_k: retrieval_limit(top_k, true),
                ..SearchOptions::default()
            };
            let hits = reader
                .search(trimmed, search_options)
                .map_err(|error| error.to_string())?;
            let hits = filter_hits_by_metadata(hits, &options.data_dir, &filters, top_k)?;
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
        Command::Delete { doc_id } => run_delete(&options.data_dir, &doc_id, output),
        Command::Doctor => run_doctor(&options.data_dir, output),
        Command::ExportDiagnostics { redact, output_dir } => {
            if !redact {
                return Err("Usage: resume-cli export-diagnostics --redact".to_string());
            }
            if let Some(output_dir) = output_dir {
                export_diagnostics_package(&options.data_dir, &output_dir, output)
            } else {
                export_diagnostics(&options.data_dir, output)
            }
        }
        Command::Benchmark {
            synthetic_count,
            query,
        } => run_synthetic_benchmark(&options.data_dir, synthetic_count, &query, output),
    }
}

struct CliOptions {
    data_dir: PathBuf,
    command: Command,
}

enum Command {
    Status,
    Doctor,
    ExportDiagnostics {
        redact: bool,
        output_dir: Option<PathBuf>,
    },
    Import {
        root: PathBuf,
    },
    Search {
        query: String,
        filters: FieldFilters,
        top_k: usize,
    },
    Delete {
        doc_id: String,
    },
    Benchmark {
        synthetic_count: usize,
        query: String,
    },
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
        return Err(CLI_USAGE.to_string());
    };
    match command.as_str() {
        "status" if parts.len() == 1 => Ok(Command::Status),
        "doctor" if parts.len() == 1 => Ok(Command::Doctor),
        "export-diagnostics" => parse_export_diagnostics_command(parts),
        "import" => parse_import_command(parts),
        "search" if parts.len() >= 2 => parse_search_command(parts),
        "search" => Err("Usage: resume-cli search <query>".to_string()),
        "delete" => parse_delete_command(parts),
        "benchmark" => parse_benchmark_command(parts),
        _ => Err(
            "Unknown command. Use status, import, search, delete, doctor, export-diagnostics, or benchmark."
                .to_string(),
        ),
    }
}

fn parse_export_diagnostics_command(parts: &[String]) -> Result<Command, String> {
    let mut redact = false;
    let mut output_dir = None;
    let mut index = 1;

    while index < parts.len() {
        match parts[index].as_str() {
            "--redact" if !redact => {
                redact = true;
                index += 1;
            }
            "--output" if output_dir.is_none() => {
                let Some(value) = parts.get(index + 1) else {
                    return Err("Usage: resume-cli export-diagnostics --redact".to_string());
                };
                if value.starts_with("--") {
                    return Err("Usage: resume-cli export-diagnostics --redact".to_string());
                }
                output_dir = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err("Usage: resume-cli export-diagnostics --redact".to_string()),
        }
    }

    if !redact {
        return Err("Usage: resume-cli export-diagnostics --redact".to_string());
    }
    Ok(Command::ExportDiagnostics { redact, output_dir })
}

fn parse_search_command(parts: &[String]) -> Result<Command, String> {
    let mut query_parts = Vec::new();
    let mut filters = FieldFilters::default();
    let mut top_k = SearchOptions::default().top_k;
    let mut index = 1;

    while index < parts.len() {
        match parts[index].as_str() {
            "--degree" | "--degree-min" => {
                let Some(value) = parts.get(index + 1) else {
                    return Err("Missing value for --degree.".to_string());
                };
                filters.degree_min = Some(
                    value
                        .parse::<DegreeLevel>()
                        .map_err(|_| "Unknown degree filter value.".to_string())?,
                );
                index += 2;
            }
            "--skill" | "--skills-any" => {
                let Some(value) = parts.get(index + 1) else {
                    return Err("Missing value for --skill.".to_string());
                };
                filters.skills_any.extend(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|skill| !skill.is_empty())
                        .map(ToString::to_string),
                );
                index += 2;
            }
            "--years-experience-min" => {
                let Some(value) = parts.get(index + 1) else {
                    return Err("Missing value for --years-experience-min.".to_string());
                };
                filters.years_experience_min = Some(parse_years_filter(value)?);
                index += 2;
            }
            "--top-k" => {
                let Some(value) = parts.get(index + 1) else {
                    return Err("Missing value for --top-k.".to_string());
                };
                top_k = parse_top_k(value)?;
                index += 2;
            }
            value if value.starts_with("--") => {
                return Err(format!("Unknown search option: {value}"));
            }
            value => {
                query_parts.push(value.to_string());
                index += 1;
            }
        }
    }

    if query_parts.is_empty() {
        return Err("Usage: resume-cli search <query>".to_string());
    }

    Ok(Command::Search {
        query: query_parts.join(" "),
        filters,
        top_k,
    })
}

fn parse_years_filter(value: &str) -> Result<f32, String> {
    let parsed = value
        .parse::<f32>()
        .map_err(|_| "Invalid years experience filter value.".to_string())?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err("Invalid years experience filter value.".to_string());
    }
    Ok(parsed)
}

fn parse_top_k(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| "Invalid --top-k value.".to_string())?;
    if !(1..=1000).contains(&parsed) {
        return Err("Invalid --top-k value.".to_string());
    }
    Ok(parsed)
}

fn parse_import_command(parts: &[String]) -> Result<Command, String> {
    if parts.len() != 3 || parts[1] != "--root" {
        return Err("Usage: resume-cli import --root <path>".to_string());
    }
    Ok(Command::Import {
        root: PathBuf::from(&parts[2]),
    })
}

fn parse_delete_command(parts: &[String]) -> Result<Command, String> {
    if parts.len() != 3 || parts[1] != "--doc-id" || parts[2].trim().is_empty() {
        return Err("Usage: resume-cli delete --doc-id <doc_id>".to_string());
    }
    let doc_id = parts[2].trim();
    validate_doc_id(doc_id)?;
    Ok(Command::Delete {
        doc_id: doc_id.to_string(),
    })
}

fn parse_benchmark_command(parts: &[String]) -> Result<Command, String> {
    if parts.len() < 5 {
        return Err(
            "Usage: resume-cli benchmark --synthetic-count <n> --query <query>".to_string(),
        );
    }

    let mut synthetic_count = None;
    let mut query_parts = Vec::new();
    let mut index = 1;
    while index < parts.len() {
        match parts[index].as_str() {
            "--synthetic-count" => {
                let Some(value) = parts.get(index + 1) else {
                    return Err("Missing value for --synthetic-count.".to_string());
                };
                synthetic_count = Some(parse_synthetic_count(value)?);
                index += 2;
            }
            "--query" => {
                let Some(value) = parts.get(index + 1) else {
                    return Err("Missing value for --query.".to_string());
                };
                if value.trim().is_empty() {
                    return Err("Benchmark query must not be empty.".to_string());
                }
                query_parts.push(value.to_string());
                index += 2;
                while index < parts.len() {
                    query_parts.push(parts[index].to_string());
                    index += 1;
                }
            }
            _ => {
                return Err(
                    "Usage: resume-cli benchmark --synthetic-count <n> --query <query>".to_string(),
                )
            }
        }
    }

    let Some(synthetic_count) = synthetic_count else {
        return Err("Missing value for --synthetic-count.".to_string());
    };
    if query_parts.is_empty() {
        return Err("Missing value for --query.".to_string());
    }

    Ok(Command::Benchmark {
        synthetic_count,
        query: query_parts.join(" "),
    })
}

fn parse_synthetic_count(value: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| "Synthetic count must be between 1 and 1000000.".to_string())?;
    if !(1..=MAX_SYNTHETIC_BENCHMARK_COUNT).contains(&parsed) {
        return Err("Synthetic count must be between 1 and 1000000.".to_string());
    }
    Ok(parsed)
}

fn validate_doc_id(doc_id: &str) -> Result<(), String> {
    let Some(suffix) = doc_id.strip_prefix("doc_") else {
        return Err("Invalid doc_id value.".to_string());
    };
    if suffix.is_empty()
        || suffix.len() > 96
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("Invalid doc_id value.".to_string());
    }
    Ok(())
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

fn run_doctor<W: Write>(data_dir: &Path, output: &mut W) -> Result<(), String> {
    let store = open_store(data_dir)?;
    let status = store
        .status()
        .map_err(|error| error.user_message().to_string())?;
    write_status_counts(output, &status)?;

    let inspection = inspect_fulltext_index(data_dir, true);
    writeln!(output, "fulltext index: {}", inspection.status).map_err(|error| error.to_string())?;
    if let Some(smoke) = inspection.query_smoke {
        writeln!(output, "query benchmark smoke: completed").map_err(|error| error.to_string())?;
        writeln!(output, "query benchmark hits: {}", smoke.hits)
            .map_err(|error| error.to_string())?;
        writeln!(output, "query benchmark elapsed_ms: {}", smoke.elapsed_ms)
            .map_err(|error| error.to_string())?;
    } else {
        writeln!(output, "query benchmark smoke: skipped").map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn export_diagnostics<W: Write>(data_dir: &Path, output: &mut W) -> Result<(), String> {
    let package = collect_diagnostics_package(data_dir)?;

    writeln!(output, "diagnostics redaction: enabled").map_err(|error| error.to_string())?;
    writeln!(output, "diagnostics format: skeleton").map_err(|error| error.to_string())?;
    write_status_counts(output, &package.status)?;
    writeln!(output, "fulltext index: {}", package.fulltext_status)
        .map_err(|error| error.to_string())?;
    writeln!(output, "documents: aggregate-only").map_err(|error| error.to_string())?;
    writeln!(output, "paths: redacted").map_err(|error| error.to_string())?;
    writeln!(output, "file names: excluded").map_err(|error| error.to_string())?;
    writeln!(output, "snippets: excluded").map_err(|error| error.to_string())?;
    writeln!(output, "queries: excluded").map_err(|error| error.to_string())?;
    writeln!(output, "raw text: excluded").map_err(|error| error.to_string())?;
    writeln!(output, "environment: local-only").map_err(|error| error.to_string())?;
    writeln!(output, "remote side effects: none").map_err(|error| error.to_string())?;
    writeln!(output, "{}", render_diagnostic_check(&package.daemon_check))
        .map_err(|error| error.to_string())?;
    writeln!(output, "{}", render_diagnostic_check(&package.disk_check))
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn export_diagnostics_package<W: Write>(
    data_dir: &Path,
    output_dir: &Path,
    output: &mut W,
) -> Result<(), String> {
    let package = collect_diagnostics_package(data_dir)?;
    fs::create_dir_all(output_dir)
        .map_err(|_| "Could not create diagnostics package.".to_string())?;
    let package_dir = diagnostics_package_dir(output_dir)?;
    commit_diagnostics_package(&package_dir, &package)?;

    writeln!(output, "diagnostics package: created").map_err(|error| error.to_string())?;
    writeln!(output, "diagnostics files: 3").map_err(|error| error.to_string())?;
    writeln!(output, "diagnostics redaction: enabled").map_err(|error| error.to_string())?;
    writeln!(output, "remote side effects: none").map_err(|error| error.to_string())
}

fn commit_diagnostics_package(
    package_dir: &Path,
    package: &DiagnosticsPackage,
) -> Result<(), String> {
    commit_diagnostics_package_with(package_dir, |staging_dir| {
        write_complete_diagnostics_package(staging_dir, package)
    })
}

fn commit_diagnostics_package_with(
    package_dir: &Path,
    writer: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let staging_dir = package_dir.with_extension("tmp");
    fs::create_dir(&staging_dir)
        .map_err(|_| "Could not create diagnostics package.".to_string())?;

    let write_result = writer(&staging_dir);
    if write_result.is_ok() {
        if fs::rename(&staging_dir, package_dir).is_err() {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err("Could not create diagnostics package.".to_string());
        }
    } else {
        let _ = fs::remove_dir_all(&staging_dir);
        write_result?;
    }
    Ok(())
}

fn diagnostics_package_dir(output_dir: &Path) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Could not create diagnostics package.".to_string())?
        .as_nanos();
    Ok(output_dir.join(format!(
        "diagnostics-package-{}-{stamp}",
        std::process::id()
    )))
}

fn write_complete_diagnostics_package(
    package_dir: &Path,
    package: &DiagnosticsPackage,
) -> Result<(), String> {
    write_diagnostics_package_file(
        package_dir,
        "manifest.json",
        &render_diagnostics_manifest(package),
    )?;
    write_diagnostics_package_file(
        package_dir,
        "status.txt",
        &render_diagnostics_status(package),
    )?;
    write_diagnostics_package_file(
        package_dir,
        "checks.txt",
        &render_diagnostics_checks(package),
    )
}

fn write_diagnostics_package_file(
    package_dir: &Path,
    file_name: &str,
    contents: &str,
) -> Result<(), String> {
    fs::write(package_dir.join(file_name), contents)
        .map_err(|_| "Could not write diagnostics package.".to_string())
}

struct DiagnosticsPackage {
    status: meta_store::StoreStatus,
    fulltext_status: &'static str,
    daemon_check: DiagnosticCheck,
    disk_check: DiagnosticCheck,
}

fn collect_diagnostics_package(data_dir: &Path) -> Result<DiagnosticsPackage, String> {
    let store = open_store(data_dir)?;
    let status = store
        .status()
        .map_err(|error| error.user_message().to_string())?;
    Ok(DiagnosticsPackage {
        status,
        fulltext_status: inspect_fulltext_index(data_dir, false).status,
        daemon_check: simulate_daemon_kill_diagnostic(0, "", ""),
        disk_check: simulate_disk_full_diagnostic(false, "", ""),
    })
}

fn render_diagnostics_manifest(package: &DiagnosticsPackage) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"diagnostics_schema_version\": 1,\n",
            "  \"schema_version\": {},\n",
            "  \"visible_documents\": {},\n",
            "  \"searchable_documents\": {},\n",
            "  \"ocr_required_documents\": {},\n",
            "  \"index_states\": {},\n",
            "  \"fulltext_index\": \"{}\",\n",
            "  \"redaction_enabled\": true,\n",
            "  \"local_only\": true,\n",
            "  \"remote_side_effects\": \"none\",\n",
            "  \"diagnostic_checks\": [\"{}\", \"{}\"]\n",
            "}}\n"
        ),
        package.status.schema_version,
        package.status.visible_document_count,
        package.status.searchable_document_count,
        package.status.ocr_required_document_count,
        package.status.index_state_count,
        package.fulltext_status,
        package.daemon_check.status,
        package.disk_check.status
    )
}

fn render_diagnostics_status(package: &DiagnosticsPackage) -> String {
    format!(
        concat!(
            "diagnostics redaction: enabled\n",
            "diagnostics format: package\n",
            "metadata schema: {}\n",
            "visible documents: {}\n",
            "index states: {}\n",
            "searchable documents: {}\n",
            "ocr required documents: {}\n",
            "fulltext index: {}\n",
            "documents: aggregate-only\n",
            "paths: redacted\n",
            "file names: excluded\n",
            "snippets: excluded\n",
            "queries: excluded\n",
            "raw text: excluded\n",
            "environment: local-only\n",
            "remote side effects: none\n"
        ),
        package.status.schema_version,
        package.status.visible_document_count,
        package.status.index_state_count,
        package.status.searchable_document_count,
        package.status.ocr_required_document_count,
        package.fulltext_status
    )
}

fn render_diagnostics_checks(package: &DiagnosticsPackage) -> String {
    format!(
        "{}\n{}\n",
        render_diagnostic_check(&package.daemon_check),
        render_diagnostic_check(&package.disk_check)
    )
}

struct BenchmarkSummary {
    synthetic_count: usize,
    indexed_count: usize,
    search_hits: usize,
    post_delete_hits: usize,
    index_elapsed_ms: u128,
    search_elapsed_ms: u128,
    delete_elapsed_ms: u128,
    post_delete_verification: &'static str,
    large_corpus_status: &'static str,
}

fn run_synthetic_benchmark<W: Write>(
    data_dir: &Path,
    synthetic_count: usize,
    query: &str,
    output: &mut W,
) -> Result<(), String> {
    if query.trim().is_empty() {
        return Err("Benchmark query must not be empty.".to_string());
    }
    let summary = with_benchmark_scratch(data_dir, |scratch_dir| {
        run_synthetic_benchmark_in_scratch(scratch_dir, synthetic_count, query)
            .map_err(|_| "Synthetic benchmark failed.".to_string())
    })?;

    write_benchmark_summary(output, &summary)
}

fn with_benchmark_scratch<T>(
    data_dir: &Path,
    runner: impl FnOnce(&Path) -> Result<T, String>,
) -> Result<T, String> {
    fs::create_dir_all(data_dir)
        .map_err(|_| "Could not create benchmark data area.".to_string())?;
    let scratch_dir = benchmark_scratch_dir(data_dir)?;
    fs::create_dir_all(&scratch_dir)
        .map_err(|_| "Could not create benchmark scratch area.".to_string())?;

    let result = runner(&scratch_dir);
    let cleanup_result = fs::remove_dir_all(&scratch_dir);
    match (result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(_)) => Err("Could not clean up benchmark scratch area.".to_string()),
        (Err(error), Ok(())) => Err(error),
        (Err(_), Err(_)) => {
            Err("Synthetic benchmark failed and scratch cleanup failed.".to_string())
        }
    }
}

fn benchmark_scratch_dir(data_dir: &Path) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "Could not create benchmark scratch area.".to_string())?
        .as_nanos();
    Ok(data_dir.join(format!("benchmark-scratch-{}-{stamp}", std::process::id())))
}

fn run_synthetic_benchmark_in_scratch(
    scratch_dir: &Path,
    synthetic_count: usize,
    query: &str,
) -> Result<BenchmarkSummary, String> {
    let store = open_store(scratch_dir)?;
    let index_dir = fulltext_index_dir(scratch_dir);
    let mut writer = FullTextIndexWriter::open_or_create(&index_dir)
        .map_err(|_| "Could not initialize synthetic benchmark index.".to_string())?;
    let index_started = Instant::now();
    let indexed_count =
        seed_synthetic_benchmark_documents(&store, &mut writer, synthetic_count, query.trim())?;
    writer
        .commit()
        .map_err(|_| "Could not commit synthetic benchmark index.".to_string())?;
    drop(writer);
    let index_elapsed_ms = index_started.elapsed().as_millis();

    let search_started = Instant::now();
    let search_hits = benchmark_search(scratch_dir, query.trim(), synthetic_count)?;
    let search_elapsed_ms = search_started.elapsed().as_millis();

    let delete_started = Instant::now();
    let (post_delete_hits, post_delete_verification) =
        if let Some(deleted_doc_id) = search_hits.first().map(|hit| hit.doc_id.clone()) {
            let mut delete_output = Vec::new();
            run_delete(scratch_dir, &deleted_doc_id, &mut delete_output)
                .map_err(|_| "Synthetic benchmark delete failed.".to_string())?;
            let post_delete_hits = benchmark_search(scratch_dir, query.trim(), synthetic_count)?;
            if post_delete_hits
                .iter()
                .any(|hit| hit.doc_id == deleted_doc_id)
            {
                return Err("Synthetic benchmark post-delete verification failed.".to_string());
            }
            (post_delete_hits.len(), "removed")
        } else {
            (0, "no-hit")
        };
    let delete_elapsed_ms = delete_started.elapsed().as_millis();

    Ok(BenchmarkSummary {
        synthetic_count,
        indexed_count,
        search_hits: search_hits.len(),
        post_delete_hits,
        index_elapsed_ms,
        search_elapsed_ms,
        delete_elapsed_ms,
        post_delete_verification,
        large_corpus_status: if synthetic_count >= LARGE_CORPUS_THRESHOLD {
            "synthetic-only"
        } else {
            "not-run"
        },
    })
}

fn seed_synthetic_benchmark_documents(
    store: &MetadataStore,
    writer: &mut FullTextIndexWriter,
    synthetic_count: usize,
    query: &str,
) -> Result<usize, String> {
    let mut indexed_count = 0;
    store
        .begin_bulk_write()
        .map_err(|error| error.user_message().to_string())?;
    let result = (|| {
        for index in 0..synthetic_count {
            let ordinal = index + 1;
            let file_name = format!("synthetic-benchmark-{ordinal:06}.pdf");
            let clean_text = format!(
                "Benchmark synthetic resume {query} Rust Tantivy metadata deletion smoke cohort {}",
                ordinal % 17
            );
            let text_hash = hex_sha256(clean_text.as_bytes());
            let document = Document {
                doc_id: DocumentId::new(),
                source_uri: format!("local://synthetic-benchmark/{file_name}"),
                normalized_path: format!("/synthetic-benchmark/{file_name}"),
                file_name: file_name.clone(),
                extension: DocumentExtension::Pdf,
                byte_size: clean_text.len() as u64,
                mtime: "2026-05-31T00:00:00Z".to_string(),
                content_hash: Some(format!("benchmark-content-{ordinal}")),
                text_hash: Some(text_hash),
                is_deleted: false,
                created_at: "2026-05-31T00:00:00Z".to_string(),
                updated_at: "2026-05-31T00:00:00Z".to_string(),
            };
            let doc_id = document.doc_id.to_string();
            let version_id = format!("benchmark-version-{ordinal}");
            store
                .upsert_document(&document)
                .map_err(|error| error.user_message().to_string())?;
            store
                .upsert_resume_version(ParsedResumeRecord {
                    version_id: &version_id,
                    doc_id: &doc_id,
                    parse_version: "s15-synthetic-benchmark",
                    schema_version: "s15-synthetic-benchmark",
                    raw_text: Some(&clean_text),
                    clean_text: Some(&clean_text),
                    visibility: "SEARCHABLE",
                })
                .map_err(|error| error.user_message().to_string())?;
            writer
                .add_document(IndexDocument {
                    doc_id: doc_id.clone(),
                    version_id: version_id.clone(),
                    file_name,
                    clean_text,
                    section_type: "experience".to_string(),
                    is_deleted: false,
                })
                .map_err(|error| error.to_string())?;
            store
                .upsert_index_state(
                    &format!("fulltext:{doc_id}"),
                    Some(&version_id),
                    "SEARCHABLE",
                    None,
                )
                .map_err(|error| error.user_message().to_string())?;
            indexed_count += 1;
        }
        Ok(indexed_count)
    })();
    match result {
        Ok(indexed_count) => {
            store
                .commit_bulk_write()
                .map_err(|error| error.user_message().to_string())?;
            Ok(indexed_count)
        }
        Err(error) => {
            store.rollback_bulk_write();
            Err(error)
        }
    }
}

fn benchmark_search(
    data_dir: &Path,
    query: &str,
    synthetic_count: usize,
) -> Result<Vec<SearchHit>, String> {
    let top_k = synthetic_count.min(1000);
    let reader = FullTextIndexReader::open_existing(fulltext_index_dir(data_dir))
        .map_err(|_| "Could not open synthetic benchmark index.".to_string())?;
    let hits = reader
        .search(
            query,
            SearchOptions {
                top_k,
                ..SearchOptions::default()
            },
        )
        .map_err(|_| "Synthetic benchmark search failed.".to_string())?;
    filter_hits_by_metadata(hits, data_dir, &FieldFilters::default(), top_k)
}

fn write_benchmark_summary<W: Write>(
    output: &mut W,
    summary: &BenchmarkSummary,
) -> Result<(), String> {
    writeln!(
        output,
        "synthetic document count: {}",
        summary.synthetic_count
    )
    .map_err(|error| error.to_string())?;
    writeln!(output, "indexed count: {}", summary.indexed_count)
        .map_err(|error| error.to_string())?;
    writeln!(output, "search hits: {}", summary.search_hits).map_err(|error| error.to_string())?;
    writeln!(output, "post-delete hits: {}", summary.post_delete_hits)
        .map_err(|error| error.to_string())?;
    writeln!(
        output,
        "post-delete verification: {}",
        summary.post_delete_verification
    )
    .map_err(|error| error.to_string())?;
    writeln!(output, "index elapsed_ms: {}", summary.index_elapsed_ms)
        .map_err(|error| error.to_string())?;
    writeln!(output, "search elapsed_ms: {}", summary.search_elapsed_ms)
        .map_err(|error| error.to_string())?;
    writeln!(output, "delete elapsed_ms: {}", summary.delete_elapsed_ms)
        .map_err(|error| error.to_string())?;
    writeln!(
        output,
        "large-corpus status: {}",
        summary.large_corpus_status
    )
    .map_err(|error| error.to_string())
}

fn write_status_counts<W: Write>(
    output: &mut W,
    status: &meta_store::StoreStatus,
) -> Result<(), String> {
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
    writeln!(
        output,
        "searchable documents: {}",
        status.searchable_document_count
    )
    .map_err(|error| error.to_string())?;
    writeln!(
        output,
        "ocr required documents: {}",
        status.ocr_required_document_count
    )
    .map_err(|error| error.to_string())
}

#[derive(Clone, Copy)]
struct FullTextInspection {
    status: &'static str,
    query_smoke: Option<QuerySmoke>,
}

#[derive(Clone, Copy)]
struct QuerySmoke {
    hits: usize,
    elapsed_ms: u128,
}

fn inspect_fulltext_index(data_dir: &Path, run_query_smoke: bool) -> FullTextInspection {
    let index_dir = fulltext_index_dir(data_dir);
    let reader = match FullTextIndexReader::open_existing(&index_dir) {
        Ok(reader) => reader,
        Err(FullTextError::MissingIndex) => {
            return FullTextInspection {
                status: "missing",
                query_smoke: None,
            };
        }
        Err(_) => {
            return FullTextInspection {
                status: "corrupt-or-unreadable",
                query_smoke: None,
            };
        }
    };

    if !run_query_smoke {
        return FullTextInspection {
            status: "available",
            query_smoke: None,
        };
    }

    let started = Instant::now();
    match reader.search(
        DIAGNOSTIC_QUERY_TEXT,
        SearchOptions {
            top_k: 1,
            ..SearchOptions::default()
        },
    ) {
        Ok(hits) => FullTextInspection {
            status: "available",
            query_smoke: Some(QuerySmoke {
                hits: hits.len(),
                elapsed_ms: started.elapsed().as_millis(),
            }),
        },
        Err(_) => FullTextInspection {
            status: "corrupt-or-unreadable",
            query_smoke: None,
        },
    }
}

#[derive(Clone, Copy)]
struct DiagnosticCheck {
    name: &'static str,
    status: &'static str,
    detail: &'static str,
}

fn simulate_daemon_kill_diagnostic(
    interrupted_jobs: u64,
    _local_path: &str,
    _raw_text: &str,
) -> DiagnosticCheck {
    if interrupted_jobs == 0 {
        DiagnosticCheck {
            name: "daemon kill simulation",
            status: "clean",
            detail: "no interrupted jobs simulated",
        }
    } else {
        DiagnosticCheck {
            name: "daemon kill simulation",
            status: "recoverable",
            detail: "interrupted work remains retryable",
        }
    }
}

fn simulate_disk_full_diagnostic(
    write_rejected: bool,
    _local_path: &str,
    _payload: &str,
) -> DiagnosticCheck {
    if write_rejected {
        DiagnosticCheck {
            name: "disk space simulation",
            status: "write-rejected",
            detail: "write rejected; local path and payload redacted",
        }
    } else {
        DiagnosticCheck {
            name: "disk space simulation",
            status: "not-triggered",
            detail: "no disk exhaustion simulated",
        }
    }
}

fn render_diagnostic_check(check: &DiagnosticCheck) -> String {
    format!("{}: {} ({})", check.name, check.status, check.detail)
}

fn fulltext_index_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("indexes").join("fulltext")
}

#[derive(Clone, Copy)]
enum FullTextDeleteStatus {
    Committed,
    NotPresent,
}

impl FullTextDeleteStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "committed",
            Self::NotPresent => "not-present",
        }
    }
}

fn run_delete<W: Write>(data_dir: &Path, doc_id: &str, output: &mut W) -> Result<(), String> {
    let doc_id = doc_id.trim();
    let index_name = fulltext_index_name(doc_id);
    let store = open_store(data_dir)?;
    if store
        .document_by_doc_id(doc_id)
        .map_err(|error| error.user_message().to_string())?
        .is_none()
    {
        return Err(format!("No document found for doc_id={doc_id}."));
    }

    if !store
        .mark_document_deleted_with_index_state(doc_id, &index_name, None, "DELETE_PENDING", None)
        .map_err(|error| error.user_message().to_string())?
    {
        return Err(format!("No document found for doc_id={doc_id}."));
    }

    let fulltext_status = match delete_from_fulltext_index(data_dir, doc_id) {
        Ok(status) => status,
        Err(error) => {
            store
                .upsert_index_state(
                    &index_name,
                    None,
                    "DELETE_ERROR",
                    Some("fulltext-delete-failed"),
                )
                .map_err(|store_error| store_error.user_message().to_string())?;
            return Err(error);
        }
    };
    store
        .upsert_index_state(&index_name, None, "DELETED", None)
        .map_err(|error| error.user_message().to_string())?;

    writeln!(output, "deleted doc_id={doc_id}").map_err(|error| error.to_string())?;
    writeln!(output, "metadata documents marked deleted: 1").map_err(|error| error.to_string())?;
    writeln!(
        output,
        "fulltext index deletion: {}",
        fulltext_status.as_str()
    )
    .map_err(|error| error.to_string())?;
    writeln!(output, "index state: DELETED").map_err(|error| error.to_string())
}

fn delete_from_fulltext_index(
    data_dir: &Path,
    doc_id: &str,
) -> Result<FullTextDeleteStatus, String> {
    let mut writer = match FullTextIndexWriter::open_existing(fulltext_index_dir(data_dir)) {
        Ok(writer) => writer,
        Err(FullTextError::MissingIndex) => return Ok(FullTextDeleteStatus::NotPresent),
        Err(_) => {
            return Err(format!(
                "Could not update full-text index for doc_id={doc_id}."
            ))
        }
    };
    writer.delete_document(doc_id);
    writer
        .commit()
        .map_err(|_| format!("Could not update full-text index for doc_id={doc_id}."))?;
    Ok(FullTextDeleteStatus::Committed)
}

fn fulltext_index_name(doc_id: &str) -> String {
    format!("fulltext:{doc_id}")
}

fn retrieval_limit(top_k: usize, has_filters: bool) -> usize {
    if has_filters {
        top_k.saturating_mul(5).min(top_k.max(100))
    } else {
        top_k
    }
}

fn filter_hits_by_metadata(
    hits: Vec<SearchHit>,
    data_dir: &Path,
    filters: &FieldFilters,
    top_k: usize,
) -> Result<Vec<SearchHit>, String> {
    let store = open_store(data_dir)?;
    let mut filtered = Vec::new();

    for mut hit in hits {
        let Some(clean_text) = store
            .clean_text_by_doc_id(&hit.doc_id)
            .map_err(|error| error.user_message().to_string())?
        else {
            continue;
        };

        if filters.has_constraints() && !field_summary_from_text(&clean_text).matches(filters) {
            continue;
        }

        hit.rank = filtered.len() + 1;
        filtered.push(hit);

        if filtered.len() >= top_k {
            break;
        }
    }

    Ok(filtered)
}

fn field_summary_from_text(text: &str) -> FieldSummary {
    let evidence = extract_strong_entities(text)
        .into_iter()
        .map(|entity| {
            FieldEvidence::new(
                entity.entity_type(),
                entity.raw_value(),
                entity.normalized_value(),
                entity.confidence(),
            )
        })
        .collect::<Vec<_>>();
    FieldSummary::from_evidence(&evidence)
}

#[derive(Default)]
struct ImportSummary {
    discovered_documents: usize,
    searchable_documents: usize,
    ocr_required_documents: usize,
    skipped_documents: usize,
}

enum ParsedDocument {
    Searchable {
        raw_text: String,
        clean_text: String,
    },
    OcrRequired,
    Skipped,
}

fn run_smoke_import(
    store: &MetadataStore,
    data_dir: &Path,
    root: &Path,
) -> Result<ImportSummary, String> {
    let crawler = Crawler::new();
    let scan = crawler.scan(root);
    let mut summary = ImportSummary {
        skipped_documents: scan.errors.len(),
        ..ImportSummary::default()
    };
    let mut writer = None;

    for file in scan.files {
        summary.discovered_documents += 1;
        match import_one_file(store, data_dir, &file, &mut writer)? {
            ParsedDocument::Searchable { .. } => summary.searchable_documents += 1,
            ParsedDocument::OcrRequired => summary.ocr_required_documents += 1,
            ParsedDocument::Skipped => summary.skipped_documents += 1,
        }
    }

    Ok(summary)
}

fn import_one_file(
    store: &MetadataStore,
    data_dir: &Path,
    file: &DiscoveredFile,
    writer: &mut Option<FullTextIndexWriter>,
) -> Result<ParsedDocument, String> {
    let path = Path::new(file.normalized_path.as_str());
    let bytes =
        fs::read(path).map_err(|_| "Could not read one discovered import file.".to_string())?;
    let content_hash = hex_sha256(&bytes);
    let now = file.fingerprint.mtime_millis.to_string();
    let document = Document {
        doc_id: DocumentId::new(),
        source_uri: file.normalized_path.as_str().to_string(),
        normalized_path: file.normalized_path.as_str().to_string(),
        file_name: file.file_name.clone(),
        extension: document_extension(file.extension),
        byte_size: file.fingerprint.size_bytes,
        mtime: now.clone(),
        content_hash: Some(content_hash),
        text_hash: None,
        is_deleted: false,
        created_at: now.clone(),
        updated_at: now,
    };
    store
        .upsert_document(&document)
        .map_err(|error| error.user_message().to_string())?;
    let stored_document = store
        .document_by_normalized_path(file.normalized_path.as_str())
        .map_err(|error| error.user_message().to_string())?
        .ok_or_else(|| "Imported document metadata was not persisted.".to_string())?;
    if stored_document.is_deleted {
        cleanup_tombstoned_import(store, data_dir, &stored_document.doc_id)?;
        return Ok(ParsedDocument::Skipped);
    }
    let job_id = store
        .insert_ingest_job(
            &stored_document.doc_id,
            "parse_index",
            JobState::Running,
            3,
            1,
        )
        .map_err(|error| error.user_message().to_string())?;

    let parsed = parse_discovered_file(file, bytes)?;
    match &parsed {
        ParsedDocument::Searchable {
            raw_text,
            clean_text,
        } => {
            let text_hash = hex_sha256(clean_text.as_bytes());
            let mut indexed_document = document;
            indexed_document.text_hash = Some(text_hash);
            store
                .upsert_document(&indexed_document)
                .map_err(|error| error.user_message().to_string())?;
            let version_id = version_id_for_document(&stored_document.doc_id);
            store
                .upsert_resume_version(ParsedResumeRecord {
                    version_id: &version_id,
                    doc_id: &stored_document.doc_id,
                    parse_version: "s9-smoke",
                    schema_version: "s9-smoke",
                    raw_text: Some(raw_text),
                    clean_text: Some(clean_text),
                    visibility: "SEARCHABLE",
                })
                .map_err(|error| error.user_message().to_string())?;
            let section_type = first_section_type(clean_text);
            let fulltext_writer = ensure_fulltext_writer(writer, data_dir)?;
            fulltext_writer
                .add_document(IndexDocument {
                    doc_id: stored_document.doc_id.clone(),
                    version_id: version_id.clone(),
                    file_name: stored_document.file_name.clone(),
                    clean_text: clean_text.clone(),
                    section_type,
                    is_deleted: false,
                })
                .map_err(|error| error.to_string())?;
            fulltext_writer
                .commit()
                .map_err(|error| error.to_string())?;
            store
                .upsert_index_state(
                    &format!("fulltext:{}", stored_document.doc_id),
                    Some(&version_id),
                    "SEARCHABLE",
                    None,
                )
                .map_err(|error| error.user_message().to_string())?;
            store
                .update_job_state(job_id, JobState::Completed, 1, None)
                .map_err(|error| error.user_message().to_string())?;
        }
        ParsedDocument::OcrRequired => {
            let version_id = version_id_for_document(&stored_document.doc_id);
            store
                .upsert_resume_version(ParsedResumeRecord {
                    version_id: &version_id,
                    doc_id: &stored_document.doc_id,
                    parse_version: "s9-smoke",
                    schema_version: "s9-smoke",
                    raw_text: None,
                    clean_text: None,
                    visibility: "OCR_REQUIRED",
                })
                .map_err(|error| error.user_message().to_string())?;
            let fulltext_writer = ensure_fulltext_writer(writer, data_dir)?;
            fulltext_writer.delete_document(&stored_document.doc_id);
            fulltext_writer
                .commit()
                .map_err(|error| error.to_string())?;
            store
                .upsert_index_state(
                    &format!("fulltext:{}", stored_document.doc_id),
                    Some(&version_id),
                    "OCR_REQUIRED",
                    None,
                )
                .map_err(|error| error.user_message().to_string())?;
            store
                .update_job_state(job_id, JobState::Completed, 1, None)
                .map_err(|error| error.user_message().to_string())?;
        }
        ParsedDocument::Skipped => {
            let fulltext_writer = ensure_fulltext_writer(writer, data_dir)?;
            fulltext_writer.delete_document(&stored_document.doc_id);
            fulltext_writer
                .commit()
                .map_err(|error| error.to_string())?;
            store
                .upsert_index_state(
                    &format!("fulltext:{}", stored_document.doc_id),
                    None,
                    "SKIPPED",
                    Some("unsupported"),
                )
                .map_err(|error| error.user_message().to_string())?;
            store
                .update_job_state(job_id, JobState::PermanentFailed, 1, Some("unsupported"))
                .map_err(|error| error.user_message().to_string())?;
        }
    }

    Ok(parsed)
}

fn cleanup_tombstoned_import(
    store: &MetadataStore,
    data_dir: &Path,
    doc_id: &str,
) -> Result<(), String> {
    let index_name = fulltext_index_name(doc_id);
    match delete_from_fulltext_index(data_dir, doc_id) {
        Ok(_) => store
            .upsert_index_state(&index_name, None, "DELETED", None)
            .map_err(|error| error.user_message().to_string()),
        Err(error) => {
            store
                .upsert_index_state(
                    &index_name,
                    None,
                    "DELETE_ERROR",
                    Some("fulltext-delete-failed"),
                )
                .map_err(|store_error| store_error.user_message().to_string())?;
            Err(error)
        }
    }
}

fn ensure_fulltext_writer<'a>(
    writer: &'a mut Option<FullTextIndexWriter>,
    data_dir: &Path,
) -> Result<&'a mut FullTextIndexWriter, String> {
    if writer.is_none() {
        *writer = Some(
            FullTextIndexWriter::open_or_create(fulltext_index_dir(data_dir))
                .map_err(|error| error.to_string())?,
        );
    }
    writer
        .as_mut()
        .ok_or_else(|| "Full-text index writer was not initialized.".to_string())
}

fn parse_discovered_file(file: &DiscoveredFile, bytes: Vec<u8>) -> Result<ParsedDocument, String> {
    let input = ParseInput::new(file.file_name.clone(), bytes);
    let output = match file.extension {
        SupportedExtension::Docx => DocxParser
            .parse(&input)
            .map_err(|error| error.user_message().to_string())?,
        SupportedExtension::Pdf => PdfParser
            .parse(&input)
            .map_err(|error| error.user_message().to_string())?,
        _ => return Ok(ParsedDocument::Skipped),
    };

    if output.ocr_required() || output.support_level() == SupportLevel::OcrRequired {
        return Ok(ParsedDocument::OcrRequired);
    }

    let Some(raw_text) = output.text().map(ToOwned::to_owned) else {
        return Ok(ParsedDocument::Skipped);
    };
    let normalized = text_normalizer::normalize_text(&raw_text);
    let clean_text = normalized.text().trim().to_owned();
    if clean_text.is_empty() {
        return Ok(ParsedDocument::Skipped);
    }

    Ok(ParsedDocument::Searchable {
        raw_text,
        clean_text,
    })
}

fn document_extension(extension: SupportedExtension) -> DocumentExtension {
    match extension {
        SupportedExtension::Docx => DocumentExtension::Docx,
        SupportedExtension::Pdf => DocumentExtension::Pdf,
        SupportedExtension::Doc => DocumentExtension::Doc,
        SupportedExtension::Txt => DocumentExtension::Txt,
        SupportedExtension::Image => DocumentExtension::Image,
    }
}

fn first_section_type(text: &str) -> String {
    sectionize(text)
        .first()
        .map(|section| format!("{:?}", section.section_type()).to_ascii_lowercase())
        .unwrap_or_else(|| "other".to_string())
}

fn stable_id(prefix: &str, bytes: &[u8]) -> String {
    let hash = hex_sha256(bytes);
    format!("{prefix}_{}", &hash[..32])
}

fn version_id_for_document(doc_id: &str) -> String {
    stable_id("ver", doc_id.as_bytes())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{commit_diagnostics_package_with, run_with_args, with_benchmark_scratch};
    use index_fulltext::{FullTextIndexWriter, IndexDocument};
    use std::fs;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::FileOptions;

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
    fn doctor_initializes_empty_data_dir_and_skips_missing_index() -> Result<(), String> {
        let data_dir = unique_data_dir("doctor-empty")?;
        let mut output = Vec::new();

        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "doctor"],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("metadata schema: 2"));
        assert!(text.contains("visible documents: 0"));
        assert!(text.contains("fulltext index: missing"));
        assert!(text.contains("query benchmark smoke: skipped"));
        assert!(!text.contains(data_dir.as_str()));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn doctor_runs_small_query_smoke_without_leaking_query_text_or_paths() -> Result<(), String> {
        let data_dir = unique_data_dir("doctor-index")?;
        let index_dir = data_dir.join("indexes").join("fulltext");
        seed_search_document(
            data_dir.as_ref(),
            index_dir.as_ref(),
            "doc-doctor",
            "synthetic-private-doctor.pdf",
            "diagnostic-smoke-token hidden resume raw text",
        )?;
        let mut output = Vec::new();

        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "doctor"],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("fulltext index: available"));
        assert!(text.contains("query benchmark smoke: completed"));
        assert!(text.contains("query benchmark hits: 1"));
        assert!(!text.contains("diagnostic-smoke-token"));
        assert!(!text.contains("hidden resume raw text"));
        assert!(!text.contains("synthetic-private-doctor.pdf"));
        assert!(!text.contains(index_dir.as_str()));
        assert!(!text.contains(data_dir.as_str()));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn doctor_reports_corrupt_fulltext_snapshot_redacted_without_error() -> Result<(), String> {
        let data_dir = unique_data_dir("doctor-corrupt")?;
        let index_dir = data_dir.join("indexes").join("fulltext");
        fs::create_dir_all(index_dir.as_ref()).map_err(|error| error.to_string())?;
        fs::write(
            index_dir.join("meta.json").as_ref(),
            b"not valid tantivy metadata",
        )
        .map_err(|error| error.to_string())?;
        let mut output = Vec::new();

        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "doctor"],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("fulltext index: corrupt-or-unreadable"));
        assert!(text.contains("query benchmark smoke: skipped"));
        assert!(!text.contains(index_dir.as_str()));
        assert!(!text.contains(data_dir.as_str()));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn simulated_fault_diagnostics_are_redacted() -> Result<(), String> {
        let daemon = super::simulate_daemon_kill_diagnostic(
            2,
            "/local/redacted/resume.pdf",
            "raw resume text",
        );
        assert_eq!(daemon.name, "daemon kill simulation");
        assert_eq!(daemon.status, "recoverable");
        let daemon_text = super::render_diagnostic_check(&daemon);
        assert!(daemon_text.contains("daemon kill simulation: recoverable"));
        assert!(!daemon_text.contains("/local/redacted"));
        assert!(!daemon_text.contains("raw resume text"));

        let disk_full = super::simulate_disk_full_diagnostic(
            true,
            "/local/redacted/indexes/fulltext",
            "sensitive-payload-marker",
        );
        assert_eq!(disk_full.name, "disk space simulation");
        assert_eq!(disk_full.status, "write-rejected");
        let disk_text = super::render_diagnostic_check(&disk_full);
        assert!(disk_text.contains("disk space simulation: write-rejected"));
        assert!(!disk_text.contains("/local/redacted"));
        assert!(!disk_text.contains("sensitive-payload-marker"));
        Ok(())
    }

    #[test]
    fn export_diagnostics_requires_redact_and_excludes_local_payloads() -> Result<(), String> {
        let data_dir = unique_data_dir("export-diagnostics")?;
        let synthetic_email = ["private", "@", "invalid.test"].concat();
        let synthetic_phone = ["555", "010", "2121"].join("-");
        let synthetic_raw_text =
            format!("confidential raw resume text {synthetic_email} {synthetic_phone}");
        seed_private_metadata(
            data_dir.as_ref(),
            &synthetic_email,
            &synthetic_phone,
            &synthetic_raw_text,
        )?;

        let mut unredacted_output = Vec::new();
        let error = run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "export-diagnostics",
            ],
            &mut unredacted_output,
        )
        .err()
        .ok_or_else(|| "export-diagnostics without --redact should fail".to_string())?;
        assert!(error.contains("Usage: resume-cli export-diagnostics --redact"));
        assert!(!error.contains(data_dir.as_str()));
        assert!(!error.contains(&synthetic_email));
        assert!(!error.contains(&synthetic_phone));

        let mut output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "export-diagnostics",
                "--redact",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("diagnostics redaction: enabled"));
        assert!(text.contains("metadata schema: 2"));
        assert!(text.contains("visible documents: 1"));
        assert!(text.contains("documents: aggregate-only"));
        assert!(text.contains("paths: redacted"));
        assert!(text.contains("raw text: excluded"));
        assert!(text.contains("remote side effects: none"));
        assert!(!text.contains(data_dir.as_str()));
        assert!(!text.contains(&synthetic_email));
        assert!(!text.contains(&synthetic_phone));
        assert!(!text.contains(&synthetic_raw_text));
        assert!(!text.contains("synthetic-private-export.pdf"));
        assert!(!text.contains("diagnostic-smoke-token"));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn export_diagnostics_package_writes_redacted_aggregate_files() -> Result<(), String> {
        let data_dir = unique_data_dir("export-diagnostics-package")?;
        let output_dir = unique_data_dir("diagnostics-output-private")?;
        let synthetic_email = ["package", "@", "invalid.test"].concat();
        let synthetic_phone = ["555", "011", "3131"].join("-");
        let synthetic_query = "PackagePrivateNeedle";
        let synthetic_doc_id = seed_search_document(
            data_dir.as_ref(),
            data_dir.join("indexes/fulltext").as_ref(),
            "package-private-doc",
            "synthetic-private-package.pdf",
            &format!(
                "confidential package raw resume text {synthetic_email} {synthetic_phone} {synthetic_query}"
            ),
        )?;
        seed_private_metadata(
            data_dir.as_ref(),
            &synthetic_email,
            &synthetic_phone,
            "second confidential package raw resume text",
        )?;
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "export-diagnostics",
                "--redact",
                "--output",
                output_dir.as_str(),
            ],
            &mut output,
        )?;

        let stdout = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(stdout.contains("diagnostics package: created"));
        assert!(stdout.contains("diagnostics files: 3"));
        assert!(stdout.contains("diagnostics redaction: enabled"));
        assert_no_diagnostics_private_payload(
            &stdout,
            data_dir.as_str(),
            output_dir.as_str(),
            &synthetic_email,
            &synthetic_phone,
            synthetic_query,
            &synthetic_doc_id,
        );

        let mut package_dirs = fs::read_dir(output_dir.as_ref())
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        package_dirs.sort_by_key(|entry| entry.file_name());
        assert_eq!(package_dirs.len(), 1);
        let package_dir = package_dirs[0].path();
        assert!(package_dir.is_dir());
        let file_names = fs::read_dir(&package_dir)
            .map_err(|error| error.to_string())?
            .map(|entry| {
                entry
                    .map_err(|error| error.to_string())
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(
            sorted(file_names),
            vec![
                "checks.txt".to_string(),
                "manifest.json".to_string(),
                "status.txt".to_string(),
            ]
        );

        for file_name in ["manifest.json", "status.txt", "checks.txt"] {
            let contents = fs::read_to_string(package_dir.join(file_name))
                .map_err(|error| error.to_string())?;
            assert_no_diagnostics_private_payload(
                &contents,
                data_dir.as_str(),
                output_dir.as_str(),
                &synthetic_email,
                &synthetic_phone,
                synthetic_query,
                &synthetic_doc_id,
            );
            assert!(!contents.contains("synthetic-private-package.pdf"));
            assert!(!contents.contains("synthetic-private-export.pdf"));
            assert!(!contents.contains("confidential package raw resume text"));
            assert!(!contents.contains("second confidential package raw resume text"));
        }

        let manifest = fs::read_to_string(package_dir.join("manifest.json"))
            .map_err(|error| error.to_string())?;
        assert!(manifest.contains("\"schema_version\": 2"));
        assert!(manifest.contains("\"redaction_enabled\": true"));
        assert!(manifest.contains("\"remote_side_effects\": \"none\""));
        assert!(manifest.contains("\"local_only\": true"));
        let status = fs::read_to_string(package_dir.join("status.txt"))
            .map_err(|error| error.to_string())?;
        assert!(status.contains("visible documents: 2"));
        assert!(status.contains("searchable documents: 1"));
        assert!(status.contains("index states: 1"));
        assert!(status.contains("fulltext index: available"));
        let checks = fs::read_to_string(package_dir.join("checks.txt"))
            .map_err(|error| error.to_string())?;
        assert!(checks.contains("daemon kill simulation: clean"));
        assert!(checks.contains("disk space simulation: not-triggered"));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        fs::remove_dir_all(output_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn export_diagnostics_package_can_run_twice_in_same_output_dir() -> Result<(), String> {
        let data_dir = unique_data_dir("export-diagnostics-package-repeat")?;
        let output_dir = unique_data_dir("diagnostics-output-repeat-private")?;

        for _ in 0..2 {
            let mut output = Vec::new();
            run_with_args(
                [
                    "resume-cli",
                    "--data-dir",
                    data_dir.as_str(),
                    "export-diagnostics",
                    "--redact",
                    "--output",
                    output_dir.as_str(),
                ],
                &mut output,
            )?;
            let stdout = String::from_utf8(output).map_err(|error| error.to_string())?;
            assert!(stdout.contains("diagnostics package: created"));
            assert!(!stdout.contains(output_dir.as_str()));
            assert!(!stdout.contains(data_dir.as_str()));
        }

        let package_dirs = fs::read_dir(output_dir.as_ref())
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        assert_eq!(package_dirs.len(), 2);
        for entry in package_dirs {
            let package_dir = entry.path();
            assert!(package_dir.is_dir());
            assert!(package_dir.join("manifest.json").is_file());
            assert!(package_dir.join("status.txt").is_file());
            assert!(package_dir.join("checks.txt").is_file());
        }

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        fs::remove_dir_all(output_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn diagnostics_package_commit_cleans_staging_after_write_failure() -> Result<(), String> {
        let output_dir = unique_data_dir("diagnostics-output-failed-write")?;
        let package_dir = output_dir.as_ref().join("diagnostics-package-test");
        let staging_dir = package_dir.with_extension("tmp");
        let sensitive_payload = "sensitive diagnostic payload";

        let error = commit_diagnostics_package_with(package_dir.as_ref(), |staging_dir| {
            fs::write(staging_dir.join("manifest.json"), sensitive_payload)
                .map_err(|error| error.to_string())?;
            Err::<(), String>("Could not write diagnostics package.".to_string())
        })
        .err()
        .ok_or_else(|| "staged package write should fail".to_string())?;

        assert!(error.contains("Could not write diagnostics package"));
        assert!(!error.contains(sensitive_payload));
        assert!(!error.contains(output_dir.as_str()));
        assert!(!package_dir.exists());
        assert!(!staging_dir.exists());

        fs::remove_dir_all(output_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn export_diagnostics_rejects_invalid_output_args_without_echoing_payloads(
    ) -> Result<(), String> {
        let data_dir = unique_data_dir("export-diagnostics-invalid-output")?;
        let sensitive_output = data_dir.join("sensitive-output-private@example.invalid");
        let mut missing_value_output = Vec::new();

        let missing_value_error = run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "export-diagnostics",
                "--redact",
                "--output",
            ],
            &mut missing_value_output,
        )
        .err()
        .ok_or_else(|| "missing output value should fail".to_string())?;
        assert!(missing_value_error.contains("Usage: resume-cli export-diagnostics --redact"));
        assert!(!missing_value_error.contains(data_dir.as_str()));
        assert!(missing_value_output.is_empty());

        let mut unredacted_output = Vec::new();
        let unredacted_error = run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "export-diagnostics",
                "--output",
                sensitive_output.as_str(),
            ],
            &mut unredacted_output,
        )
        .err()
        .ok_or_else(|| "package export without redaction should fail".to_string())?;
        assert!(unredacted_error.contains("Usage: resume-cli export-diagnostics --redact"));
        assert!(!unredacted_error.contains(data_dir.as_str()));
        assert!(!unredacted_error.contains(sensitive_output.as_str()));
        assert!(!unredacted_error.contains("private@example.invalid"));
        assert!(unredacted_output.is_empty());

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn benchmark_small_synthetic_run_outputs_aggregate_only_and_cleans_scratch(
    ) -> Result<(), String> {
        let data_dir = unique_data_dir("benchmark-small")?;
        let query = "BenchmarkPrivateNeedle";
        let local_payload = "synthetic-benchmark-000001.pdf";
        let scratch_payload = "benchmark-scratch";
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "benchmark",
                "--synthetic-count",
                "3",
                "--query",
                query,
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("synthetic document count: 3"));
        assert!(text.contains("indexed count: 3"));
        assert!(text.contains("search hits: 3"));
        assert!(text.contains("post-delete hits: 2"));
        assert!(text.contains("post-delete verification: removed"));
        assert!(text.contains("index elapsed_ms: "));
        assert!(text.contains("search elapsed_ms: "));
        assert!(text.contains("delete elapsed_ms: "));
        assert!(text.contains("large-corpus status: not-run"));
        assert!(!text.contains(query));
        assert!(!text.contains(data_dir.as_str()));
        assert!(!text.contains(local_payload));
        assert!(!text.contains(scratch_payload));
        assert!(!text.contains("doc_"));
        assert!(!text.contains("Benchmark synthetic resume"));
        let scratch_entries = fs::read_dir(data_dir.as_ref())
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(scratch_payload)
            })
            .count();
        assert_eq!(scratch_entries, 0);
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn benchmark_rejects_invalid_count_without_echoing_query() -> Result<(), String> {
        let data_dir = unique_data_dir("benchmark-invalid")?;
        let query = "SensitiveInvalidBenchmarkQuery";
        let mut output = Vec::new();

        let error = run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "benchmark",
                "--synthetic-count",
                "0",
                "--query",
                query,
            ],
            &mut output,
        )
        .err()
        .ok_or_else(|| "invalid synthetic count should fail".to_string())?;

        assert!(error.contains("Synthetic count must be between 1 and 1000000."));
        assert!(!error.contains(query));
        assert!(!error.contains(data_dir.as_str()));
        assert!(output.is_empty());
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn benchmark_cleans_scratch_after_failed_run_without_echoing_payload() -> Result<(), String> {
        let data_dir = unique_data_dir("benchmark-failure-cleanup")?;
        let sensitive_payload = "SensitiveBenchmarkFailureQuery";

        let error = with_benchmark_scratch(data_dir.as_ref(), |scratch_dir| {
            fs::write(scratch_dir.join("payload.txt"), sensitive_payload)
                .map_err(|error| error.to_string())?;
            Err::<(), String>("Synthetic benchmark failed.".to_string())
        })
        .err()
        .ok_or_else(|| "failed benchmark should return an error".to_string())?;

        assert!(error.contains("Synthetic benchmark failed"));
        assert!(!error.contains(sensitive_payload));
        assert!(!error.contains(data_dir.as_str()));
        let scratch_entries = fs::read_dir(data_dir.as_ref())
            .map_err(|error| error.to_string())?
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("benchmark-scratch")
            })
            .count();
        assert_eq!(scratch_entries, 0);

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
        let doc_id = seed_search_document(
            data_dir.as_ref(),
            index_dir.as_ref(),
            "doc-cli",
            "synthetic-cli.pdf",
            "Synthetic Java 支付 project experience text",
        )?;
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
        assert!(text.contains(&format!("doc_id={doc_id}")));
        assert!(text.contains("file_name=synthetic-cli.pdf"));
        assert!(text.contains("snippet="));
        assert!(!text.contains(index_dir.as_str()));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn search_accepts_degree_filter_and_top_k_after_query() -> Result<(), String> {
        let data_dir = unique_data_dir("search-degree")?;
        let index_dir = data_dir.join("indexes").join("fulltext");
        seed_search_document(
            data_dir.as_ref(),
            index_dir.as_ref(),
            "doc-associate",
            "synthetic-associate.pdf",
            "Synthetic Java engineer Associate Degree Skills Java",
        )?;
        seed_search_document(
            data_dir.as_ref(),
            index_dir.as_ref(),
            "doc-bachelor",
            "synthetic-bachelor.pdf",
            "Synthetic Java engineer Bachelor of Science Skills Java Spring Cloud",
        )?;
        let mut output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "Java",
                "--degree",
                "bachelor",
                "--top-k",
                "20",
            ],
            &mut output,
        )?;

        let text = String::from_utf8(output).map_err(|error| error.to_string())?;
        assert!(text.contains("file_name=synthetic-bachelor.pdf"));
        assert!(!text.contains("synthetic-associate.pdf"));
        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn search_rejects_invalid_numeric_filter_values() -> Result<(), String> {
        for args in [
            [
                "resume-cli",
                "search",
                "Java",
                "--years-experience-min",
                "NaN",
            ],
            [
                "resume-cli",
                "search",
                "Java",
                "--years-experience-min",
                "-1",
            ],
            ["resume-cli", "search", "Java", "--top-k", "0"],
            ["resume-cli", "search", "Java", "--top-k", "10001"],
        ] {
            let mut output = Vec::new();
            let error = run_with_args(args, &mut output)
                .err()
                .ok_or_else(|| "invalid numeric filter should fail".to_string())?;
            assert!(
                error.contains("Invalid years experience filter value")
                    || error.contains("Invalid --top-k value")
            );
        }

        Ok(())
    }

    #[test]
    fn import_indexes_synthetic_docx_and_pdf_then_search_survives_reopen() -> Result<(), String> {
        let data_dir = unique_data_dir("import-search")?;
        let import_root = data_dir.join("root");
        fs::create_dir_all(import_root.as_ref()).map_err(|error| error.to_string())?;
        fs::write(
            import_root.join("synthetic-java.pdf").as_ref(),
            text_layer_pdf_bytes(),
        )
        .map_err(|error| error.to_string())?;
        fs::write(
            import_root.join("synthetic-docx.docx").as_ref(),
            synthetic_docx_bytes()?,
        )
        .map_err(|error| error.to_string())?;
        let mut import_output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut import_output,
        )?;

        let import_text = String::from_utf8(import_output).map_err(|error| error.to_string())?;
        assert!(import_text.contains("queued import task: 1"));
        assert!(import_text.contains("searchable documents: 2"));
        assert!(!import_text.contains(import_root.as_str()));

        let mut status_output = Vec::new();
        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "status"],
            &mut status_output,
        )?;
        let status_text = String::from_utf8(status_output).map_err(|error| error.to_string())?;
        assert!(status_text.contains("queued imports: 0"));
        assert!(status_text.contains("searchable documents: 2"));

        let mut search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "Java",
            ],
            &mut search_output,
        )?;
        let search_text = String::from_utf8(search_output).map_err(|error| error.to_string())?;
        assert!(search_text.contains("rank=1"));
        assert!(search_text.contains("file_name=synthetic-java.pdf"));
        assert!(search_text.contains("Java"));

        let mut reopened_search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "docx",
            ],
            &mut reopened_search_output,
        )?;
        let reopened_text =
            String::from_utf8(reopened_search_output).map_err(|error| error.to_string())?;
        assert!(reopened_text.contains("file_name=synthetic-docx.docx"));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn import_routes_image_only_pdf_to_ocr_required_without_indexing_fake_text(
    ) -> Result<(), String> {
        let data_dir = unique_data_dir("import-ocr")?;
        let import_root = data_dir.join("root");
        fs::create_dir_all(import_root.as_ref()).map_err(|error| error.to_string())?;
        fs::write(
            import_root.join("synthetic-scan.pdf").as_ref(),
            image_only_pdf_bytes(),
        )
        .map_err(|error| error.to_string())?;
        let mut import_output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut import_output,
        )?;

        let import_text = String::from_utf8(import_output).map_err(|error| error.to_string())?;
        assert!(import_text.contains("ocr required documents: 1"));

        let mut status_output = Vec::new();
        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "status"],
            &mut status_output,
        )?;
        let status_text = String::from_utf8(status_output).map_err(|error| error.to_string())?;
        assert!(status_text.contains("ocr required documents: 1"));

        let mut search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "synthetic",
            ],
            &mut search_output,
        )?;
        let search_text = String::from_utf8(search_output).map_err(|error| error.to_string())?;
        assert!(!search_text.contains("synthetic-scan.pdf"));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn import_keeps_same_text_documents_as_distinct_search_results() -> Result<(), String> {
        let data_dir = unique_data_dir("import-same-text")?;
        let import_root = data_dir.join("root");
        fs::create_dir_all(import_root.as_ref()).map_err(|error| error.to_string())?;
        fs::write(
            import_root.join("synthetic-copy-a.pdf").as_ref(),
            text_layer_pdf_bytes_with("Synthetic Java duplicate resume text"),
        )
        .map_err(|error| error.to_string())?;
        fs::write(
            import_root.join("synthetic-copy-b.pdf").as_ref(),
            text_layer_pdf_bytes_with("Synthetic Java duplicate resume text"),
        )
        .map_err(|error| error.to_string())?;
        let mut import_output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut import_output,
        )?;

        let import_text = String::from_utf8(import_output).map_err(|error| error.to_string())?;
        assert!(import_text.contains("searchable documents: 2"));

        let mut search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "Java",
            ],
            &mut search_output,
        )?;
        let search_text = String::from_utf8(search_output).map_err(|error| error.to_string())?;
        assert!(search_text.contains("file_name=synthetic-copy-a.pdf"));
        assert!(search_text.contains("file_name=synthetic-copy-b.pdf"));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn reimporting_same_path_as_ocr_required_removes_old_search_hit() -> Result<(), String> {
        let data_dir = unique_data_dir("import-ocr-replaces-text")?;
        let import_root = data_dir.join("root");
        fs::create_dir_all(import_root.as_ref()).map_err(|error| error.to_string())?;
        let resume_path = import_root.join("synthetic-changing.pdf");
        fs::write(
            resume_path.as_ref(),
            text_layer_pdf_bytes_with("Synthetic Java text before scan replacement"),
        )
        .map_err(|error| error.to_string())?;
        let mut first_import_output = Vec::new();

        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut first_import_output,
        )?;

        let mut first_search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "Java",
            ],
            &mut first_search_output,
        )?;
        let first_search =
            String::from_utf8(first_search_output).map_err(|error| error.to_string())?;
        assert!(first_search.contains("file_name=synthetic-changing.pdf"));

        fs::write(resume_path.as_ref(), image_only_pdf_bytes())
            .map_err(|error| error.to_string())?;
        let mut second_import_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut second_import_output,
        )?;

        let mut status_output = Vec::new();
        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "status"],
            &mut status_output,
        )?;
        let status_text = String::from_utf8(status_output).map_err(|error| error.to_string())?;
        assert!(status_text.contains("searchable documents: 0"));
        assert!(status_text.contains("ocr required documents: 1"));

        let mut second_search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "Java",
            ],
            &mut second_search_output,
        )?;
        let second_search =
            String::from_utf8(second_search_output).map_err(|error| error.to_string())?;
        assert!(!second_search.contains("synthetic-changing.pdf"));
        assert!(!second_search.contains("before scan replacement"));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn delete_by_doc_id_hides_imported_document_and_keeps_source_file() -> Result<(), String> {
        let data_dir = unique_data_dir("delete-imported")?;
        let import_root = data_dir.join("root");
        fs::create_dir_all(import_root.as_ref()).map_err(|error| error.to_string())?;
        let resume_path = import_root.join("synthetic-delete.pdf");
        let synthetic_email = ["delete", "@", "invalid.test"].concat();
        let synthetic_phone = ["555", "020", "3030"].join("-");
        let raw_text =
            format!("DeletionToken Java propagation text {synthetic_email} {synthetic_phone}");
        fs::write(resume_path.as_ref(), text_layer_pdf_bytes_with(&raw_text))
            .map_err(|error| error.to_string())?;

        let mut import_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut import_output,
        )?;

        let mut search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "DeletionToken",
            ],
            &mut search_output,
        )?;
        let search_text = String::from_utf8(search_output).map_err(|error| error.to_string())?;
        assert!(search_text.contains("file_name=synthetic-delete.pdf"));
        let doc_id = doc_id_from_search_output(&search_text)?;

        let mut delete_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "delete",
                "--doc-id",
                doc_id.as_str(),
            ],
            &mut delete_output,
        )?;
        let delete_text = String::from_utf8(delete_output).map_err(|error| error.to_string())?;

        assert!(delete_text.contains(&format!("doc_id={doc_id}")));
        assert!(delete_text.contains("index state: DELETED"));
        assert!(resume_path.is_file());
        assert!(!delete_text.contains(data_dir.as_str()));
        assert!(!delete_text.contains(import_root.as_str()));
        assert!(!delete_text.contains(resume_path.as_str()));
        assert!(!delete_text.contains("synthetic-delete.pdf"));
        assert!(!delete_text.contains("DeletionToken"));
        assert!(!delete_text.contains(&synthetic_email));
        assert!(!delete_text.contains(&synthetic_phone));
        assert!(!delete_text.contains(&raw_text));

        let mut status_output = Vec::new();
        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "status"],
            &mut status_output,
        )?;
        let status_text = String::from_utf8(status_output).map_err(|error| error.to_string())?;
        assert!(status_text.contains("visible documents: 0"));
        assert!(status_text.contains("searchable documents: 0"));

        let mut reopened_search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "DeletionToken",
            ],
            &mut reopened_search_output,
        )?;
        let reopened_search =
            String::from_utf8(reopened_search_output).map_err(|error| error.to_string())?;
        assert!(reopened_search.contains("no search results"));
        assert!(!reopened_search.contains("synthetic-delete.pdf"));
        assert!(!reopened_search.contains("DeletionToken"));
        assert!(!reopened_search.contains(&synthetic_email));
        assert!(!reopened_search.contains(&synthetic_phone));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn reimport_after_delete_keeps_tombstoned_document_hidden() -> Result<(), String> {
        let data_dir = unique_data_dir("delete-reimport")?;
        let import_root = data_dir.join("root");
        fs::create_dir_all(import_root.as_ref()).map_err(|error| error.to_string())?;
        let resume_path = import_root.join("synthetic-delete-reimport.pdf");
        fs::write(
            resume_path.as_ref(),
            text_layer_pdf_bytes_with("ReimportDeleteToken Java text before tombstone"),
        )
        .map_err(|error| error.to_string())?;

        let mut first_import_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut first_import_output,
        )?;
        let mut first_search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "ReimportDeleteToken",
            ],
            &mut first_search_output,
        )?;
        let first_search =
            String::from_utf8(first_search_output).map_err(|error| error.to_string())?;
        let doc_id = doc_id_from_search_output(&first_search)?;

        let mut delete_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "delete",
                "--doc-id",
                doc_id.as_str(),
            ],
            &mut delete_output,
        )?;
        assert!(resume_path.is_file());

        fs::write(
            resume_path.as_ref(),
            text_layer_pdf_bytes_with("ReimportDeleteToken Java text after tombstone"),
        )
        .map_err(|error| error.to_string())?;
        let mut second_import_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut second_import_output,
        )?;
        let second_import =
            String::from_utf8(second_import_output).map_err(|error| error.to_string())?;
        assert!(second_import.contains("skipped documents: 1"));
        assert!(!second_import.contains(resume_path.as_str()));
        assert!(!second_import.contains("synthetic-delete-reimport.pdf"));

        let mut second_search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "ReimportDeleteToken",
            ],
            &mut second_search_output,
        )?;
        let second_search =
            String::from_utf8(second_search_output).map_err(|error| error.to_string())?;
        assert!(second_search.contains("no search results"));
        assert!(!second_search.contains("synthetic-delete-reimport.pdf"));
        assert!(!second_search.contains("after tombstone"));
        assert!(resume_path.is_file());

        let mut status_output = Vec::new();
        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "status"],
            &mut status_output,
        )?;
        let status_text = String::from_utf8(status_output).map_err(|error| error.to_string())?;
        assert!(status_text.contains("visible documents: 0"));
        assert!(status_text.contains("searchable documents: 0"));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn delete_by_doc_id_without_fulltext_index_does_not_create_index() -> Result<(), String> {
        let data_dir = unique_data_dir("delete-no-index")?;
        let source_path = data_dir.join("source").join("synthetic-metadata-only.pdf");
        fs::create_dir_all(data_dir.join("source").as_ref()).map_err(|error| error.to_string())?;
        fs::write(source_path.as_ref(), b"synthetic source bytes")
            .map_err(|error| error.to_string())?;
        let doc_id = seed_metadata_only_document(
            data_dir.as_ref(),
            source_path.as_ref(),
            "synthetic-metadata-only.pdf",
            "MetadataOnlyToken Java text",
        )?;
        let index_dir = data_dir.join("indexes").join("fulltext");

        let mut delete_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "delete",
                "--doc-id",
                doc_id.as_str(),
            ],
            &mut delete_output,
        )?;
        let delete_text = String::from_utf8(delete_output).map_err(|error| error.to_string())?;

        assert!(delete_text.contains("fulltext index deletion: not-present"));
        assert!(delete_text.contains("index state: DELETED"));
        assert!(source_path.is_file());
        assert!(!index_dir.as_ref().exists());

        let mut status_output = Vec::new();
        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "status"],
            &mut status_output,
        )?;
        let status_text = String::from_utf8(status_output).map_err(|error| error.to_string())?;
        assert!(status_text.contains("visible documents: 0"));
        assert!(status_text.contains("searchable documents: 0"));
        assert!(status_text.contains("index states: 1"));
        assert!(!delete_text.contains(source_path.as_str()));
        assert!(!delete_text.contains("synthetic-metadata-only.pdf"));
        assert!(!delete_text.contains("MetadataOnlyToken"));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn delete_with_corrupt_fulltext_leaves_tombstone_and_error_state() -> Result<(), String> {
        let data_dir = unique_data_dir("delete-corrupt-index")?;
        let source_path = data_dir.join("source").join("synthetic-corrupt-index.pdf");
        fs::create_dir_all(data_dir.join("source").as_ref()).map_err(|error| error.to_string())?;
        fs::write(source_path.as_ref(), b"synthetic source bytes")
            .map_err(|error| error.to_string())?;
        let doc_id = seed_metadata_only_document(
            data_dir.as_ref(),
            source_path.as_ref(),
            "synthetic-corrupt-index.pdf",
            "CorruptDeleteToken Java text",
        )?;
        let index_dir = data_dir.join("indexes").join("fulltext");
        fs::create_dir_all(index_dir.as_ref()).map_err(|error| error.to_string())?;
        fs::write(
            index_dir.join("meta.json").as_ref(),
            b"not tantivy metadata",
        )
        .map_err(|error| error.to_string())?;

        let mut output = Vec::new();
        let error = run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "delete",
                "--doc-id",
                doc_id.as_str(),
            ],
            &mut output,
        )
        .err()
        .ok_or_else(|| "corrupt index delete should fail".to_string())?;

        assert!(error.contains(&doc_id));
        assert!(error.contains("Could not update full-text index"));
        assert!(!error.contains(data_dir.as_str()));
        assert!(!error.contains(source_path.as_str()));
        assert!(!error.contains("synthetic-corrupt-index.pdf"));
        assert!(output.is_empty());
        assert!(source_path.is_file());

        let store = meta_store::MetadataStore::open(data_dir.join("metadata.sqlite").as_ref())
            .map_err(|error| error.user_message().to_string())?;
        let stored = store
            .document_by_doc_id(&doc_id)
            .map_err(|error| error.user_message().to_string())?
            .ok_or_else(|| "deleted document metadata missing".to_string())?;
        assert!(stored.is_deleted);
        assert_eq!(
            store
                .index_state_status(&format!("fulltext:{doc_id}"))
                .map_err(|error| error.user_message().to_string())?,
            Some("DELETE_ERROR".to_string())
        );

        let mut status_output = Vec::new();
        run_with_args(
            ["resume-cli", "--data-dir", data_dir.as_str(), "status"],
            &mut status_output,
        )?;
        let status_text = String::from_utf8(status_output).map_err(|error| error.to_string())?;
        assert!(status_text.contains("visible documents: 0"));
        assert!(status_text.contains("searchable documents: 0"));

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn search_hides_stale_fulltext_hit_after_metadata_delete_error() -> Result<(), String> {
        let data_dir = unique_data_dir("delete-error-search-visibility")?;
        let import_root = data_dir.join("root");
        fs::create_dir_all(import_root.as_ref()).map_err(|error| error.to_string())?;
        let resume_path = import_root.join("synthetic-stale-delete-hit.pdf");
        fs::write(
            resume_path.as_ref(),
            text_layer_pdf_bytes_with("StaleDeleteToken Java text must stay hidden"),
        )
        .map_err(|error| error.to_string())?;

        let mut import_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "import",
                "--root",
                import_root.as_str(),
            ],
            &mut import_output,
        )?;

        let mut first_search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "StaleDeleteToken",
            ],
            &mut first_search_output,
        )?;
        let first_search =
            String::from_utf8(first_search_output).map_err(|error| error.to_string())?;
        let doc_id = doc_id_from_search_output(&first_search)?;

        let store = meta_store::MetadataStore::open(data_dir.join("metadata.sqlite").as_ref())
            .map_err(|error| error.user_message().to_string())?;
        store
            .mark_document_deleted_with_index_state(
                &doc_id,
                &format!("fulltext:{doc_id}"),
                None,
                "DELETE_ERROR",
                Some("fulltext-delete-failed"),
            )
            .map_err(|error| error.user_message().to_string())?;

        let mut second_search_output = Vec::new();
        run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "search",
                "StaleDeleteToken",
            ],
            &mut second_search_output,
        )?;
        let second_search =
            String::from_utf8(second_search_output).map_err(|error| error.to_string())?;
        assert!(second_search.contains("no search results"));
        assert!(!second_search.contains(&doc_id));
        assert!(!second_search.contains("synthetic-stale-delete-hit.pdf"));
        assert!(!second_search.contains("StaleDeleteToken"));
        assert!(resume_path.is_file());

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn delete_unknown_doc_id_fails_without_local_payloads() -> Result<(), String> {
        let data_dir = unique_data_dir("delete-unknown")?;
        let unknown_doc_id = "doc_unknown_delete";
        let mut output = Vec::new();

        let error = run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "delete",
                "--doc-id",
                unknown_doc_id,
            ],
            &mut output,
        )
        .err()
        .ok_or_else(|| "unknown doc_id delete should fail".to_string())?;

        assert!(error.contains(unknown_doc_id));
        assert!(error.contains("No document found"));
        assert!(!error.contains(data_dir.as_str()));
        assert!(!error.contains("synthetic"));
        assert!(output.is_empty());

        fs::remove_dir_all(data_dir).map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn delete_rejects_malformed_doc_id_without_echoing_value() -> Result<(), String> {
        let data_dir = unique_data_dir("delete-malformed-doc-id")?;
        let malformed_doc_id = "/local/redacted/resume.pdf\nInjectedToken";
        let mut output = Vec::new();

        let error = run_with_args(
            [
                "resume-cli",
                "--data-dir",
                data_dir.as_str(),
                "delete",
                "--doc-id",
                malformed_doc_id,
            ],
            &mut output,
        )
        .err()
        .ok_or_else(|| "malformed doc_id delete should fail".to_string())?;

        assert!(error.contains("Invalid doc_id value"));
        assert!(!error.contains(malformed_doc_id));
        assert!(!error.contains("/local/redacted"));
        assert!(!error.contains("resume.pdf"));
        assert!(!error.contains("InjectedToken"));
        assert!(output.is_empty());

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

    fn seed_search_document(
        data_dir: &std::path::Path,
        index_dir: &std::path::Path,
        doc_id: &str,
        file_name: &str,
        clean_text: &str,
    ) -> Result<String, String> {
        use core_domain::{Document, DocumentExtension, DocumentId};
        use meta_store::{MetadataStore, ParsedResumeRecord};

        let store = MetadataStore::open(data_dir.join("metadata.sqlite"))
            .map_err(|error| error.user_message().to_string())?;
        store
            .run_migrations()
            .map_err(|error| error.user_message().to_string())?;
        let document = Document {
            doc_id: DocumentId::new(),
            source_uri: format!("local://synthetic/{file_name}"),
            normalized_path: format!("/synthetic/{file_name}"),
            file_name: file_name.to_string(),
            extension: DocumentExtension::Pdf,
            byte_size: 128,
            mtime: "2026-01-01T00:00:00Z".to_string(),
            content_hash: Some(format!("{doc_id}-content")),
            text_hash: Some(format!("{doc_id}-text")),
            is_deleted: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let stored_doc_id = document.doc_id.to_string();
        store
            .upsert_document(&document)
            .map_err(|error| error.user_message().to_string())?;
        store
            .upsert_resume_version(ParsedResumeRecord {
                version_id: &format!("ver-{doc_id}"),
                doc_id: &stored_doc_id,
                parse_version: "test",
                schema_version: "test",
                raw_text: Some(clean_text),
                clean_text: Some(clean_text),
                visibility: "SEARCHABLE",
            })
            .map_err(|error| error.user_message().to_string())?;

        let mut writer = FullTextIndexWriter::open_or_create(index_dir)
            .map_err(|error| format!("could not create synthetic full-text test index: {error}"))?;
        writer
            .add_document(IndexDocument {
                doc_id: stored_doc_id.clone(),
                version_id: format!("ver-{doc_id}"),
                file_name: file_name.to_string(),
                clean_text: clean_text.to_string(),
                section_type: "experience".to_string(),
                is_deleted: false,
            })
            .map_err(|error| error.to_string())?;
        writer.commit().map_err(|error| error.to_string())?;
        Ok(stored_doc_id)
    }

    fn seed_metadata_only_document(
        data_dir: &std::path::Path,
        source_path: &std::path::Path,
        file_name: &str,
        clean_text: &str,
    ) -> Result<String, String> {
        use core_domain::{Document, DocumentExtension, DocumentId};
        use meta_store::{MetadataStore, ParsedResumeRecord};

        let store = MetadataStore::open(data_dir.join("metadata.sqlite"))
            .map_err(|error| error.user_message().to_string())?;
        store
            .run_migrations()
            .map_err(|error| error.user_message().to_string())?;
        let source_path_text = source_path.to_string_lossy().to_string();
        let document = Document {
            doc_id: DocumentId::new(),
            source_uri: format!("file://{source_path_text}"),
            normalized_path: source_path_text,
            file_name: file_name.to_string(),
            extension: DocumentExtension::Pdf,
            byte_size: 128,
            mtime: "2026-01-01T00:00:00Z".to_string(),
            content_hash: Some("metadata-only-content".to_string()),
            text_hash: Some("metadata-only-text".to_string()),
            is_deleted: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let stored_doc_id = document.doc_id.to_string();
        store
            .upsert_document(&document)
            .map_err(|error| error.user_message().to_string())?;
        store
            .upsert_resume_version(ParsedResumeRecord {
                version_id: "metadata-only-version",
                doc_id: &stored_doc_id,
                parse_version: "test",
                schema_version: "test",
                raw_text: Some(clean_text),
                clean_text: Some(clean_text),
                visibility: "SEARCHABLE",
            })
            .map_err(|error| error.user_message().to_string())?;
        store
            .upsert_index_state(
                &format!("fulltext:{stored_doc_id}"),
                Some("metadata-only-version"),
                "SEARCHABLE",
                None,
            )
            .map_err(|error| error.user_message().to_string())?;
        Ok(stored_doc_id)
    }

    fn doc_id_from_search_output(text: &str) -> Result<String, String> {
        text.split_whitespace()
            .find_map(|part| part.strip_prefix("doc_id="))
            .map(ToString::to_string)
            .ok_or_else(|| "search output did not include a doc_id".to_string())
    }

    fn seed_private_metadata(
        data_dir: &std::path::Path,
        synthetic_email: &str,
        synthetic_phone: &str,
        synthetic_raw_text: &str,
    ) -> Result<(), String> {
        use core_domain::{Document, DocumentExtension, DocumentId};
        use meta_store::{MetadataStore, ParsedResumeRecord};

        let store = MetadataStore::open(data_dir.join("metadata.sqlite"))
            .map_err(|error| error.user_message().to_string())?;
        store
            .run_migrations()
            .map_err(|error| error.user_message().to_string())?;
        let private_path = data_dir.join("synthetic-private-export.pdf");
        let private_path_text = private_path.to_string_lossy().to_string();
        let document = Document {
            doc_id: DocumentId::new(),
            source_uri: format!("file://{private_path_text}"),
            normalized_path: private_path_text,
            file_name: "synthetic-private-export.pdf".to_string(),
            extension: DocumentExtension::Pdf,
            byte_size: 256,
            mtime: "2026-01-01T00:00:00Z".to_string(),
            content_hash: Some(format!("{synthetic_email}-{synthetic_phone}")),
            text_hash: Some("private-text-hash".to_string()),
            is_deleted: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let stored_doc_id = document.doc_id.to_string();
        store
            .upsert_document(&document)
            .map_err(|error| error.user_message().to_string())?;
        store
            .upsert_resume_version(ParsedResumeRecord {
                version_id: "private-export-version",
                doc_id: &stored_doc_id,
                parse_version: "test-private",
                schema_version: "test-private",
                raw_text: Some(synthetic_raw_text),
                clean_text: Some(synthetic_raw_text),
                visibility: "SEARCHABLE",
            })
            .map_err(|error| error.user_message().to_string())?;
        store
            .upsert_index_state(
                "fulltext:private-export",
                Some("private-export-version"),
                "SEARCHABLE",
                Some("private path and payload must stay redacted"),
            )
            .map_err(|error| error.user_message().to_string())
    }

    fn sorted(mut values: Vec<String>) -> Vec<String> {
        values.sort();
        values
    }

    fn assert_no_diagnostics_private_payload(
        text: &str,
        data_dir: &str,
        output_dir: &str,
        synthetic_email: &str,
        synthetic_phone: &str,
        synthetic_query: &str,
        synthetic_doc_id: &str,
    ) {
        assert!(!text.contains(data_dir));
        assert!(!text.contains(output_dir));
        assert!(!text.contains(synthetic_email));
        assert!(!text.contains(synthetic_phone));
        assert!(!text.contains(synthetic_query));
        assert!(!text.contains(synthetic_doc_id));
        assert!(!text.contains("doc_id"));
        assert!(!text.contains("file_name"));
    }

    fn text_layer_pdf_bytes() -> Vec<u8> {
        text_layer_pdf_bytes_with("Synthetic Java engineer with PDF text layer")
    }

    fn text_layer_pdf_bytes_with(text: &str) -> Vec<u8> {
        format!(
            "%PDF-1.4
1 0 obj
<< /Type /Page /Contents 2 0 R /Resources << /Font << /F1 3 0 R >> >> >>
endobj
2 0 obj
<< /Length 90 >>
stream
BT
/F1 12 Tf
72 720 Td
({text}) Tj
ET
endstream
endobj
3 0 obj
<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>
endobj
%%EOF"
        )
        .into_bytes()
    }

    fn image_only_pdf_bytes() -> Vec<u8> {
        b"%PDF-1.4
1 0 obj
<< /Type /Page /Resources << /XObject << /Im1 2 0 R >> >> /Contents 3 0 R >>
endobj
2 0 obj
<< /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 4 >>
stream
0000
endstream
endobj
3 0 obj
<< /Length 24 >>
stream
q 10 0 0 10 0 0 cm /Im1 Do Q
endstream
endobj
%%EOF"
            .to_vec()
    }

    fn synthetic_docx_bytes() -> Result<Vec<u8>, String> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = FileOptions::default();

        writer
            .start_file("[Content_Types].xml", options)
            .map_err(|error| error.to_string())?;
        writer
            .write_all(br#"<?xml version="1.0" encoding="UTF-8"?><Types/>"#)
            .map_err(|error| error.to_string())?;
        writer
            .start_file("word/document.xml", options)
            .map_err(|error| error.to_string())?;
        writer
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Synthetic docx Java project</w:t></w:r></w:p>
  </w:body>
</w:document>"#,
            )
            .map_err(|error| error.to_string())?;

        writer
            .finish()
            .map(|cursor| cursor.into_inner())
            .map_err(|error| error.to_string())
    }
}
