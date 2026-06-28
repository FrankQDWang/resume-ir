use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use core_domain::{EntityMentionId, SectionType};
use extractor_rules::{extract_strong_fields, FieldType, RuleMatch};
pub use fs_crawler::ScanProfile;
use fs_crawler::{
    crawl_directory_with_options_and_control, normalize_path, CrawlError, CrawlErrorKind,
    DiscoveredFile, FsOperation, NormalizedPath, ScanBudgetKind, ScanControl, ScanOptions,
};
use index_fulltext::{
    incremental_snapshot_documents, publish_snapshot, IndexDocument, IndexSection,
};
use meta_store::{
    Document, DocumentId, DocumentStatus, EntityMention, EntityType, FileExtension,
    ImportScanBudgetKind as StoreImportScanBudgetKind, ImportScanError, ImportScanErrorKind,
    ImportScanErrorOperation, ImportTask, ImportTaskId, ImportTaskStatus, IndexState,
    IndexStateStatus, MetaStore, ResumeVersion, ResumeVersionId, ResumeVisibility, UnixTimestamp,
};
use parser_common::{ParseInput, ParseStatus, Parser, ParserErrorKind, ResourceBudget};
use parser_doc::DocParser;
use parser_docx::DocxParser;
use parser_pdf::PdfParser;
use parser_text::TxtParser;
use privacy::{ContactHasher, ContactKind};
use sectionizer::{SectionChunk, Sectionizer};
use text_normalizer::TextNormalizer;

