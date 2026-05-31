use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use core_domain::SectionType;
use fs_crawler::{crawl_directory, DiscoveredFile};
use index_fulltext::{FullTextIndex, IndexDocument, IndexSection};
use meta_store::{
    Document, DocumentStatus, FileExtension, ImportTask, ImportTaskStatus, IndexState,
    IndexStateStatus, MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
};
use parser_common::{ParseInput, ParseStatus, Parser, ParserErrorKind, ResourceBudget};
use parser_docx::DocxParser;
use parser_pdf::PdfParser;
use sectionizer::{SectionChunk, Sectionizer};
use text_normalizer::TextNormalizer;

const PARSE_VERSION: &str = "parser-v1";
const SCHEMA_VERSION: &str = "resume-ir-s9-v1";
const INDEX_MANIFEST_VERSION: &str = "fulltext-s9-v1";

pub fn crate_name() -> &'static str {
    "import-pipeline"
}

pub type Result<T> = std::result::Result<T, ImportPipelineError>;

pub fn import_root(
    data_dir: &Path,
    store: &MetaStore,
    task: &ImportTask,
    root: &Path,
    now: UnixTimestamp,
) -> Result<ImportSummary> {
    store
        .update_import_task_status(&task.id, ImportTaskStatus::Running, now)
        .map_err(ImportPipelineError::store)?;

    let result = run_import(data_dir, store, root, now);
    match result {
        Ok(summary) => {
            store
                .update_import_task_status(&task.id, ImportTaskStatus::Completed, now)
                .map_err(ImportPipelineError::store)?;
            Ok(summary)
        }
        Err(error) => {
            let _ = store.update_import_task_status(
                &task.id,
                if error.retryable {
                    ImportTaskStatus::FailedRetryable
                } else {
                    ImportTaskStatus::FailedPermanent
                },
                now,
            );
            Err(error)
        }
    }
}

