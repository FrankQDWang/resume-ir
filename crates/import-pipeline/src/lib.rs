use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use core_domain::{EntityMentionId, SectionType};
use extractor_rules::{extract_strong_fields, FieldType, RuleMatch};
pub use fs_crawler::ScanProfile;
use fs_crawler::{
    crawl_directory_with_options, CrawlError, CrawlErrorKind, DiscoveredFile, FsOperation,
    NormalizedPath, ScanBudgetKind, ScanOptions,
};
use index_fulltext::{publish_snapshot, IndexDocument, IndexSection};
use meta_store::{
    Document, DocumentStatus, EntityMention, EntityType, FileExtension, ImportScanError,
    ImportScanErrorKind, ImportScanErrorOperation, ImportTask, ImportTaskId, ImportTaskStatus,
    IndexState, IndexStateStatus, MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility,
    UnixTimestamp,
};
use parser_common::{ParseInput, ParseStatus, Parser, ParserErrorKind, ResourceBudget};
use parser_docx::DocxParser;
use parser_pdf::PdfParser;
use privacy::{ContactHasher, ContactKind};
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
    import_root_with_options(data_dir, store, task, root, now, ImportOptions::default())
}

pub fn import_root_with_options(
    data_dir: &Path,
    store: &MetaStore,
    task: &ImportTask,
    root: &Path,
    now: UnixTimestamp,
    options: ImportOptions,
) -> Result<ImportSummary> {
    store
        .update_import_task_status(&task.id, ImportTaskStatus::Running, now)
        .map_err(ImportPipelineError::store)?;

    let result = run_import(data_dir, store, task, root, now, options);
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
    task: &ImportTask,
    root: &Path,
    now: UnixTimestamp,
    options: ImportOptions,
) -> Result<ImportSummary> {
    let report = crawl_directory_with_options(
        root,
        ScanOptions {
            profile: options.scan_profile,
            max_files: options.max_files,
        },
    )
    .map_err(ImportPipelineError::crawl)?;
    let scanned_directories = report.scanned_directories.clone();
    let skipped_directories = report.skipped_directories.clone();
    let scan_errors = import_scan_errors_from_crawl(&task.id, &report.errors, now);
    let scan_budget_exhausted = report.budget_exhausted;
    let mut summary = ImportSummary {
        files_discovered: report.files.len(),
        scan_errors: report.errors.len(),
        ignored_entries: report.ignored_count,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget: scan_budget_exhausted.map(ImportScanBudget::from),
    };
    store
        .replace_import_scan_errors(&task.id, &scan_errors)
        .map_err(ImportPipelineError::store)?;
    let mut pending_index_documents = Vec::new();
    let sectionizer = Sectionizer::default();
    let can_propagate_deletions = report.errors.is_empty() && scan_budget_exhausted.is_none();
    let discovered_doc_ids = report
        .files
        .iter()
        .map(|file| file.document_id.as_str().to_string())
        .collect::<BTreeSet<_>>();

    for file in report.files {
        match process_file(data_dir, store, &file, &sectionizer, now)? {
            ProcessedFile::Searchable {
                document,
                index_document,
            } => {
                pending_index_documents.push((*document, *index_document));
            }
            ProcessedFile::OcrRequired { ocr_job_queued } => {
                summary.ocr_required_documents += 1;
                if ocr_job_queued {
                    summary.ocr_jobs_queued += 1;
                }
            }
            ProcessedFile::Failed => {
                summary.failed_documents += 1;
            }
        }
    }

    if can_propagate_deletions {
        summary.deleted_documents = mark_missing_documents_deleted(
            store,
            root,
            options.scan_profile,
            &scanned_directories,
            &skipped_directories,
            &discovered_doc_ids,
            now,
        )?;
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
    let snapshot_token = index_snapshot_token(
        now,
        indexed_document_count,
        summary.ocr_required_documents,
        summary.deleted_documents,
    );
    write_full_text_index(data_dir, &snapshot_token, index_documents)?;

    for (mut document, _) in pending_index_documents {
        document.status = DocumentStatus::Searchable;
        document.updated_at = now;
        store
            .upsert_document(&document)
            .map_err(ImportPipelineError::store)?;
        summary.searchable_documents += 1;
    }

    update_index_state(store, now, snapshot_token)?;

    Ok(summary)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportOptions {
    pub scan_profile: ScanProfile,
    pub max_files: Option<usize>,
}

pub fn rebuild_full_text_index(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
) -> Result<IndexRebuildSummary> {
    let sectionizer = Sectionizer::default();
    let index_documents = persisted_index_documents(store, &sectionizer, &BTreeSet::new())?;
    let indexed_documents = index_documents.len();
    let snapshot_token = index_snapshot_token(now, indexed_documents, 0, 0);
    write_full_text_index(data_dir, &snapshot_token, index_documents)?;
    update_index_state(store, now, snapshot_token)?;

    Ok(IndexRebuildSummary { indexed_documents })
}

fn write_full_text_index(
    data_dir: &Path,
    snapshot_token: &str,
    index_documents: Vec<IndexDocument>,
) -> Result<()> {
    publish_snapshot(
        &data_dir.join("search-index"),
        snapshot_token,
        index_documents,
    )
    .map_err(ImportPipelineError::index)
}

fn update_index_state(store: &MetaStore, now: UnixTimestamp, snapshot_token: String) -> Result<()> {
    store
        .upsert_index_state(&IndexState {
            manifest_version: INDEX_MANIFEST_VERSION.to_string(),
            snapshot_token: Some(snapshot_token),
            status: IndexStateStatus::Ready,
            updated_at: now,
        })
        .map_err(ImportPipelineError::store)
}

fn index_snapshot_token(
    now: UnixTimestamp,
    indexed_documents: usize,
    ocr_required_documents: usize,
    deleted_documents: usize,
) -> String {
    format!(
        "fulltext-{}-{}-{indexed_documents}-{ocr_required_documents}-{deleted_documents}",
        now.as_unix_seconds(),
        snapshot_unique_suffix(now)
    )
}

fn snapshot_unique_suffix(now: UnixTimestamp) -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_else(|_| now.as_unix_seconds() as u128)
}

fn mark_missing_documents_deleted(
    store: &MetaStore,
    root: &Path,
    scan_profile: ScanProfile,
    scanned_directories: &[NormalizedPath],
    skipped_directories: &[NormalizedPath],
    discovered_doc_ids: &BTreeSet<String>,
    now: UnixTimestamp,
) -> Result<usize> {
    let documents = store
        .visible_documents()
        .map_err(ImportPipelineError::store)?;
    let mut deleted_count = 0_usize;

    for document in documents {
        if !document_path_is_deletion_candidate(
            &document.normalized_path,
            root,
            scan_profile,
            scanned_directories,
            skipped_directories,
        ) {
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

fn document_path_is_deletion_candidate(
    document_path: &str,
    root: &Path,
    scan_profile: ScanProfile,
    scanned_directories: &[NormalizedPath],
    skipped_directories: &[NormalizedPath],
) -> bool {
    if !document_path_is_under_root(document_path, root) {
        return false;
    }

    if scan_profile == ScanProfile::Explicit {
        return true;
    }

    document_parent_is_scanned(document_path, scanned_directories)
        && !document_path_is_under_any_normalized_root(document_path, skipped_directories)
}

fn document_path_is_under_root(document_path: &str, root: &Path) -> bool {
    Path::new(document_path).starts_with(root)
}

fn document_path_is_under_any_normalized_root(
    document_path: &str,
    roots: &[NormalizedPath],
) -> bool {
    let document_path = Path::new(document_path);
    roots
        .iter()
        .any(|root| document_path.starts_with(Path::new(root.as_str())))
}

fn document_parent_is_scanned(document_path: &str, scanned_directories: &[NormalizedPath]) -> bool {
    let Some(parent_path) = normalized_parent_path(document_path) else {
        return false;
    };

    scanned_directories
        .iter()
        .any(|directory| directory.as_str() == parent_path)
}

fn normalized_parent_path(path: &str) -> Option<&str> {
    let (parent, _) = path.rsplit_once('/')?;
    if parent.is_empty() {
        Some("/")
    } else {
        Some(parent)
    }
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
    data_dir: &Path,
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
            return Ok(if document.status == DocumentStatus::OcrRequired {
                ProcessedFile::OcrRequired {
                    ocr_job_queued: mark_ocr_required_and_enqueue(store, &mut document, now)?,
                }
            } else {
                store
                    .upsert_document(&document)
                    .map_err(ImportPipelineError::store)?;
                ProcessedFile::Failed
            });
        }
    };

    if parse_output.status() == ParseStatus::OcrRequired {
        return Ok(ProcessedFile::OcrRequired {
            ocr_job_queued: mark_ocr_required_and_enqueue(store, &mut document, now)?,
        });
    }

    let clean_text = TextNormalizer::normalize(parse_output.text())
        .text()
        .to_string();
    if clean_text.trim().is_empty() {
        return Ok(ProcessedFile::OcrRequired {
            ocr_job_queued: mark_ocr_required_and_enqueue(store, &mut document, now)?,
        });
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
    let existing_candidate_id = store
        .resume_version_by_id(&version_id)
        .map_err(ImportPipelineError::store)?
        .and_then(|version| version.candidate_id);
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id.clone(),
            document_id: document.id.clone(),
            candidate_id: existing_candidate_id,
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
    let mentions = entity_mentions_from_rules(&version_id, &clean_text);
    store
        .replace_entity_mentions(&version_id, &mentions)
        .map_err(ImportPipelineError::store)?;
    assign_candidate_from_contact_mentions(data_dir, store, &version_id, &mentions)?;

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

fn assign_candidate_from_contact_mentions(
    data_dir: &Path,
    store: &MetaStore,
    version_id: &ResumeVersionId,
    mentions: &[EntityMention],
) -> Result<()> {
    let email = best_normalized_contact(mentions, EntityType::Email);
    let phone = best_normalized_contact(mentions, EntityType::Phone);
    if email.is_none() && phone.is_none() {
        return Ok(());
    }

    let hasher = ContactHasher::load_or_create(data_dir).map_err(ImportPipelineError::privacy)?;
    let email_hash = email
        .map(|value| hasher.hash_contact(ContactKind::Email, value))
        .transpose()
        .map_err(ImportPipelineError::privacy)?;
    let phone_hash = phone
        .map(|value| hasher.hash_contact(ContactKind::Phone, value))
        .transpose()
        .map_err(ImportPipelineError::privacy)?;
    store
        .assign_candidate_from_hashed_contacts(version_id, email_hash.as_ref(), phone_hash.as_ref())
        .map_err(ImportPipelineError::store)?;

    Ok(())
}

fn best_normalized_contact(mentions: &[EntityMention], entity_type: EntityType) -> Option<&str> {
    let mut candidates = mentions
        .iter()
        .filter(|mention| mention.entity_type == entity_type)
        .filter_map(|mention| {
            let normalized = mention.normalized_value.as_deref()?;
            Some((
                normalized,
                mention.confidence,
                mention.span_start.unwrap_or(usize::MAX),
            ))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.2.cmp(&right.2))
            .then_with(|| left.0.cmp(right.0))
    });
    candidates.first().map(|candidate| candidate.0)
}

fn mark_ocr_required_and_enqueue(
    store: &MetaStore,
    document: &mut Document,
    now: UnixTimestamp,
) -> Result<bool> {
    document.status = DocumentStatus::OcrRequired;
    document.updated_at = now;
    store
        .upsert_document(document)
        .map_err(ImportPipelineError::store)?;
    let enqueue = store
        .enqueue_ocr_job_for_document(&document.id, now)
        .map_err(ImportPipelineError::store)?;

    Ok(enqueue.inserted)
}

fn entity_mentions_from_rules(
    version_id: &ResumeVersionId,
    clean_text: &str,
) -> Vec<EntityMention> {
    extract_strong_fields(clean_text)
        .into_iter()
        .enumerate()
        .map(|(index, field)| entity_mention_from_rule(version_id, index, field))
        .collect()
}

fn entity_mention_from_rule(
    version_id: &ResumeVersionId,
    index: usize,
    field: RuleMatch,
) -> EntityMention {
    EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[
            "rules-v1",
            version_id.as_str(),
            &index.to_string(),
        ]),
        resume_version_id: version_id.clone(),
        section_id: None,
        entity_type: entity_type_from_field_type(&field.field_type),
        raw_value: field.raw_value,
        normalized_value: field.normalized_value,
        span_start: Some(field.span_start),
        span_end: Some(field.span_end),
        confidence: field.confidence,
        extractor: "rules-v1".to_string(),
    }
}

fn entity_type_from_field_type(field_type: &FieldType) -> EntityType {
    match field_type {
        FieldType::Email => EntityType::Email,
        FieldType::Phone => EntityType::Phone,
        FieldType::DateRange => EntityType::DateRange,
        FieldType::School => EntityType::School,
        FieldType::Degree => EntityType::Degree,
        FieldType::Company => EntityType::Company,
        FieldType::Title => EntityType::Title,
        FieldType::Skill => EntityType::Skill,
        FieldType::Certificate => EntityType::Certificate,
        FieldType::YearsExperience => EntityType::YearsExperience,
    }
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
    OcrRequired {
        ocr_job_queued: bool,
    },
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub files_discovered: usize,
    pub scan_errors: usize,
    pub ignored_entries: usize,
    pub searchable_documents: usize,
    pub ocr_required_documents: usize,
    pub ocr_jobs_queued: usize,
    pub failed_documents: usize,
    pub deleted_documents: usize,
    pub scan_budget: Option<ImportScanBudget>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportScanBudget {
    pub kind: ImportScanBudgetKind,
    pub limit: usize,
    pub observed: usize,
    pub exhausted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportScanBudgetKind {
    Files,
}

impl From<fs_crawler::ScanBudgetExhausted> for ImportScanBudget {
    fn from(value: fs_crawler::ScanBudgetExhausted) -> Self {
        Self {
            kind: match value.kind {
                ScanBudgetKind::Files => ImportScanBudgetKind::Files,
            },
            limit: value.limit,
            observed: value.observed,
            exhausted: true,
        }
    }
}

fn import_scan_errors_from_crawl(
    task_id: &ImportTaskId,
    errors: &[CrawlError],
    now: UnixTimestamp,
) -> Vec<ImportScanError> {
    errors
        .iter()
        .enumerate()
        .map(|(index, error)| ImportScanError {
            import_task_id: task_id.clone(),
            error_index: u64::try_from(index).expect("scan error index fits into u64"),
            kind: import_scan_error_kind(error.kind),
            operation: import_scan_error_operation(error.operation),
            path_digest: None,
            updated_at: now,
        })
        .collect()
}

fn import_scan_error_kind(kind: CrawlErrorKind) -> ImportScanErrorKind {
    match kind {
        CrawlErrorKind::PermissionDenied => ImportScanErrorKind::PermissionDenied,
        CrawlErrorKind::SourceUnavailable => ImportScanErrorKind::SourceUnavailable,
        CrawlErrorKind::LockedOrUnreadable => ImportScanErrorKind::LockedOrUnreadable,
        CrawlErrorKind::Io => ImportScanErrorKind::Io,
    }
}

fn import_scan_error_operation(operation: FsOperation) -> ImportScanErrorOperation {
    match operation {
        FsOperation::NormalizePath => ImportScanErrorOperation::NormalizePath,
        FsOperation::ReadDirectory => ImportScanErrorOperation::ReadDirectory,
        FsOperation::ReadMetadata => ImportScanErrorOperation::ReadMetadata,
        FsOperation::Fingerprint => ImportScanErrorOperation::Fingerprint,
    }
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

    fn privacy(_error: privacy::PrivacyError) -> Self {
        Self {
            kind: ImportPipelineErrorKind::Privacy,
            retryable: false,
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
            ImportPipelineErrorKind::Privacy => {
                formatter.write_str("contact privacy boundary failed")
            }
        }
    }
}

impl std::error::Error for ImportPipelineError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportPipelineErrorKind {
    Store,
    Crawl,
    Index,
    Privacy,
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use fs_crawler::{normalize_path, NormalizedPath, ScanProfile};

    use super::document_path_is_deletion_candidate;

    #[test]
    fn discovery_deletion_requires_direct_parent_directory_to_be_scanned() {
        let root = Path::new("/fixture");
        let scanned_directories = vec![normalized_path("/fixture")];

        assert!(document_path_is_deletion_candidate(
            "/fixture/root-resume.pdf",
            root,
            ScanProfile::Discovery,
            &scanned_directories,
            &[],
        ));
        assert!(!document_path_is_deletion_candidate(
            "/fixture/unreadable/resume.pdf",
            root,
            ScanProfile::Discovery,
            &scanned_directories,
            &[],
        ));
    }

    #[test]
    fn discovery_deletion_excludes_skipped_subtrees_even_when_parent_was_seen() {
        let root = Path::new("/fixture");
        let scanned_directories = vec![
            normalized_path("/fixture"),
            normalized_path("/fixture/Documents"),
        ];
        let skipped_directories = vec![normalized_path("/fixture/node_modules")];

        assert!(document_path_is_deletion_candidate(
            "/fixture/Documents/resume.pdf",
            root,
            ScanProfile::Discovery,
            &scanned_directories,
            &skipped_directories,
        ));
        assert!(!document_path_is_deletion_candidate(
            "/fixture/node_modules/resume.pdf",
            root,
            ScanProfile::Discovery,
            &scanned_directories,
            &skipped_directories,
        ));
    }

    fn normalized_path(path: &str) -> NormalizedPath {
        normalize_path(path).unwrap()
    }
}