const PARSE_VERSION: &str = "parser-v1";
const OCR_PARSE_VERSION: &str = "ocr-v1";
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
    if task.status != ImportTaskStatus::Running {
        store
            .update_import_task_status(&task.id, ImportTaskStatus::Running, now)
            .map_err(ImportPipelineError::store)?;
    }

    let result = run_import(data_dir, store, task, root, now, options);
    let finished_at = current_timestamp_or(now);
    match result {
        Ok(summary) => {
            store
                .update_import_task_status(&task.id, ImportTaskStatus::Completed, finished_at)
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
                finished_at,
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
    ensure_import_not_cancelled(store, &task.id)?;
    let cancel_check = || store.is_import_task_cancelled(&task.id).unwrap_or(true);
    let ensure_not_cancelled = || ensure_import_not_cancelled(store, &task.id);
    let report = crawl_directory_with_options_and_control(
        root,
        ScanOptions {
            profile: options.scan_profile,
            max_files: options.max_files,
        },
        ScanControl::from_cancel_check(&cancel_check),
    )
    .map_err(ImportPipelineError::crawl)?;
    ensure_import_not_cancelled(store, &task.id)?;
    let scanned_directories = report.scanned_directories.clone();
    let skipped_directories = report.skipped_directories.clone();
    let scan_errors = import_scan_errors_from_crawl(&task.id, &report.errors, now);
    let scan_budget_exhausted = report.budget_exhausted;
    let scan_budget = options.max_files.map(|limit| ImportScanBudget {
        kind: ImportScanBudgetKind::Files,
        limit,
        observed: report.files.len(),
        exhausted: scan_budget_exhausted.is_some(),
    });
    let mut summary = ImportSummary {
        files_discovered: report.files.len(),
        scan_errors: report.errors.len(),
        ignored_entries: report.ignored_count,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        failure_counts: ImportFailureCounts::default(),
        deleted_documents: 0,
        scan_budget,
    };
    store
        .replace_import_scan_errors(&task.id, &scan_errors)
        .map_err(ImportPipelineError::store)?;
    publish_import_progress(store, &task.id, &summary, now)?;
    ensure_import_not_cancelled(store, &task.id)?;
    let mut pending_index_documents = Vec::new();
    let sectionizer = Sectionizer::default();
    let can_propagate_deletions = report.errors.is_empty() && scan_budget_exhausted.is_none();
    let discovered_doc_ids = report
        .files
        .iter()
        .map(|file| file.document_id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let mut pending_excluded_doc_ids = BTreeSet::new();

    let total_files = report.files.len();
    for (index, file) in report.files.into_iter().enumerate() {
        ensure_not_cancelled()?;
        match process_file(
            data_dir,
            store,
            &file,
            &sectionizer,
            now,
            &ensure_not_cancelled,
        )? {
            ProcessedFile::Searchable {
                document,
                index_document,
            } => {
                pending_excluded_doc_ids.insert(file.document_id.as_str().to_string());
                pending_index_documents.push((*document, *index_document));
            }
            ProcessedFile::OcrRequired { ocr_job_queued } => {
                pending_excluded_doc_ids.insert(file.document_id.as_str().to_string());
                summary.ocr_required_documents += 1;
                if ocr_job_queued {
                    summary.ocr_jobs_queued += 1;
                }
            }
            ProcessedFile::Failed { kind } => {
                pending_excluded_doc_ids.insert(file.document_id.as_str().to_string());
                summary.failed_documents += 1;
                summary.failure_counts.increment(kind);
            }
            ProcessedFile::Unchanged => {}
        }
        let flushed_searchables = if should_flush_searchable_documents(
            index,
            total_files,
            pending_index_documents.len(),
            summary.searchable_documents,
        ) {
            flush_pending_searchable_documents(
                data_dir,
                store,
                now,
                &mut summary,
                &mut pending_index_documents,
                &mut pending_excluded_doc_ids,
                &ensure_not_cancelled,
            )?
        } else {
            false
        };
        if flushed_searchables || should_publish_import_progress(index, total_files) {
            publish_import_progress(store, &task.id, &summary, now)?;
        }
    }

    if can_propagate_deletions {
        ensure_import_not_cancelled(store, &task.id)?;
        let deleted_document_ids = mark_missing_documents_deleted(
            store,
            root,
            options.scan_profile,
            &scanned_directories,
            &skipped_directories,
            &discovered_doc_ids,
            now,
        )?;
        summary.deleted_documents = deleted_document_ids.len();
        pending_excluded_doc_ids.extend(deleted_document_ids);
        publish_import_progress(store, &task.id, &summary, now)?;
    } else {
        summary.deleted_documents = 0;
    }
    flush_pending_searchable_documents(
        data_dir,
        store,
        now,
        &mut summary,
        &mut pending_index_documents,
        &mut pending_excluded_doc_ids,
        &ensure_not_cancelled,
    )?;
    publish_import_progress(store, &task.id, &summary, now)?;

    Ok(summary)
}

const IMPORT_PROGRESS_UPDATE_EVERY_FILES: usize = 32;
const IMPORT_SEARCHABLE_FLUSH_BATCH: usize = 8;

fn should_publish_import_progress(index: usize, total: usize) -> bool {
    let processed = index + 1;
    processed == total || processed.is_multiple_of(IMPORT_PROGRESS_UPDATE_EVERY_FILES)
}

fn should_flush_searchable_documents(
    index: usize,
    total: usize,
    pending_searchable_documents: usize,
    searchable_documents: usize,
) -> bool {
    let processed = index + 1;
    processed == total
        || (searchable_documents == 0 && pending_searchable_documents > 0)
        || pending_searchable_documents >= IMPORT_SEARCHABLE_FLUSH_BATCH
        || processed.is_multiple_of(IMPORT_PROGRESS_UPDATE_EVERY_FILES)
}

fn publish_import_progress(
    store: &MetaStore,
    task_id: &ImportTaskId,
    summary: &ImportSummary,
    updated_at: UnixTimestamp,
) -> Result<()> {
    let Some(mut scope) = store
        .import_scan_scope_by_task_id(task_id)
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(());
    };

    scope.files_discovered = summary.files_discovered as u64;
    scope.ignored_entries = summary.ignored_entries as u64;
    scope.scan_errors = summary.scan_errors as u64;
    scope.searchable_documents = summary.searchable_documents as u64;
    scope.ocr_required_documents = summary.ocr_required_documents as u64;
    scope.ocr_jobs_queued = summary.ocr_jobs_queued as u64;
    scope.failed_documents = summary.failed_documents as u64;
    scope.deleted_documents = summary.deleted_documents as u64;
    scope.scan_budget_kind = summary.scan_budget.map(|budget| match budget.kind {
        ImportScanBudgetKind::Files => StoreImportScanBudgetKind::Files,
    });
    scope.scan_budget_limit = summary.scan_budget.map(|budget| budget.limit as u64);
    scope.scan_budget_observed = summary.scan_budget.map(|budget| budget.observed as u64);
    scope.scan_budget_exhausted = summary.scan_budget.is_some_and(|budget| budget.exhausted);
    scope.updated_at = current_timestamp_or(updated_at);
    store
        .upsert_import_scan_scope(&scope)
        .map_err(ImportPipelineError::store)
}

fn flush_pending_searchable_documents(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<(Document, IndexDocument)>,
    pending_excluded_doc_ids: &mut BTreeSet<String>,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
) -> Result<bool> {
    if pending_index_documents.is_empty() && pending_excluded_doc_ids.is_empty() {
        return Ok(false);
    }

    ensure_not_cancelled()?;
    let pending_replacements = pending_index_documents
        .iter()
        .map(|(_, index_document)| index_document.clone())
        .collect::<Vec<_>>();
    let (snapshot_token, indexed_document_count) = write_incremental_full_text_index(
        data_dir,
        store,
        now,
        pending_replacements,
        pending_excluded_doc_ids,
        summary.ocr_required_documents,
        summary.deleted_documents,
    )?;

    for (mut document, _) in pending_index_documents.drain(..) {
        ensure_not_cancelled()?;
        document.status = DocumentStatus::Searchable;
        document.updated_at = now;
        store
            .upsert_document(&document)
            .map_err(ImportPipelineError::store)?;
        summary.searchable_documents += 1;
    }

    pending_excluded_doc_ids.clear();
    ensure_not_cancelled()?;
    update_index_state(store, now, snapshot_token, indexed_document_count)?;
    Ok(true)
}

fn ensure_import_not_cancelled(store: &MetaStore, task_id: &ImportTaskId) -> Result<()> {
    if store
        .is_import_task_cancelled(task_id)
        .map_err(ImportPipelineError::store)?
    {
        Err(ImportPipelineError::cancelled())
    } else {
        Ok(())
    }
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
    let (snapshot_token, indexed_documents) =
        write_rebuilt_full_text_index(data_dir, store, now, &BTreeSet::new(), Vec::new())?;
    update_index_state(store, now, snapshot_token, indexed_documents)?;

    Ok(IndexRebuildSummary { indexed_documents })
}

pub fn remove_documents_from_full_text_index(
    data_dir: &Path,
    store: &MetaStore,
    document_ids: &BTreeSet<String>,
    now: UnixTimestamp,
) -> Result<IndexRebuildSummary> {
    let (snapshot_token, indexed_documents) = write_incremental_full_text_index(
        data_dir,
        store,
        now,
        Vec::new(),
        document_ids,
        0,
        document_ids.len(),
    )?;
    update_index_state(store, now, snapshot_token, indexed_documents)?;

    Ok(IndexRebuildSummary { indexed_documents })
}

pub fn index_ocr_text(
    data_dir: &Path,
    store: &MetaStore,
    document_id: &DocumentId,
    ocr_text: &str,
    confidence: Option<f32>,
    page_count: Option<u32>,
    now: UnixTimestamp,
) -> Result<OcrTextIndexSummary> {
    let Some(mut document) = store
        .document_by_id(document_id)
        .map_err(ImportPipelineError::store)?
    else {
        return Err(ImportPipelineError {
            kind: ImportPipelineErrorKind::Store,
            retryable: false,
        });
    };

    let clean_text = TextNormalizer::normalize(ocr_text).text().to_string();
    let pending_doc_ids = BTreeSet::from([document.id.as_str().to_string()]);
    if clean_text.trim().is_empty() {
        let (snapshot_token, indexed_documents) = write_incremental_full_text_index(
            data_dir,
            store,
            now,
            Vec::new(),
            &pending_doc_ids,
            0,
            0,
        )?;
        document.status = DocumentStatus::OcrDone;
        document.updated_at = now;
        store
            .upsert_document(&document)
            .map_err(ImportPipelineError::store)?;
        update_index_state(store, now, snapshot_token, indexed_documents)?;
        return Ok(OcrTextIndexSummary {
            searchable: false,
            indexed_documents,
        });
    }

    let version_id = ResumeVersionId::from_non_secret_parts(&[
        "ocr",
        document.id.as_str(),
        OCR_PARSE_VERSION,
        SCHEMA_VERSION,
    ]);
    let existing_candidate_id = store
        .resume_version_by_id(&version_id)
        .map_err(ImportPipelineError::store)?
        .and_then(|version| version.candidate_id);
    store
        .upsert_resume_version(&ResumeVersion {
            id: version_id.clone(),
            document_id: document.id.clone(),
            candidate_id: existing_candidate_id,
            parse_version: OCR_PARSE_VERSION.to_string(),
            schema_version: SCHEMA_VERSION.to_string(),
            language_set: language_set(&clean_text),
            page_count,
            raw_text: Some(ocr_text.to_string()),
            clean_text: Some(clean_text.clone()),
            quality_score: Some(confidence.unwrap_or(0.5)),
            visibility: ResumeVisibility::Searchable,
        })
        .map_err(ImportPipelineError::store)?;
    let mentions = entity_mentions_from_rules(&version_id, &clean_text);
    store
        .replace_entity_mentions(&version_id, &mentions)
        .map_err(ImportPipelineError::store)?;
    assign_candidate_from_contact_mentions(data_dir, store, &version_id, &mentions)?;

    let sectionizer = Sectionizer::default();
    let pending_index_document = IndexDocument {
        doc_id: document.id.to_string(),
        version_id: version_id.to_string(),
        file_name: document.file_name.clone(),
        clean_text: clean_text.clone(),
        sections: sections_to_index(sectionizer.sectionize(&clean_text)),
        is_deleted: document.is_deleted,
    };
    let (snapshot_token, indexed_documents) = write_incremental_full_text_index(
        data_dir,
        store,
        now,
        vec![pending_index_document],
        &pending_doc_ids,
        0,
        0,
    )?;
    document.status = DocumentStatus::Searchable;
    document.updated_at = now;
    store
        .upsert_document(&document)
        .map_err(ImportPipelineError::store)?;
    update_index_state(store, now, snapshot_token, indexed_documents)?;

    Ok(OcrTextIndexSummary {
        searchable: true,
        indexed_documents,
    })
}

pub fn detect_ocr_page_count(extension: &FileExtension, bytes: &[u8]) -> Result<u32> {
    if !matches!(extension, FileExtension::Pdf) {
        return Ok(1);
    }

    let output = PdfParser
        .parse(
            ParseInput::from_bytes(Some("pdf"), bytes),
            ResourceBudget::default(),
        )
        .map_err(ImportPipelineError::parser)?;
    Ok(output
        .page_count()
        .and_then(|page_count| u32::try_from(page_count).ok())
        .filter(|page_count| *page_count > 0)
        .unwrap_or(1))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OcrTextIndexSummary {
    pub searchable: bool,
    pub indexed_documents: usize,
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

fn write_incremental_full_text_index(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    replacement_documents: Vec<IndexDocument>,
    excluded_doc_ids: &BTreeSet<String>,
    ocr_required_documents: usize,
    deleted_documents: usize,
) -> Result<(String, usize)> {
    let index_documents = match incremental_snapshot_documents(
        &data_dir.join("search-index"),
        replacement_documents.clone(),
        excluded_doc_ids,
    ) {
        Ok(index_documents) => index_documents,
        Err(_) => {
            let sectionizer = Sectionizer::default();
            let mut rebuilt_documents =
                persisted_index_documents(store, &sectionizer, excluded_doc_ids)?;
            rebuilt_documents.extend(
                replacement_documents
                    .into_iter()
                    .filter(|document| !document.is_deleted),
            );
            rebuilt_documents
        }
    };
    let indexed_documents = index_documents.len();
    let snapshot_token = index_snapshot_token(
        now,
        indexed_documents,
        ocr_required_documents,
        deleted_documents,
    );
    write_full_text_index(data_dir, &snapshot_token, index_documents)?;

    Ok((snapshot_token, indexed_documents))
}

fn write_rebuilt_full_text_index(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    pending_doc_ids: &BTreeSet<String>,
    pending_index_documents: Vec<IndexDocument>,
) -> Result<(String, usize)> {
    let sectionizer = Sectionizer::default();
    let mut index_documents = persisted_index_documents(store, &sectionizer, pending_doc_ids)?;
    index_documents.extend(pending_index_documents);
    let indexed_documents = index_documents.len();
    let snapshot_token = index_snapshot_token(now, indexed_documents, 0, 0);
    write_full_text_index(data_dir, &snapshot_token, index_documents)?;

    Ok((snapshot_token, indexed_documents))
}

fn update_index_state(
    store: &MetaStore,
    now: UnixTimestamp,
    snapshot_token: String,
    manifest_document_count: usize,
) -> Result<()> {
    let visible_epoch = store
        .index_state()
        .map_err(ImportPipelineError::store)?
        .map_or(1, |state| state.visible_epoch.saturating_add(1));
    store
        .upsert_index_state(&IndexState {
            manifest_version: INDEX_MANIFEST_VERSION.to_string(),
            snapshot_token: Some(snapshot_token),
            status: IndexStateStatus::Ready,
            updated_at: now,
            visible_epoch,
            manifest_document_count: manifest_document_count as u64,
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

fn current_timestamp_or(default: UnixTimestamp) -> UnixTimestamp {
    let Some(current) = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .map(UnixTimestamp::from_unix_seconds)
    else {
        return default;
    };

    if current.as_unix_seconds() >= default.as_unix_seconds() {
        current
    } else {
        default
    }
}

fn mark_missing_documents_deleted(
    store: &MetaStore,
    root: &Path,
    scan_profile: ScanProfile,
    scanned_directories: &[NormalizedPath],
    skipped_directories: &[NormalizedPath],
    discovered_doc_ids: &BTreeSet<String>,
    now: UnixTimestamp,
) -> Result<BTreeSet<String>> {
    let documents = store
        .visible_documents()
        .map_err(ImportPipelineError::store)?;
    let mut deleted_doc_ids = BTreeSet::new();

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
            deleted_doc_ids.insert(document.id.as_str().to_string());
        }
    }

    Ok(deleted_doc_ids)
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
    let Ok(root) = normalize_path(root) else {
        return false;
    };
    normalized_path_is_under_root(document_path, root.as_str())
}

fn document_path_is_under_any_normalized_root(
    document_path: &str,
    roots: &[NormalizedPath],
) -> bool {
    roots
        .iter()
        .any(|root| normalized_path_is_under_root(document_path, root.as_str()))
}

fn normalized_path_is_under_root(document_path: &str, root: &str) -> bool {
    if document_path == root {
        return true;
    }
    if root.ends_with('/') {
        return document_path.starts_with(root);
    }

    document_path
        .strip_prefix(root)
        .is_some_and(|suffix| suffix.starts_with('/'))
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
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
) -> Result<ProcessedFile> {
    ensure_not_cancelled()?;
    if file_is_exact_rerun_noop(store, file)? {
        return Ok(ProcessedFile::Unchanged);
    }

    let mut document = document_from_discovered_file(file, now, DocumentStatus::Discovered);
    store
        .upsert_document(&document)
        .map_err(ImportPipelineError::store)?;
    ensure_not_cancelled()?;

    if file.extension == FileExtension::Txt
        && file.byte_size > parser_text::DEFAULT_MAX_BYTES as u64
    {
        document.status = DocumentStatus::FailedPermanent;
        document.updated_at = now;
        store
            .upsert_document(&document)
            .map_err(ImportPipelineError::store)?;
        return Ok(ProcessedFile::Failed {
            kind: ImportFailureKind::TextTooLarge,
        });
    }

    let path = PathBuf::from(file.normalized_path.as_str());
    ensure_not_cancelled()?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            document.status = DocumentStatus::FailedRetryable;
            store
                .upsert_document(&document)
                .map_err(ImportPipelineError::store)?;
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::ReadError,
            });
        }
    };
    ensure_not_cancelled()?;

    let extension = file_extension_label(&file.extension);
    ensure_not_cancelled()?;
    let parse_output = match file.extension {
        FileExtension::Docx => DocxParser
            .parse(
                ParseInput::from_bytes(Some(extension), &bytes),
                ResourceBudget::default(),
            )
            .map_err(|error| (error, document.clone())),
        FileExtension::Doc => DocParser::default()
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
        FileExtension::Txt => TxtParser
            .parse(
                ParseInput::from_bytes(Some(extension), &bytes),
                ResourceBudget::default().with_max_bytes(parser_text::DEFAULT_MAX_BYTES),
            )
            .map_err(|error| (error, document.clone())),
        _ => {
            document.status = DocumentStatus::FailedPermanent;
            store
                .upsert_document(&document)
                .map_err(ImportPipelineError::store)?;
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::UnsupportedExtension,
            });
        }
    };
    ensure_not_cancelled()?;

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
                ProcessedFile::Failed {
                    kind: ImportFailureKind::from_parser_error(error.kind()),
                }
            });
        }
    };

    if parse_output.status() == ParseStatus::OcrRequired {
        ensure_not_cancelled()?;
        return Ok(ProcessedFile::OcrRequired {
            ocr_job_queued: mark_ocr_required_and_enqueue(store, &mut document, now)?,
        });
    }

    ensure_not_cancelled()?;
    let clean_text = TextNormalizer::normalize(parse_output.text())
        .text()
        .to_string();
    if clean_text.trim().is_empty() {
        if file.extension == FileExtension::Txt {
            document.status = DocumentStatus::FailedPermanent;
            document.updated_at = now;
            store
                .upsert_document(&document)
                .map_err(ImportPipelineError::store)?;
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::EmptyText,
            });
        }

        ensure_not_cancelled()?;
        return Ok(ProcessedFile::OcrRequired {
            ocr_job_queued: mark_ocr_required_and_enqueue(store, &mut document, now)?,
        });
    }

    ensure_not_cancelled()?;
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
    ensure_not_cancelled()?;
    store
        .replace_entity_mentions(&version_id, &mentions)
        .map_err(ImportPipelineError::store)?;
    ensure_not_cancelled()?;
    assign_candidate_from_contact_mentions(data_dir, store, &version_id, &mentions)?;

    ensure_not_cancelled()?;
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
        FieldType::Name => EntityType::Name,
        FieldType::Email => EntityType::Email,
        FieldType::Phone => EntityType::Phone,
        FieldType::WeChat => EntityType::WeChat,
        FieldType::DateRange => EntityType::DateRange,
        FieldType::School => EntityType::School,
        FieldType::SchoolTier => EntityType::SchoolTier,
        FieldType::Degree => EntityType::Degree,
        FieldType::Major => EntityType::Major,
        FieldType::Company => EntityType::Company,
        FieldType::Title => EntityType::Title,
        FieldType::Location => EntityType::Location,
        FieldType::Skill => EntityType::Skill,
        FieldType::Certificate => EntityType::Certificate,
        FieldType::YearsExperience => EntityType::YearsExperience,
    }
}

