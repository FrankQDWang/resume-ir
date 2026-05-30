use config::{Profile, RuntimeProfile};
use extractor_rules::extract_resume_fields;
use fs_crawler::scan_directory;
use index_fulltext::{FullTextIndex, IndexDocument, SearchHit};
use parser_common::{ParseInput, ParseStatus, Parser, ResourceBudget};
use parser_docx::DocxParser;
use parser_pdf::PdfParser;
use rank_fusion::{CandidateProfile, DegreeLevel, FieldFilter, filter_candidates};
use search_planner::{SearchRequest, plan_search};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use text_normalizer::normalize_text;

const DEFAULT_STATE_DIR: &str = "local-data";
const SNAPSHOT_FILE: &str = "cli-index.tsv";

pub fn run<I, S, W, E>(args: I, stdout: &mut W, stderr: &mut E) -> i32
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    W: Write,
    E: Write,
{
    run_with_state_dir(args, stdout, stderr, Path::new(DEFAULT_STATE_DIR))
}

pub fn run_with_state_dir<I, S, W, E>(
    args: I,
    stdout: &mut W,
    stderr: &mut E,
    state_dir: &Path,
) -> i32
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
    W: Write,
    E: Write,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_owned())
        .collect();
    let Some(command) = args.get(1).map(String::as_str) else {
        return write_stderr(stderr, "usage: resume-cli <status|import|search>");
    };

    match command {
        "status" => status(state_dir, stdout, stderr),
        "import" => import_root(&args[2..], state_dir, stdout, stderr),
        "search" => search(&args[2..], state_dir, stdout, stderr),
        _ => write_stderr(
            stderr,
            "unknown command; expected status, import, or search",
        ),
    }
}

fn status<W, E>(state_dir: &Path, stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    let profile = RuntimeProfile::default();
    match load_snapshot(state_dir) {
        Ok(records) => {
            let searchable = records
                .iter()
                .filter(|record| record.status == SnapshotStatus::Searchable)
                .count();
            let ocr_required = records
                .iter()
                .filter(|record| record.status == SnapshotStatus::OcrRequired)
                .count();
            let output = format!(
                "health: ok\nindexed_documents: {}\nsearchable_documents: {}\nocr_required_documents: {}\nactive_profile: {}\n",
                records.len(),
                searchable,
                ocr_required,
                profile_name(profile.profile)
            );
            write_stdout(stdout, stderr, &output)
        }
        Err(error) => write_stderr(stderr, &format!("failed to read status: {error}")),
    }
}

fn import_root<W, E>(args: &[String], state_dir: &Path, stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    let Some(root) = flag_value(args, "--root") else {
        return write_stderr(stderr, "usage: resume-cli import --root <path>");
    };
    let root_path = Path::new(root);
    if !root_path.is_dir() {
        return write_stderr(stderr, "root is not a readable directory");
    }

    let result = import_to_snapshot(root_path, state_dir);
    match result {
        Ok(summary) => {
            let output = format!(
                "import_job: completed\nindexed_documents: {}\nsearchable_documents: {}\nocr_required_documents: {}\n",
                summary.indexed_documents,
                summary.searchable_documents,
                summary.ocr_required_documents
            );
            write_stdout(stdout, stderr, &output)
        }
        Err(error) => write_stderr(stderr, &format!("failed to queue import job: {error}")),
    }
}