fn run_import(
    data_dir: &Path,
    store: &MetaStore,
    root: &Path,
    now: UnixTimestamp,
) -> Result<ImportSummary> {
    let report = crawl_directory(root).map_err(ImportPipelineError::crawl)?;
    let mut summary = ImportSummary {
        files_discovered: report.files.len(),
        scan_errors: report.errors.len(),
        ignored_entries: report.ignored_count,
        searchable_documents: 0,
        ocr_required_documents: 0,
        failed_documents: 0,
        deleted_documents: 0,
    };
    let mut pending_index_documents = Vec::new();
    let sectionizer = Sectionizer::default();
    let can_propagate_deletions = report.errors.is_empty();
    let discovered_doc_ids = report
        .files
        .iter()
        .map(|file| file.document_id.as_str().to_string())
        .collect::<BTreeSet<_>>();

    for file in report.files {
        match process_file(store, &file, &sectionizer, now)? {
            ProcessedFile::Searchable {
                document,
                index_document,
            } => {
                pending_index_documents.push((*document, *index_document));
            }
            ProcessedFile::OcrRequired => {
                summary.ocr_required_documents += 1;
            }
            ProcessedFile::Failed => {
                summary.failed_documents += 1;
            }
        }
    }

    if can_propagate_deletions {
        summary.deleted_documents =
            mark_missing_documents_deleted(store, root, &discovered_doc_ids, now)?;
    }
    let pending_doc_ids = pending_index_documents
        .iter()
        .map(|(document, _)| document.id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let mut index_documents = persisted_index_documents(store, &sectionizer, &pending_doc_ids)?;
    index_documents.extend(
        pending_index_documents
            .iter()
            .map(|(_, index_document)| index_document.clone()),
    );
    let indexed_document_count = index_documents.len();
    write_full_text_index(data_dir, index_documents)?;

    for (mut document, _) in pending_index_documents {
        document.status = DocumentStatus::Searchable;
        document.updated_at = now;
        store
            .upsert_document(&document)
            .map_err(ImportPipelineError::store)?;
        summary.searchable_documents += 1;
    }

    update_index_state(
        store,
        now,
        indexed_document_count,
        summary.ocr_required_documents,
        summary.deleted_documents,
    )?;

    Ok(summary)
}

pub fn rebuild_full_text_index(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
) -> Result<IndexRebuildSummary> {
    let sectionizer = Sectionizer::default();
    let index_documents = persisted_index_documents(store, &sectionizer, &BTreeSet::new())?;
    let indexed_documents = index_documents.len();
    write_full_text_index(data_dir, index_documents)?;
    update_index_state(store, now, indexed_documents, 0, 0)?;

    Ok(IndexRebuildSummary { indexed_documents })
}

fn write_full_text_index(data_dir: &Path, index_documents: Vec<IndexDocument>) -> Result<()> {
    let index = FullTextIndex::open_or_create(&data_dir.join("search-index"))
        .map_err(ImportPipelineError::index)?;
    index
        .replace_documents(index_documents)
        .map_err(ImportPipelineError::index)?;
    index.commit().map_err(ImportPipelineError::index)?;
    drop(index);

    Ok(())
}

fn update_index_state(
    store: &MetaStore,
    now: UnixTimestamp,
    indexed_documents: usize,
    ocr_required_documents: usize,
    deleted_documents: usize,
) -> Result<()> {
    store
        .upsert_index_state(&IndexState {
            manifest_version: INDEX_MANIFEST_VERSION.to_string(),
            snapshot_token: Some(format!(
                "snapshot:{}:{}:{}:{}",
                now.as_unix_seconds(),
                indexed_documents,
                ocr_required_documents,
                deleted_documents
            )),
            status: IndexStateStatus::Ready,
            updated_at: now,
        })
        .map_err(ImportPipelineError::store)
}

fn mark_missing_documents_deleted(
    store: &MetaStore,
    root: &Path,
    discovered_doc_ids: &BTreeSet<String>,
    now: UnixTimestamp,
) -> Result<usize> {
    let documents = store
        .visible_documents()
        .map_err(ImportPipelineError::store)?;
    let mut deleted_count = 0_usize;

    for document in documents {
        if !document_path_is_under_root(&document.normalized_path, root) {
            continue;
        }
        if discovered_doc_ids.contains(document.id.as_str()) {
            continue;
        }
        if store
            .mark_document_deleted(&document.id, now)
            .map_err(ImportPipelineError::store)?
            .is_some()
        {
            deleted_count += 1;
        }
    }

    Ok(deleted_count)
}

fn document_path_is_under_root(document_path: &str, root: &Path) -> bool {
    Path::new(document_path).starts_with(root)
}

fn persisted_index_documents(
    store: &MetaStore,
    sectionizer: &Sectionizer,
    pending_doc_ids: &BTreeSet<String>,
) -> Result<Vec<IndexDocument>> {
    let documents = store
        .visible_documents()
        .map_err(ImportPipelineError::store)?;
    let mut index_documents = Vec::new();

    for document in documents {
        if pending_doc_ids.contains(document.id.as_str())
            || !matches!(
                document.status,
                DocumentStatus::Searchable | DocumentStatus::IndexedPartial
            )
        {
            continue;
        }

        let versions = store
            .resume_versions_for_document(&document.id)
            .map_err(ImportPipelineError::store)?;
        if let Some(index_document) = versions
            .iter()
            .find_map(|version| index_document_from_resume_version(&document, version, sectionizer))
        {
            index_documents.push(index_document);
        }
    }

    Ok(index_documents)
}

fn index_document_from_resume_version(
    document: &Document,
    version: &ResumeVersion,
    sectionizer: &Sectionizer,
) -> Option<IndexDocument> {
    if version.visibility == ResumeVisibility::Hidden {
        return None;
    }

    let clean_text = version.clean_text.as_ref()?;
    if clean_text.trim().is_empty() {
        return None;
    }

    Some(IndexDocument {
        doc_id: document.id.to_string(),
        version_id: version.id.to_string(),
        file_name: document.file_name.clone(),
        clean_text: clean_text.clone(),
        sections: sections_to_index(sectionizer.sectionize(clean_text)),
        is_deleted: document.is_deleted,
    })
}

fn process_file(
    store: &MetaStore,
    file: &DiscoveredFile,
    sectionizer: &Sectionizer,
    now: UnixTimestamp,
) -> Result<ProcessedFile> {
    let mut document = document_from_discovered_file(file, now, DocumentStatus::Discovered);
    store
        .upsert_document(&document)
        .map_err(ImportPipelineError::store)?;

    let path = PathBuf::from(file.normalized_path.as_str());
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            document.status = DocumentStatus::FailedRetryable;
            store
                .upsert_document(&document)
                .map_err(ImportPipelineError::store)?;
            return Ok(ProcessedFile::Failed);
        }
    };

    let extension = file_extension_label(&file.extension);
    let parse_output = match file.extension {
        FileExtension::Docx => DocxParser
            .parse(
                ParseInput::from_bytes(Some(extension), &bytes),
                ResourceBudget::default(),
            )
            .map_err(|error| (error, document.clone())),
        FileExtension::Pdf => PdfParser
            .parse(
                ParseInput::from_bytes(Some(extension), &bytes),
                ResourceBudget::default(),
            )
            .map_err(|error| (error, document.clone())),
        _ => {
            document.status = DocumentStatus::FailedPermanent;
            store
                .upsert_document(&document)
                .map_err(ImportPipelineError::store)?;
            return Ok(ProcessedFile::Failed);
        }
    };

    let parse_output = match parse_output {
        Ok(parse_output) => parse_output,
        Err((error, mut document)) => {
            document.status = if error.retryable() {
                DocumentStatus::FailedRetryable
            } else if error.kind() == ParserErrorKind::OcrRequired {
                DocumentStatus::OcrRequired
            } else {
                DocumentStatus::FailedPermanent
            };
            store
                .upsert_document(&document)
                .map_err(ImportPipelineError::store)?;
            return Ok(if document.status == DocumentStatus::OcrRequired {
                ProcessedFile::OcrRequired
            } else {
                ProcessedFile::Failed
            });
        }
    };

    if parse_output.status() == ParseStatus::OcrRequired {
        document.status = DocumentStatus::OcrRequired;
        store
            .upsert_document(&document)
            .map_err(ImportPipelineError::store)?;
        return Ok(ProcessedFile::OcrRequired);
    }

    let clean_text = TextNormalizer::normalize(parse_output.text())
        .text()
        .to_string();
    if clean_text.trim().is_empty() {
        document.status = DocumentStatus::OcrRequired;
        store
            .upsert_document(&document)
            .map_err(ImportPipelineError::store)?;
        return Ok(ProcessedFile::OcrRequired);
    }

    document.status = DocumentStatus::TextCleaned;
    let version_id = ResumeVersionId::from_non_secret_parts(&[
        "s9",
        document.id.as_str(),
        PARSE_VERSION,
        SCHEMA_VERSION,
    ]);
    store
        .upsert_document(&document)
        .map_err(ImportPipelineError::store)?;
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id.clone(),
            document_id: document.id.clone(),
            candidate_id: None,
            parse_version: PARSE_VERSION.to_string(),
            schema_version: SCHEMA_VERSION.to_string(),
            language_set: language_set(&clean_text),
            page_count: parse_output
                .page_count()
                .and_then(|page_count| u32::try_from(page_count).ok()),
            raw_text: Some(parse_output.text().to_string()),
            clean_text: Some(clean_text.clone()),
            quality_score: Some(0.8),
            visibility: ResumeVisibility::Searchable,
        })
        .map_err(ImportPipelineError::store)?;

    let sections = sectionizer.sectionize(&clean_text);
    Ok(ProcessedFile::Searchable {
        document: Box::new(document.clone()),
        index_document: Box::new(IndexDocument {
            doc_id: document.id.to_string(),
            version_id: version_id.to_string(),
            file_name: file.file_name.clone(),
            clean_text,
            sections: sections_to_index(sections),
            is_deleted: false,
        }),
    })
}