fn file_is_exact_rerun_noop(store: &MetaStore, file: &DiscoveredFile) -> Result<bool> {
    let Some(document) = store
        .document_by_id(&file.document_id)
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(false);
    };

    if document.is_deleted
        || document.normalized_path != file.normalized_path.as_str()
        || document.file_name != file.file_name
        || document.extension != file.extension
        || document.byte_size != file.byte_size
        || document.mtime != file.mtime
        || document.content_hash.as_deref() != Some(file.fingerprint.as_str())
    {
        return Ok(false);
    }

    match document.status {
        DocumentStatus::Searchable | DocumentStatus::IndexedPartial => store
            .latest_visible_resume_version_for_document(&document.id)
            .map_err(ImportPipelineError::store)
            .map(|version| version.is_some()),
        DocumentStatus::OcrRequired => Ok(true),
        _ => Ok(false),
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
    Unchanged,
    OcrRequired {
        ocr_job_queued: bool,
    },
    Failed {
        kind: ImportFailureKind,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub files_discovered: usize,
    pub scan_errors: usize,
    pub ignored_entries: usize,
    pub searchable_documents: usize,
    pub ocr_required_documents: usize,
    pub ocr_jobs_queued: usize,
    pub failed_documents: usize,
    pub failure_counts: ImportFailureCounts,
    pub deleted_documents: usize,
    pub scan_budget: Option<ImportScanBudget>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportFailureCounts {
    counts: BTreeMap<ImportFailureKind, usize>,
}

impl ImportFailureCounts {
    fn increment(&mut self, kind: ImportFailureKind) {
        *self.counts.entry(kind).or_default() += 1;
    }

    pub fn add(&mut self, kind: ImportFailureKind, count: usize) {
        *self.counts.entry(kind).or_default() += count;
    }

    pub fn get(&self, kind: ImportFailureKind) -> usize {
        self.counts.get(&kind).copied().unwrap_or(0)
    }

    pub fn entries(&self) -> impl Iterator<Item = (ImportFailureKind, usize)> + '_ {
        self.counts.iter().map(|(kind, count)| (*kind, *count))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportFailureKind {
    TextTooLarge,
    ReadError,
    UnsupportedExtension,
    ParserUnsupported,
    ParserCorrupted,
    ParserEncrypted,
    ParserTimeout,
    ParserResourceExhausted,
    ParserIo,
    ParserCancelled,
    ParserInternal,
    EmptyText,
}

impl ImportFailureKind {
    fn from_parser_error(kind: ParserErrorKind) -> Self {
        match kind {
            ParserErrorKind::Unsupported => Self::ParserUnsupported,
            ParserErrorKind::Corrupted => Self::ParserCorrupted,
            ParserErrorKind::Encrypted => Self::ParserEncrypted,
            ParserErrorKind::Timeout => Self::ParserTimeout,
            ParserErrorKind::ResourceExhausted => Self::ParserResourceExhausted,
            ParserErrorKind::Io => Self::ParserIo,
            ParserErrorKind::Cancelled => Self::ParserCancelled,
            ParserErrorKind::OcrRequired | ParserErrorKind::Internal => Self::ParserInternal,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TextTooLarge => "text_too_large",
            Self::ReadError => "read_error",
            Self::UnsupportedExtension => "unsupported_extension",
            Self::ParserUnsupported => "parser_unsupported",
            Self::ParserCorrupted => "parser_corrupted",
            Self::ParserEncrypted => "parser_encrypted",
            Self::ParserTimeout => "parser_timeout",
            Self::ParserResourceExhausted => "parser_resource_exhausted",
            Self::ParserIo => "parser_io",
            Self::ParserCancelled => "parser_cancelled",
            Self::ParserInternal => "parser_internal",
            Self::EmptyText => "empty_text",
        }
    }
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
        CrawlErrorKind::Cancelled => ImportScanErrorKind::Io,
        CrawlErrorKind::PermissionDenied => ImportScanErrorKind::PermissionDenied,
        CrawlErrorKind::SourceUnavailable => ImportScanErrorKind::SourceUnavailable,
        CrawlErrorKind::LockedOrUnreadable => ImportScanErrorKind::LockedOrUnreadable,
        CrawlErrorKind::Io => ImportScanErrorKind::Io,
    }
}

fn import_scan_error_operation(operation: FsOperation) -> ImportScanErrorOperation {
    match operation {
        FsOperation::CheckCancellation => ImportScanErrorOperation::ReadDirectory,
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

    fn crawl(error: fs_crawler::CrawlError) -> Self {
        if error.kind == CrawlErrorKind::Cancelled {
            return Self::cancelled();
        }

        Self {
            kind: ImportPipelineErrorKind::Crawl,
            retryable: true,
        }
    }

    fn cancelled() -> Self {
        Self {
            kind: ImportPipelineErrorKind::Cancelled,
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

    fn parser(_error: parser_common::ParserError) -> Self {
        Self {
            kind: ImportPipelineErrorKind::Parser,
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
            ImportPipelineErrorKind::Cancelled => formatter.write_str("import task was cancelled"),
            ImportPipelineErrorKind::Store => formatter.write_str("metadata update failed"),
            ImportPipelineErrorKind::Crawl => formatter.write_str("file scan failed"),
            ImportPipelineErrorKind::Index => formatter.write_str("search index update failed"),
            ImportPipelineErrorKind::Privacy => {
                formatter.write_str("contact privacy boundary failed")
            }
            ImportPipelineErrorKind::Parser => formatter.write_str("document parser failed"),
        }
    }
}

impl std::error::Error for ImportPipelineError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportPipelineErrorKind {
    Cancelled,
    Store,
    Crawl,
    Index,
    Privacy,
    Parser,
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use fs_crawler::{normalize_path, NormalizedPath, ScanProfile};
    use meta_store::{
        ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskStatus,
        MetaStore, UnixTimestamp,
    };

    use super::{
        current_timestamp_or, document_path_is_deletion_candidate, import_root_with_options,
        ImportOptions, ImportPipelineErrorKind,
    };

    #[cfg(unix)]
    static DOC_CONVERTER_ENV_LOCK: Mutex<()> = Mutex::new(());

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

    #[test]
    fn deletion_candidate_matches_windows_normalized_paths() {
        let root = Path::new(r"C:\fixture");
        let scanned_directories = vec![normalized_path(r"C:\fixture")];

        assert!(document_path_is_deletion_candidate(
            "c:/fixture/resume.pdf",
            root,
            ScanProfile::Explicit,
            &scanned_directories,
            &[],
        ));
        assert!(!document_path_is_deletion_candidate(
            "c:/fixture-neighbor/resume.pdf",
            root,
            ScanProfile::Explicit,
            &scanned_directories,
            &[],
        ));
    }

    #[test]
    fn current_timestamp_or_never_returns_before_default_timestamp() {
        let future_default = UnixTimestamp::from_unix_seconds(4_000_000_000);

        assert_eq!(current_timestamp_or(future_default), future_default);
    }

    fn normalized_path(path: &str) -> NormalizedPath {
        normalize_path(path).unwrap()
    }

    #[test]
    fn import_root_stops_running_task_when_cancellation_marker_exists() {
        let temp = TestDir::new("import-pipeline-cancel-running");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            b"Synthetic Candidate\nEmail: synthetic@example.test\nSkills: Rust",
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_000);
        let cancel_at = UnixTimestamp::from_unix_seconds(1_700_000_010);
        let task = import_task("running-cancelled-import", root.to_str().unwrap(), now);
        store.insert_import_task(&task).unwrap();
        store.cancel_import_task(&task.id, cancel_at).unwrap();

        let error = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap_err();

        assert_eq!(error.kind, ImportPipelineErrorKind::Cancelled);
        let stored_task = store.import_task_by_id(&task.id).unwrap().unwrap();
        assert_eq!(stored_task.status, ImportTaskStatus::FailedRetryable);
        assert_eq!(store.status_summary().unwrap().searchable_documents, 0);
        assert!(!data_dir.join("search-index").join("active").exists());
    }

    #[test]
    fn import_root_updates_existing_scan_scope_progress_without_daemon_postprocessing() {
        let temp = TestDir::new("import-pipeline-live-progress");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            b"Synthetic Candidate\nEmail: synthetic@example.test\nSkills: Rust",
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_100);
        let task = import_task("live-progress-import", root.to_str().unwrap(), now);
        store.insert_import_task(&task).unwrap();
        store
            .upsert_import_scan_scope(&ImportScanScope {
                import_task_id: task.id.clone(),
                root_kind: ImportRootKind::Explicit,
                root_preset: None,
                scan_profile: ImportScanProfile::Explicit,
                requested_root_path: root.to_str().unwrap().to_string(),
                canonical_root_path: root.to_str().unwrap().to_string(),
                files_discovered: 0,
                ignored_entries: 0,
                scan_errors: 0,
                searchable_documents: 0,
                ocr_required_documents: 0,
                ocr_jobs_queued: 0,
                failed_documents: 0,
                deleted_documents: 0,
                scan_budget_kind: None,
                scan_budget_limit: None,
                scan_budget_observed: None,
                scan_budget_exhausted: false,
                updated_at: now,
            })
            .unwrap();

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.files_discovered, 1);
        assert_eq!(summary.searchable_documents, 1);
        let scope = store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        assert_eq!(scope.files_discovered, 1);
        assert_eq!(scope.searchable_documents, 1);
        assert_eq!(scope.scan_budget_observed, None);
        assert!(!format!("{scope:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn import_root_keeps_utf16be_literal_pdf_text_layer_searchable_without_ocr() {
        let temp = TestDir::new("import-pipeline-utf16be-literal-pdf");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("utf16-literal-resume.pdf"),
            utf16be_literal_text_layer_pdf_bytes(),
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_150);
        let task = import_task("utf16be-literal-pdf-import", root.to_str().unwrap(), now);
        store.insert_import_task(&task).unwrap();

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        let expected = "\u{4E2D}\u{6587}\u{7B80}\u{5386}";
        assert_eq!(summary.files_discovered, 1);
        assert_eq!(summary.searchable_documents, 1);
        assert_eq!(summary.ocr_required_documents, 0);
        assert_eq!(summary.failed_documents, 0);
        let document = store.visible_documents().unwrap().remove(0);
        let versions = store.resume_versions_for_document(&document.id).unwrap();
        assert_eq!(versions.len(), 1);
        assert!(versions[0]
            .clean_text
            .as_deref()
            .unwrap()
            .contains(expected));
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn import_root_keeps_tounicode_cmap_pdf_text_layer_searchable_without_ocr() {
        let temp = TestDir::new("import-pipeline-tounicode-cmap-pdf");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("tounicode-cmap-resume.pdf"),
            tounicode_cmap_pdf_bytes(),
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_175);
        let task = import_task("tounicode-cmap-pdf-import", root.to_str().unwrap(), now);
        store.insert_import_task(&task).unwrap();

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.files_discovered, 1);
        assert_eq!(summary.searchable_documents, 1);
        assert_eq!(summary.ocr_required_documents, 0);
        assert_eq!(summary.failed_documents, 0);
        let document = store.visible_documents().unwrap().remove(0);
        let versions = store.resume_versions_for_document(&document.id).unwrap();
        assert_eq!(versions.len(), 1);
        assert!(versions[0]
            .clean_text
            .as_deref()
            .unwrap()
            .contains("中文简历"));
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn import_root_rerun_with_unchanged_searchable_file_keeps_index_state_stable() {
        let temp = TestDir::new("import-pipeline-zero-change-rerun");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            b"Synthetic Candidate\nEmail: synthetic@example.test\nSkills: Rust",
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_190);
        let first_task = import_task(
            "zero-change-first-import",
            root.to_str().unwrap(),
            first_now,
        );
        store.insert_import_task(&first_task).unwrap();

        let first_summary = import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let first_index_state = store.index_state().unwrap().unwrap();
        let first_status = store.status_summary().unwrap();

        assert_eq!(first_summary.files_discovered, 1);
        assert_eq!(first_summary.searchable_documents, 1);
        assert_eq!(first_status.searchable_documents, 1);

        let second_now = UnixTimestamp::from_unix_seconds(1_700_000_191);
        let second_task = import_task(
            "zero-change-second-import",
            root.to_str().unwrap(),
            second_now,
        );
        store.insert_import_task(&second_task).unwrap();

        let second_summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();
        let second_index_state = store.index_state().unwrap().unwrap();
        let second_status = store.status_summary().unwrap();
        let documents = store.visible_documents().unwrap();

        assert_eq!(second_summary.files_discovered, 1);
        assert_eq!(second_summary.searchable_documents, 0);
        assert_eq!(second_summary.ocr_required_documents, 0);
        assert_eq!(second_summary.ocr_jobs_queued, 0);
        assert_eq!(second_summary.failed_documents, 0);
        assert_eq!(second_summary.deleted_documents, 0);
        assert_eq!(second_status.searchable_documents, 1);
        assert_eq!(second_status.ocr_jobs_queued, 0);
        assert_eq!(documents.len(), 1);
        assert_eq!(
            store
                .resume_versions_for_document(&documents[0].id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            second_index_state.visible_epoch,
            first_index_state.visible_epoch
        );
        assert_eq!(
            second_index_state.manifest_document_count,
            first_index_state.manifest_document_count
        );
        assert_eq!(
            second_index_state.snapshot_token,
            first_index_state.snapshot_token
        );
        assert!(!format!("{second_index_state:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn import_root_rerun_with_unchanged_ocr_required_file_keeps_ocr_queue_stable() {
        let temp = TestDir::new("import-pipeline-zero-change-ocr-rerun");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("scanned-resume.pdf"), scanned_pdf_bytes()).unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_195);
        let first_task = import_task(
            "zero-change-ocr-first-import",
            root.to_str().unwrap(),
            first_now,
        );
        store.insert_import_task(&first_task).unwrap();

        let first_summary = import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let first_index_state = store.index_state().unwrap().unwrap();
        let first_status = store.status_summary().unwrap();

        assert_eq!(first_summary.files_discovered, 1);
        assert_eq!(first_summary.searchable_documents, 0);
        assert_eq!(first_summary.ocr_required_documents, 1);
        assert_eq!(first_summary.ocr_jobs_queued, 1);
        assert_eq!(first_status.searchable_documents, 0);
        assert_eq!(first_status.ocr_queue_depth, 1);
        assert_eq!(first_status.ocr_jobs_queued, 1);

        let second_now = UnixTimestamp::from_unix_seconds(1_700_000_196);
        let second_task = import_task(
            "zero-change-ocr-second-import",
            root.to_str().unwrap(),
            second_now,
        );
        store.insert_import_task(&second_task).unwrap();

        let second_summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();
        let second_index_state = store.index_state().unwrap().unwrap();
        let second_status = store.status_summary().unwrap();
        let documents = store.visible_documents().unwrap();

        assert_eq!(second_summary.files_discovered, 1);
        assert_eq!(second_summary.searchable_documents, 0);
        assert_eq!(second_summary.ocr_required_documents, 0);
        assert_eq!(second_summary.ocr_jobs_queued, 0);
        assert_eq!(second_summary.failed_documents, 0);
        assert_eq!(second_summary.deleted_documents, 0);
        assert_eq!(second_status.searchable_documents, 0);
        assert_eq!(second_status.ocr_queue_depth, 1);
        assert_eq!(second_status.ocr_jobs_queued, 1);
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].status, meta_store::DocumentStatus::OcrRequired);
        assert_eq!(
            store
                .resume_versions_for_document(&documents[0].id)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            second_index_state.visible_epoch,
            first_index_state.visible_epoch
        );
        assert_eq!(
            second_index_state.manifest_document_count,
            first_index_state.manifest_document_count
        );
        assert_eq!(
            second_index_state.snapshot_token,
            first_index_state.snapshot_token
        );
        assert!(!format!("{second_index_state:?}").contains(root.to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn import_root_parses_legacy_doc_with_local_converter_without_path_leak() {
        let _env_lock = DOC_CONVERTER_ENV_LOCK.lock().unwrap();
        let temp = TestDir::new("import-pipeline-doc-converter");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("legacy-word.doc"), synthetic_ole_doc()).unwrap();
        let converter = write_doc_converter(temp.path());
        let _env = EnvVarGuard::set(
            "RESUME_IR_DOC_TEXT_COMMAND",
            converter.to_str().unwrap().to_string(),
        );

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_200);
        let task = import_task("legacy-doc-import", root.to_str().unwrap(), now);
        store.insert_import_task(&task).unwrap();

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.files_discovered, 1);
        assert_eq!(summary.searchable_documents, 1);
        assert_eq!(summary.failed_documents, 0);
        let status = store.status_summary().unwrap();
        assert_eq!(status.searchable_documents, 1);
        let document = store.visible_documents().unwrap().remove(0);
        let versions = store.resume_versions_for_document(&document.id).unwrap();
        assert_eq!(versions.len(), 1);
        assert!(versions[0]
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Synthetic Legacy Candidate"));
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
        assert!(!format!("{summary:?}").contains(converter.to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn import_root_publishes_searchable_progress_before_full_import_completion() {
        let _env_lock = DOC_CONVERTER_ENV_LOCK.lock().unwrap();
        let temp = TestDir::new("import-pipeline-first-searchable-progress");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();

        for index in 0..32 {
            fs::write(
                root.join(format!("{index:02}-fast.txt")),
                format!(
                    "Synthetic Candidate {index}\nEmail: candidate{index}@example.test\nSkills: Rust"
                ),
            )
            .unwrap();
        }
        fs::write(root.join("zz-slow.doc"), synthetic_ole_doc()).unwrap();

        let converter = write_blocking_doc_converter(temp.path());
        let started_marker = converter.with_extension("started");
        let release_marker = converter.with_extension("release");
        let _env = EnvVarGuard::set(
            "RESUME_IR_DOC_TEXT_COMMAND",
            converter.to_str().unwrap().to_string(),
        );

        let store = MetaStore::open_data_dir(&data_dir).unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_225);
        let task = import_task(
            "first-searchable-progress-import",
            root.to_str().unwrap(),
            now,
        );
        store.insert_import_task(&task).unwrap();
        store
            .upsert_import_scan_scope(&ImportScanScope {
                import_task_id: task.id.clone(),
                root_kind: ImportRootKind::Explicit,
                root_preset: None,
                scan_profile: ImportScanProfile::Explicit,
                requested_root_path: root.to_str().unwrap().to_string(),
                canonical_root_path: root.to_str().unwrap().to_string(),
                files_discovered: 0,
                ignored_entries: 0,
                scan_errors: 0,
                searchable_documents: 0,
                ocr_required_documents: 0,
                ocr_jobs_queued: 0,
                failed_documents: 0,
                deleted_documents: 0,
                scan_budget_kind: None,
                scan_budget_limit: None,
                scan_budget_observed: None,
                scan_budget_exhausted: false,
                updated_at: now,
            })
            .unwrap();

        let data_dir_for_worker = data_dir.clone();
        let root_for_worker = root.clone();
        let task_for_worker = task.clone();
        let worker = thread::spawn(move || {
            let store = MetaStore::open_data_dir(&data_dir_for_worker).unwrap();
            store.run_migrations().unwrap();
            import_root_with_options(
                &data_dir_for_worker,
                &store,
                &task_for_worker,
                &root_for_worker,
                now,
                ImportOptions::default(),
            )
            .unwrap()
        });

        wait_for_path(&started_marker);
        let observed_store = MetaStore::open_data_dir(&data_dir).unwrap();
        observed_store.run_migrations().unwrap();
        let scope = observed_store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        let status = observed_store.status_summary().unwrap();
        let index_state_debug = format!("{:?}", observed_store.index_state().unwrap().unwrap());
        let active_snapshot_present =
            index_fulltext::FullTextIndex::open_active(&data_dir.join("search-index"))
                .unwrap()
                .is_some();

        fs::write(&release_marker, b"release").unwrap();
        let summary = worker.join().unwrap();
        let final_store = MetaStore::open_data_dir(&data_dir).unwrap();
        final_store.run_migrations().unwrap();
        let final_index_state_debug = format!("{:?}", final_store.index_state().unwrap().unwrap());

        assert_eq!(scope.files_discovered, 33);
        assert!(
            scope.searchable_documents > 0,
            "expected mid-run searchable progress before full import completion, got scope: {scope:?}"
        );
        assert!(
            status.searchable_documents > 0,
            "expected searchable documents to be visible before the final file completed, got status: {status:?}"
        );
        assert!(
            active_snapshot_present,
            "expected an active search snapshot before the final file completed"
        );
        assert!(index_state_debug.contains("visible_epoch: 5"));
        assert!(index_state_debug.contains("manifest_document_count: 32"));
        assert_eq!(summary.searchable_documents, 33);
        assert!(final_index_state_debug.contains("visible_epoch: 6"));
        assert!(final_index_state_debug.contains("manifest_document_count: 33"));
    }

    #[cfg(unix)]
    #[test]
    fn import_root_publishes_first_searchable_before_batch_threshold() {
        let _env_lock = DOC_CONVERTER_ENV_LOCK.lock().unwrap();
        let temp = TestDir::new("import-pipeline-first-searchable-early");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("00-fast.txt"),
            b"Synthetic Candidate\nEmail: fast@example.test\nSkills: Rust",
        )
        .unwrap();
        fs::write(root.join("zz-slow.doc"), synthetic_ole_doc()).unwrap();

        let converter = write_blocking_doc_converter(temp.path());
        let started_marker = converter.with_extension("started");
        let release_marker = converter.with_extension("release");
        let _env = EnvVarGuard::set(
            "RESUME_IR_DOC_TEXT_COMMAND",
            converter.to_str().unwrap().to_string(),
        );

        let store = MetaStore::open_data_dir(&data_dir).unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_230);
        let task = import_task("first-searchable-early-import", root.to_str().unwrap(), now);
        store.insert_import_task(&task).unwrap();
        store
            .upsert_import_scan_scope(&ImportScanScope {
                import_task_id: task.id.clone(),
                root_kind: ImportRootKind::Explicit,
                root_preset: None,
                scan_profile: ImportScanProfile::Explicit,
                requested_root_path: root.to_str().unwrap().to_string(),
                canonical_root_path: root.to_str().unwrap().to_string(),
                files_discovered: 0,
                ignored_entries: 0,
                scan_errors: 0,
                searchable_documents: 0,
                ocr_required_documents: 0,
                ocr_jobs_queued: 0,
                failed_documents: 0,
                deleted_documents: 0,
                scan_budget_kind: None,
                scan_budget_limit: None,
                scan_budget_observed: None,
                scan_budget_exhausted: false,
                updated_at: now,
            })
            .unwrap();

        let data_dir_for_worker = data_dir.clone();
        let root_for_worker = root.clone();
        let task_for_worker = task.clone();
        let worker = thread::spawn(move || {
            let store = MetaStore::open_data_dir(&data_dir_for_worker).unwrap();
            store.run_migrations().unwrap();
            import_root_with_options(
                &data_dir_for_worker,
                &store,
                &task_for_worker,
                &root_for_worker,
                now,
                ImportOptions::default(),
            )
            .unwrap()
        });

        wait_for_path(&started_marker);
        let observed_store = MetaStore::open_data_dir(&data_dir).unwrap();
        observed_store.run_migrations().unwrap();
        let scope = observed_store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        let status = observed_store.status_summary().unwrap();
        let index_state_debug = format!("{:?}", observed_store.index_state().unwrap().unwrap());
        let active_snapshot_present =
            index_fulltext::FullTextIndex::open_active(&data_dir.join("search-index"))
                .unwrap()
                .is_some();

        fs::write(&release_marker, b"release").unwrap();
        let summary = worker.join().unwrap();
        let final_store = MetaStore::open_data_dir(&data_dir).unwrap();
        final_store.run_migrations().unwrap();
        let final_index_state_debug = format!("{:?}", final_store.index_state().unwrap().unwrap());

        assert_eq!(scope.files_discovered, 2);
        assert!(
            scope.searchable_documents > 0,
            "expected first searchable document to publish before batch threshold, got scope: {scope:?}"
        );
        assert!(
            status.searchable_documents > 0,
            "expected searchable status before the slow file completed, got status: {status:?}"
        );
        assert!(
            active_snapshot_present,
            "expected an active search snapshot before the slow file completed"
        );
        assert!(index_state_debug.contains("visible_epoch: 1"));
        assert!(index_state_debug.contains("manifest_document_count: 1"));
        assert_eq!(summary.searchable_documents, 2);
        assert!(final_index_state_debug.contains("visible_epoch: 2"));
        assert!(final_index_state_debug.contains("manifest_document_count: 2"));
    }

    fn import_task(label: &str, root_path: &str, now: UnixTimestamp) -> ImportTask {
        ImportTask {
            id: meta_store::ImportTaskId::from_non_secret_parts(&[label]),
            root_path: root_path.to_string(),
            status: ImportTaskStatus::Running,
            queued_at: now,
            started_at: Some(now),
            finished_at: None,
            updated_at: now,
        }
    }

    fn synthetic_ole_doc() -> Vec<u8> {
        let mut bytes = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1".to_vec();
        bytes.extend_from_slice(b"SYNTHETIC PRIVATE LEGACY DOC BODY");
        bytes
    }

    fn utf16be_literal_text_layer_pdf_bytes() -> Vec<u8> {
        let mut content = b"BT /F1 12 Tf 72 720 Td (".to_vec();
        content.extend_from_slice(b"\xFE\xFF\x4E\x2D\x65\x87\x7B\x80\x53\x86");
        content.extend_from_slice(b") Tj ET\n");

        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
                content,
                b"endstream".to_vec(),
            ]
            .concat(),
        ])
    }

    fn tounicode_cmap_pdf_bytes() -> Vec<u8> {
        let cmap = br"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0001> <0004>
endcodespacerange
4 beginbfchar
<0001> <4E2D>
<0002> <6587>
<0003> <7B80>
<0004> <5386>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end
";
        let content = b"BT /F1 12 Tf 72 720 Td <0001000200030004> Tj ET\n";

        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 7 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type0 /BaseFont /TestFont /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 6 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /TestFont /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 8 0 R /DW 1000 /W [1 [1000 1000]] >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", cmap.len()).into_bytes(),
                cmap.to_vec(),
                b"endstream".to_vec(),
            ]
            .concat(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
                content.to_vec(),
                b"endstream".to_vec(),
            ]
            .concat(),
            b"<< /Type /FontDescriptor /FontName /TestFont /Flags 4 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>".to_vec(),
        ])
    }

    fn scanned_pdf_bytes() -> Vec<u8> {
        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
            b"<< /Type /XObject /Subtype /Image /Width 100 /Height 100 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 11 >>\nstream\nimage bytes\nendstream".to_vec(),
            b"<< /Length 24 >>\nstream\nq 100 0 0 100 0 0 cm /Im1 Do Q\nendstream".to_vec(),
        ])
    }

    fn build_valid_pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());

        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            if !object.ends_with(b"\n") {
                pdf.push(b'\n');
            }
            pdf.extend_from_slice(b"endobj\n");
        }

        let xref_offset = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{}\n%%EOF",
                objects.len() + 1,
                xref_offset
            )
            .as_bytes(),
        );

        pdf
    }

    #[cfg(unix)]
    fn write_doc_converter(directory: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join("fixture-doc-converter");
        fs::write(
            &path,
            r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-output" ]; then
    shift
    out="$1"
  fi
  shift
done
if [ -z "$out" ]; then
  exit 9
fi
printf 'Synthetic Legacy Candidate\nRust Search\n' > "$out"
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[cfg(unix)]
    fn write_blocking_doc_converter(directory: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join("fixture-blocking-doc-converter");
        fs::write(
            &path,
            r#"#!/bin/sh
self="$0"
started="${self%.*}.started"
release="${self%.*}.release"
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-output" ]; then
    shift
    out="$1"
  fi
  shift
done
if [ -z "$out" ]; then
  exit 9
fi
: > "$started"
while [ ! -f "$release" ]; do
  sleep 0.01
done
printf 'Slow Synthetic Legacy Candidate\nRust Search\n' > "$out"
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: String) -> Self {
            let previous = env::var(key).ok();
            env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                env::set_var(self.key, previous);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    fn wait_for_path(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if path.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {}", path.display());
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = format!(
                "{}-{}-{}",
                label,
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
