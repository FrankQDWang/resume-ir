//! Shared local import worker for the synthetic resume import pipeline.

use core_domain::{Document, DocumentExtension, DocumentId};
use fs_crawler::{Crawler, DiscoveredFile, SupportedExtension};
use index_fulltext::{FullTextError, FullTextIndexWriter, IndexDocument};
use meta_store::{JobState, MetadataStore, ParsedResumeRecord};
use parser_common::{ParseInput, Parser, SupportLevel};
use parser_docx::DocxParser;
use parser_pdf::PdfParser;
use sectionizer::sectionize;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

/// Aggregate result for one import-root drain.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ImportSummary {
    /// Number of supported documents discovered under the root.
    pub discovered_documents: usize,
    /// Number of documents parsed and indexed as searchable.
    pub searchable_documents: usize,
    /// Number of documents routed to OCR-required metadata state.
    pub ocr_required_documents: usize,
    /// Number of documents skipped due to crawl, parsing, or tombstone state.
    pub skipped_documents: usize,
}

enum ParsedDocument {
    Searchable {
        raw_text: String,
        clean_text: String,
    },
    OcrRequired,
    Skipped,
}

/// Runs the local synthetic import pipeline for one root directory.
pub fn run_import_root(
    store: &MetadataStore,
    data_dir: &Path,
    root: &Path,
) -> Result<ImportSummary, String> {
    if !root.is_dir() {
        return Err("Import root must be an existing directory.".to_string());
    }

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

fn fulltext_index_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("indexes").join("fulltext")
}

#[derive(Clone, Copy)]
enum FullTextDeleteStatus {
    Committed,
    NotPresent,
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

/// Returns the crate name for smoke tests and workspace metadata.
#[must_use]
pub fn crate_name() -> &'static str {
    "import-worker"
}