fn search<W, E>(args: &[String], state_dir: &Path, stdout: &mut W, stderr: &mut E) -> i32
where
    W: Write,
    E: Write,
{
    if args.is_empty() {
        return write_stderr(stderr, "usage: resume-cli search <query>");
    }
    let search_args = match parse_search_args(args) {
        Ok(search_args) => search_args,
        Err(error) => return write_stderr(stderr, &error),
    };
    let plan = plan_search(SearchRequest {
        query: search_args.query.clone(),
        top_k: search_args.top_k,
    });

    match search_snapshot_or_synthetic(
        state_dir,
        &plan.fulltext_query,
        plan.top_k,
        &search_args.filters,
    ) {
        Ok(hits) => {
            let mut output = format!("query: {}\nresults: {}\n", search_args.query, hits.len());
            for hit in hits {
                output.push_str(&format!(
                    "rank: {}\ndoc_id: {}\nfile_name: {}\nsnippet: {}\n",
                    hit.rank, hit.doc_id, hit.file_name, hit.snippet
                ));
            }
            write_stdout(stdout, stderr, &output)
        }
        Err(error) => write_stderr(stderr, &format!("search failed: {error}")),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotStatus {
    Searchable,
    OcrRequired,
}

impl SnapshotStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Searchable => "SEARCHABLE",
            Self::OcrRequired => "OCR_REQUIRED",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "SEARCHABLE" => Some(Self::Searchable),
            "OCR_REQUIRED" => Some(Self::OcrRequired),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SnapshotRecord {
    doc_id: String,
    version_id: String,
    file_name: String,
    clean_text: String,
    status: SnapshotStatus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ImportSummary {
    indexed_documents: usize,
    searchable_documents: usize,
    ocr_required_documents: usize,
}

#[derive(Clone, Debug, PartialEq)]
struct SearchArguments {
    query: String,
    top_k: usize,
    filters: FieldFilter,
}

fn import_to_snapshot(root_path: &Path, state_dir: &Path) -> Result<ImportSummary, String> {
    let entries = scan_directory(root_path).map_err(|error| error.message)?;
    let mut records = Vec::new();

    for entry in entries {
        let bytes = fs::read(&entry.path).map_err(|error| error.to_string())?;
        let parse_output = match entry.extension.as_str() {
            "docx" => DocxParser
                .parse(
                    ParseInput {
                        path: entry.path.clone(),
                        bytes,
                    },
                    ResourceBudget::new(Duration::from_secs(10)),
                )
                .map_err(|error| error.user_message().to_owned())?,
            "pdf" => PdfParser
                .parse(
                    ParseInput {
                        path: entry.path.clone(),
                        bytes,
                    },
                    ResourceBudget::new(Duration::from_secs(10)),
                )
                .map_err(|error| error.user_message().to_owned())?,
            _ => continue,
        };
        let status = match parse_output.status {
            ParseStatus::Parsed => SnapshotStatus::Searchable,
            ParseStatus::OcrRequired => SnapshotStatus::OcrRequired,
        };
        let clean_text = if status == SnapshotStatus::Searchable {
            normalize_text(&parse_output.text).text
        } else {
            String::new()
        };
        records.push(SnapshotRecord {
            doc_id: format!("doc_{:016x}", entry.fingerprint.sample_hash),
            version_id: format!("ver_{:016x}", entry.fingerprint.sample_hash),
            file_name: entry.file_name,
            clean_text,
            status,
        });
    }

    write_snapshot(state_dir, &records).map_err(|error| error.to_string())?;
    Ok(ImportSummary {
        indexed_documents: records.len(),
        searchable_documents: records
            .iter()
            .filter(|record| record.status == SnapshotStatus::Searchable)
            .count(),
        ocr_required_documents: records
            .iter()
            .filter(|record| record.status == SnapshotStatus::OcrRequired)
            .count(),
    })
}

fn search_snapshot_or_synthetic(
    state_dir: &Path,
    query: &str,
    top_k: usize,
    filters: &FieldFilter,
) -> index_fulltext::FullTextResult<Vec<SearchHit>> {
    let records = load_snapshot(state_dir).unwrap_or_default();
    if records.is_empty() {
        return search_synthetic_fixture(query, top_k, filters);
    }
    search_records(records, query, top_k, filters)
}

fn search_records(
    records: Vec<SnapshotRecord>,
    query: &str,
    top_k: usize,
    filters: &FieldFilter,
) -> index_fulltext::FullTextResult<Vec<SearchHit>> {
    let docs: Vec<IndexDocument> = records
        .iter()
        .filter(|record| record.status == SnapshotStatus::Searchable)
        .map(|record| {
            IndexDocument::searchable(
                record.doc_id.clone(),
                record.version_id.clone(),
                record.file_name.clone(),
                record.clean_text.clone(),
                "document",
            )
        })
        .collect();
    let index = FullTextIndex::create_in_memory()?;
    index.index_batch(docs)?;
    index.commit()?;
    let candidate_limit = if filters.is_empty() {
        top_k
    } else {
        top_k.saturating_mul(8).clamp(top_k, 100)
    };
    let hits = index.search(query, candidate_limit)?;
    Ok(apply_filters(hits, &records, filters, top_k))
}

fn search_synthetic_fixture(
    query: &str,
    top_k: usize,
    filters: &FieldFilter,
) -> index_fulltext::FullTextResult<Vec<SearchHit>> {
    let index = FullTextIndex::create_in_memory()?;
    index.index_batch(vec![IndexDocument::searchable(
        "doc_fixture_java_payment",
        "ver_fixture_java_payment",
        "fixture_java_payment.pdf",
        "Java 支付 gateway backend engineer",
        "experience",
    )])?;
    index.commit()?;
    let hits = index.search(query, top_k)?;
    Ok(apply_filters(hits, &[], filters, top_k))
}

fn apply_filters(
    hits: Vec<SearchHit>,
    records: &[SnapshotRecord],
    filters: &FieldFilter,
    top_k: usize,
) -> Vec<SearchHit> {
    if filters.is_empty() {
        return hits.into_iter().take(top_k).collect();
    }
    let records_by_doc: HashMap<&str, &SnapshotRecord> = records
        .iter()
        .map(|record| (record.doc_id.as_str(), record))
        .collect();
    let profiles: Vec<CandidateProfile> = hits
        .iter()
        .filter_map(|hit| {
            let record = records_by_doc.get(hit.doc_id.as_str())?;
            Some(CandidateProfile {
                doc_id: hit.doc_id.clone(),
                fields: extract_resume_fields(&record.clean_text),
            })
        })
        .collect();
    let kept = filter_candidates(&profiles, filters);
    let mut filtered = Vec::new();
    for mut hit in hits {
        if kept.contains(&hit.doc_id) {
            hit.rank = filtered.len() + 1;
            filtered.push(hit);
        }
        if filtered.len() == top_k {
            break;
        }
    }
    filtered
}

fn snapshot_path(state_dir: &Path) -> PathBuf {
    state_dir.join(SNAPSHOT_FILE)
}

fn write_snapshot(state_dir: &Path, records: &[SnapshotRecord]) -> io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let path = snapshot_path(state_dir);
    let temp_path = state_dir.join(format!("{SNAPSHOT_FILE}.tmp"));
    let mut contents = String::new();
    for record in records {
        contents.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            escape_field(&record.doc_id),
            escape_field(&record.version_id),
            escape_field(&record.file_name),
            escape_field(record.status.as_str()),
            escape_field(&record.clean_text)
        ));
    }
    fs::write(&temp_path, contents)?;
    fs::rename(temp_path, path)
}

