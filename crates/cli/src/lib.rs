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
                top_k: retrieval_limit(top_k, filters.has_constraints()),
                ..SearchOptions::default()
            };
            let hits = reader
                .search(trimmed, search_options)
                .map_err(|error| error.to_string())?;
            let hits = if filters.has_constraints() {
                filter_hits_by_fields(hits, &options.data_dir, &filters, top_k)?
            } else {
                hits.into_iter().take(top_k).collect()
            };
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
    Import {
        root: PathBuf,
    },
    Search {
        query: String,
        filters: FieldFilters,
        top_k: usize,
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
        return Err("Usage: resume-cli [--data-dir <path>] <status|import|search>".to_string());
    };
    match command.as_str() {
        "status" if parts.len() == 1 => Ok(Command::Status),
        "import" => parse_import_command(parts),
        "search" if parts.len() >= 2 => parse_search_command(parts),
        "search" => Err("Usage: resume-cli search <query>".to_string()),
        _ => Err("Unknown command. Use status, import, or search.".to_string()),
    }
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

fn retrieval_limit(top_k: usize, has_filters: bool) -> usize {
    if has_filters {
        top_k.saturating_mul(5).min(top_k.max(100))
    } else {
        top_k
    }
}

fn filter_hits_by_fields(
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

        if field_summary_from_text(&clean_text).matches(filters) {
            hit.rank = filtered.len() + 1;
            filtered.push(hit);
        }

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
    use super::run_with_args;
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
    ) -> Result<(), String> {
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
                doc_id: stored_doc_id,
                version_id: format!("ver-{doc_id}"),
                file_name: file_name.to_string(),
                clean_text: clean_text.to_string(),
                section_type: "experience".to_string(),
                is_deleted: false,
            })
            .map_err(|error| error.to_string())?;
        writer.commit().map_err(|error| error.to_string())
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
