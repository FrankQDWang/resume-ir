use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use fs_crawler::DiscoveredFile;
use meta_store::{ImportTaskId, OwnedMetaStore, UnixTimestamp};
use sectionizer::Sectionizer;

use super::orchestrator::publish_import_progress;
use crate::file_processing::{
    commit_parse_work_result, process_file, ImportFileResult, ParseWorkerClock,
    PendingSearchableDocument,
};
use crate::publication_coordinator::{
    flush_pending_searchable_documents, PendingProjectionRemovals,
};
use crate::search_artifact_cache::{CurrentImportCacheMode, CurrentImportDocumentCache};
use crate::source_dispositions::{ImportDispositionBatches, ProcessedFile, SearchableStagingState};
use crate::{
    ImportCancelCheckPhase, ImportFailureKind, ImportPipelineError, ImportSummary,
    LinearPromotionPolicy, Result, SearchProjectionRemovalReason, SearchPublicationVectorization,
};

pub(super) fn commit_ready_import_file_results(
    data_dir: &Path,
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
    disposition_batches: &mut ImportDispositionBatches,
    current_import_index_documents: &mut CurrentImportDocumentCache,
    import_started: Instant,
    total_files: usize,
    pending_results: &mut BTreeMap<usize, ImportFileResult>,
    next_commit_index: &mut usize,
    parse_worker_clock: &mut ParseWorkerClock,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    index_writer_heap_bytes: usize,
    search_vectorization: &SearchPublicationVectorization,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<bool> {
    let mut committed = false;
    while let Some(result) = pending_results.remove(next_commit_index) {
        set_cancel_phase(ImportCancelCheckPhase::WorkerResultCommit);
        ensure_not_cancelled()?;
        let index = *next_commit_index;
        let result = match result {
            ImportFileResult::Processed(result) => result,
            ImportFileResult::Parsed(result) => commit_parse_work_result(
                data_dir,
                store,
                now,
                &mut summary.stage_timings.db,
                &mut summary.worker_metrics,
                parse_worker_clock,
                result,
                linear_promotion,
            )?,
        };
        finish_import_file(
            store,
            task_id,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            disposition_batches,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            search_vectorization,
            index,
            total_files,
            &result.file,
            result.processed,
        )?;
        *next_commit_index += 1;
        committed = true;
    }

    Ok(committed)
}

pub(super) fn process_files_sequential(
    data_dir: &Path,
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    files: Vec<DiscoveredFile>,
    sectionizer: &Sectionizer,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
    disposition_batches: &mut ImportDispositionBatches,
    current_import_index_documents: &mut CurrentImportDocumentCache,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    index_writer_heap_bytes: usize,
    search_vectorization: &SearchPublicationVectorization,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<()> {
    let total_files = files.len();
    for (index, file) in files.into_iter().enumerate() {
        set_cancel_phase(ImportCancelCheckPhase::SequentialParse);
        ensure_not_cancelled()?;
        let processed = process_file(
            data_dir,
            store,
            &file,
            sectionizer,
            now,
            ensure_not_cancelled,
            &mut summary.stage_timings,
            &mut summary.worker_metrics,
            &mut summary.content_bytes_read,
            linear_promotion,
        )?;
        finish_import_file(
            store,
            task_id,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            disposition_batches,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            search_vectorization,
            index,
            total_files,
            &file,
            processed,
        )?;
    }

    Ok(())
}

pub(super) fn process_indexed_files_sequential(
    data_dir: &Path,
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    files: Vec<(usize, DiscoveredFile)>,
    sectionizer: &Sectionizer,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
    disposition_batches: &mut ImportDispositionBatches,
    current_import_index_documents: &mut CurrentImportDocumentCache,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    index_writer_heap_bytes: usize,
    total_files: usize,
    search_vectorization: &SearchPublicationVectorization,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<()> {
    for (index, file) in files {
        set_cancel_phase(ImportCancelCheckPhase::SequentialParse);
        ensure_not_cancelled()?;
        let processed = process_file(
            data_dir,
            store,
            &file,
            sectionizer,
            now,
            ensure_not_cancelled,
            &mut summary.stage_timings,
            &mut summary.worker_metrics,
            &mut summary.content_bytes_read,
            linear_promotion,
        )?;
        finish_import_file(
            store,
            task_id,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            disposition_batches,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            search_vectorization,
            index,
            total_files,
            &file,
            processed,
        )?;
    }

    Ok(())
}

pub(crate) fn finish_import_file(
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
    disposition_batches: &mut ImportDispositionBatches,
    current_import_index_documents: &mut CurrentImportDocumentCache,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    index_writer_heap_bytes: usize,
    search_vectorization: &SearchPublicationVectorization,
    index: usize,
    total_files: usize,
    file: &DiscoveredFile,
    processed: ProcessedFile,
) -> Result<()> {
    let source_read_failed = matches!(
        &processed,
        ProcessedFile::Failed {
            kind: ImportFailureKind::ReadError,
            ..
        }
    );
    let disposition_staging = processed.disposition_staging();
    let disposition = (!source_read_failed)
        .then(|| processed.source_disposition(index, &file.document_id))
        .transpose()?;
    match processed {
        ProcessedFile::Searchable { pending } => {
            pending_index_documents.push(*pending);
        }
        ProcessedFile::OcrRequired { ocr_job_queued, .. } => {
            summary.ocr_required_documents += 1;
            if ocr_job_queued {
                summary.ocr_jobs_queued += 1;
            }
        }
        ProcessedFile::Failed { kind, .. } => {
            summary.failed_documents += 1;
            summary.failure_counts.increment(kind);
        }
        ProcessedFile::Excluded { document, .. } => {
            pending_excluded_doc_ids.schedule(
                file.document_id.clone(),
                SearchProjectionRemovalReason::PermanentClassificationExclusion,
                Some(*document),
            )?;
        }
        ProcessedFile::UnchangedExcluded { .. } => {
            pending_excluded_doc_ids.schedule(
                file.document_id.clone(),
                SearchProjectionRemovalReason::PermanentClassificationExclusion,
                None,
            )?;
        }
        ProcessedFile::UnchangedOcrRequired { .. } => {}
        ProcessedFile::UnchangedSearchable { .. } => {
            summary.searchable_documents += 1;
        }
    }

    if source_read_failed {
        set_cancel_phase(ImportCancelCheckPhase::DbWrite);
        let progress_started = Instant::now();
        publish_import_progress(store, task_id, summary, now)?;
        summary.stage_timings.db += progress_started.elapsed();
        return Err(ImportPipelineError::migration_scan_incomplete());
    }

    disposition_batches.record(
        disposition.ok_or_else(ImportPipelineError::store_invariant)?,
        disposition_staging,
    );

    let flushed_searchables = if should_flush_searchable_documents(
        index,
        total_files,
        pending_index_documents.len(),
        summary.searchable_documents,
    ) {
        flush_pending_searchable_documents(
            store,
            now,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            Some(current_import_index_documents),
            CurrentImportCacheMode::Retain,
            ensure_not_cancelled,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            search_vectorization,
        )?
    } else {
        false
    };
    disposition_batches.searchable_staging_completed(
        SearchableStagingState::from_pending_documents(pending_index_documents),
        store,
    )?;
    disposition_batches.flush_ready_if_full(store)?;
    if flushed_searchables || should_publish_import_progress(index, total_files) {
        set_cancel_phase(ImportCancelCheckPhase::DbWrite);
        let progress_started = Instant::now();
        publish_import_progress(store, task_id, summary, now)?;
        summary.stage_timings.db += progress_started.elapsed();
    }

    Ok(())
}

const IMPORT_PROGRESS_UPDATE_EVERY_FILES: usize = 32;
const IMPORT_SEARCHABLE_FLUSH_BATCH: usize = 1024;
const IMPORT_SEARCHABLE_FLUSH_MILESTONES: [usize; 2] = [100, 1000];

fn should_publish_import_progress(index: usize, total: usize) -> bool {
    let processed = index + 1;
    processed == total || processed.is_multiple_of(IMPORT_PROGRESS_UPDATE_EVERY_FILES)
}

pub(crate) fn should_flush_searchable_documents(
    index: usize,
    total: usize,
    pending_searchable_documents: usize,
    searchable_documents: usize,
) -> bool {
    let processed = index + 1;
    (processed < total && searchable_documents == 0 && pending_searchable_documents > 0)
        || crosses_searchable_flush_milestone(pending_searchable_documents, searchable_documents)
        || pending_searchable_documents >= IMPORT_SEARCHABLE_FLUSH_BATCH
}

fn crosses_searchable_flush_milestone(
    pending_searchable_documents: usize,
    searchable_documents: usize,
) -> bool {
    let projected_searchable = searchable_documents.saturating_add(pending_searchable_documents);
    IMPORT_SEARCHABLE_FLUSH_MILESTONES
        .iter()
        .any(|milestone| searchable_documents < *milestone && projected_searchable >= *milestone)
}