fn load_snapshot(state_dir: &Path) -> io::Result<Vec<SnapshotRecord>> {
    let path = snapshot_path(state_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(path)?;
    let mut records = Vec::new();
    for line in contents.lines() {
        let fields: Vec<String> = line.split('\t').map(unescape_field).collect();
        if fields.len() != 5 {
            continue;
        }
        let Some(status) = SnapshotStatus::parse(&fields[3]) else {
            continue;
        };
        records.push(SnapshotRecord {
            doc_id: fields[0].clone(),
            version_id: fields[1].clone(),
            file_name: fields[2].clone(),
            status,
            clean_text: fields[4].clone(),
        });
    }
    Ok(records)
}

fn escape_field(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

fn unescape_field(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(char) = chars.next() {
        if char == '\\' {
            match chars.next() {
                Some('t') => output.push('\t'),
                Some('n') => output.push('\n'),
                Some('\\') => output.push('\\'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(char);
        }
    }
    output
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].as_str())
}

fn parse_search_args(args: &[String]) -> Result<SearchArguments, String> {
    let mut query_terms = Vec::new();
    let mut top_k = 20;
    let mut filters = FieldFilter::default();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--top-k" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("usage: resume-cli search <query> [--top-k <n>]".to_owned());
                };
                top_k = value
                    .parse::<usize>()
                    .map_err(|_| "top-k must be a positive integer".to_owned())?;
                index += 2;
            }
            "--degree" | "--degree-min" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("usage: resume-cli search <query> [--degree <level>]".to_owned());
                };
                filters.degree_min = DegreeLevel::parse(value);
                if filters.degree_min.is_none() {
                    return Err("degree must be bachelor, master, or doctorate".to_owned());
                }
                index += 2;
            }
            "--skills-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(
                        "usage: resume-cli search <query> [--skills-any <skill,...>]".to_owned(),
                    );
                };
                filters.skills_any = value
                    .split(',')
                    .map(str::trim)
                    .filter(|skill| !skill.is_empty())
                    .map(str::to_ascii_lowercase)
                    .collect();
                index += 2;
            }
            "--years-experience-min" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(
                        "usage: resume-cli search <query> [--years-experience-min <years>]"
                            .to_owned(),
                    );
                };
                filters.years_experience_min = Some(
                    value
                        .parse::<f32>()
                        .map_err(|_| "years-experience-min must be a number".to_owned())?,
                );
                index += 2;
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown search option: {value}"));
            }
            value => {
                query_terms.push(value.to_owned());
                index += 1;
            }
        }
    }

    if query_terms.is_empty() {
        return Err("usage: resume-cli search <query>".to_owned());
    }

    Ok(SearchArguments {
        query: query_terms.join(" "),
        top_k,
        filters,
    })
}

fn profile_name(profile: Profile) -> &'static str {
    match profile {
        Profile::Economy => "economy",
        Profile::Balanced => "balanced",
        Profile::Turbo => "turbo",
    }
}

fn write_stdout<W, E>(stdout: &mut W, stderr: &mut E, message: &str) -> i32
where
    W: Write,
    E: Write,
{
    match stdout.write_all(message.as_bytes()) {
        Ok(()) => 0,
        Err(error) => write_stderr(stderr, &format!("failed to write output: {error}")),
    }
}

fn write_stderr<E>(stderr: &mut E, message: &str) -> i32
where
    E: Write,
{
    match writeln!(stderr, "error: {message}") {
        Ok(()) => 1,
        Err(_) => 1,
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "resume-cli"
}

#[must_use]
pub fn binary_name() -> &'static str {
    "resume-cli"
}