fn document_from_discovered_file(
    file: &DiscoveredFile,
    now: UnixTimestamp,
    status: DocumentStatus,
) -> Document {
    Document {
        id: file.document_id.clone(),
        source_uri: format!("file://{}", file.normalized_path.as_str()),
        normalized_path: file.normalized_path.as_str().to_string(),
        file_name: file.file_name.clone(),
        extension: file.extension.clone(),
        byte_size: file.byte_size,
        mtime: file.mtime,
        content_hash: Some(file.fingerprint.as_str().to_string()),
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status,
    }
}

fn sections_to_index(sections: Vec<SectionChunk>) -> Vec<IndexSection> {
    sections
        .into_iter()
        .map(|section| IndexSection {
            section_type: section_type_label(&section.section_type).to_string(),
            text: section.text,
        })
        .collect()
}

fn section_type_label(section_type: &SectionType) -> &str {
    match section_type {
        SectionType::Profile => "profile",
        SectionType::Contact => "contact",
        SectionType::Education => "education",
        SectionType::Experience => "experience",
        SectionType::Project => "project",
        SectionType::Skill => "skill",
        SectionType::Certificate => "certificate",
        SectionType::Other(_) => "other",
    }
}

fn file_extension_label(extension: &FileExtension) -> &'static str {
    match extension {
        FileExtension::Docx => "docx",
        FileExtension::Pdf => "pdf",
        FileExtension::Doc => "doc",
        FileExtension::Txt => "txt",
        FileExtension::Image => "image",
        FileExtension::Other(_) => "other",
    }
}

fn language_set(text: &str) -> Vec<String> {
    let mut languages = Vec::new();
    if text
        .chars()
        .any(|character| character.is_ascii_alphabetic())
    {
        languages.push("en".to_string());
    }
    if text.chars().any(|character| {
        ('\u{4e00}'..='\u{9fff}').contains(&character)
            || ('\u{3400}'..='\u{4dbf}').contains(&character)
    }) {
        languages.push("zh".to_string());
    }

    if languages.is_empty() {
        languages.push("unknown".to_string());
    }
    languages
}

enum ProcessedFile {
    Searchable {
        document: Box<Document>,
        index_document: Box<IndexDocument>,
    },
    OcrRequired,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportSummary {
    pub files_discovered: usize,
    pub scan_errors: usize,
    pub ignored_entries: usize,
    pub searchable_documents: usize,
    pub ocr_required_documents: usize,
    pub failed_documents: usize,
    pub deleted_documents: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexRebuildSummary {
    pub indexed_documents: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportPipelineError {
    kind: ImportPipelineErrorKind,
    retryable: bool,
}

impl ImportPipelineError {
    fn store(_error: meta_store::MetaStoreError) -> Self {
        Self {
            kind: ImportPipelineErrorKind::Store,
            retryable: true,
        }
    }

    fn crawl(_error: fs_crawler::CrawlError) -> Self {
        Self {
            kind: ImportPipelineErrorKind::Crawl,
            retryable: true,
        }
    }

    fn index(_error: index_fulltext::FullTextError) -> Self {
        Self {
            kind: ImportPipelineErrorKind::Index,
            retryable: true,
        }
    }
}

impl fmt::Debug for ImportPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportPipelineError")
            .field("kind", &self.kind)
            .field("retryable", &self.retryable)
            .finish()
    }
}

impl fmt::Display for ImportPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ImportPipelineErrorKind::Store => formatter.write_str("metadata update failed"),
            ImportPipelineErrorKind::Crawl => formatter.write_str("file scan failed"),
            ImportPipelineErrorKind::Index => formatter.write_str("search index update failed"),
        }
    }
}

impl std::error::Error for ImportPipelineError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportPipelineErrorKind {
    Store,
    Crawl,
    Index,
}
