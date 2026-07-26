use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fs_crawler::{
    crawl_directory_with_options_and_control, CrawlErrorKind, ScanControl, ScanOptions,
};
use meta_store::{
    FileExtension, ImportProcessingContract, ImportScanBudgetKind as StoreImportScanBudgetKind,
    ImportTask, ImportTaskId, OwnedMetaStore, UnixTimestamp,
};
use sectionizer::Sectionizer;

use super::parallel::process_files_with_parse_workers;
use super::scan::{import_scan_errors_from_crawl, mark_missing_documents_deleted};
use super::scheduler::process_files_sequential;
use crate::migration_rebuild::ensure_migration_rebuild_scan_is_complete;
use crate::publication_coordinator::{
    flush_pending_searchable_documents, PendingProjectionRemovals,
};
use crate::search_artifact_cache::{CurrentImportCacheMode, CurrentImportDocumentCache};
use crate::source_dispositions::{ImportDispositionBatches, SearchableStagingState};
use crate::timing::measure_result_stage;
use crate::PipelineRunControl;
use crate::{
    ImportCancelCheckPhase, ImportFailureCounts, ImportMilestoneTimings, ImportOptions,
    ImportPipelineError, ImportScanBudget, ImportScanBudgetKind, ImportStageTimings, ImportSummary,
    ImportWorkerMetrics, PdfImportPolicy, Result, SearchProjectionRemovalReason,
    IMPORT_CANCEL_POLL_INTERVAL_MS,
};

pub(super) fn run_import(
    data_dir: &Path,
    store: &OwnedMetaStore,
    task: &ImportTask,
    root: &Path,
    now: UnixTimestamp,
    options: ImportOptions,
    processing_contract: &ImportProcessingContract,
    control: &PipelineRunControl,
) -> Result<ImportSummary> {
    ensure_import_can_continue(store, &task.id, control)?;
    let cancel_metrics = RefCell::new(CancelCheckMetrics::default());
    let cancel_poller = RefCell::new(ImportCancelPoller::new(Duration::from_millis(
        IMPORT_CANCEL_POLL_INTERVAL_MS,
    )));
    let cancel_phase = Cell::new(ImportCancelCheckPhase::ImportSetup);
    let poll_cancelled = || {
        cancel_poller.borrow_mut().poll(Instant::now(), || {
            store
                .is_import_task_cancelled(&task.id)
                .map_err(ImportPipelineError::store)
        })
    };
    let cancel_check = || {
        cancel_metrics.borrow_mut().record_check(cancel_phase.get());
        poll_cancelled()
            .map(|cancelled| cancelled || control.shutdown_requested())
            .unwrap_or(true)
    };
    let ensure_not_cancelled = || {
        cancel_metrics.borrow_mut().record_check(cancel_phase.get());
        if poll_cancelled()? {
            Err(ImportPipelineError::cancelled())
        } else if control.shutdown_requested() {
            Err(ImportPipelineError::interrupted())
        } else {
            Ok(())
        }
    };
    let set_cancel_phase = |phase| cancel_phase.set(phase);
    let import_started = Instant::now();
    let scan_started = Instant::now();
    set_cancel_phase(ImportCancelCheckPhase::Scan);
    let report = crawl_directory_with_options_and_control(
        root,
        ScanOptions {
            profile: options.scan_profile,
            max_files: options.max_files,
        },
        ScanControl::from_cancel_check(&cancel_check),
    );
    let mut report = match report {
        Ok(report) => report,
        Err(error) if error.kind == CrawlErrorKind::Cancelled => {
            ensure_not_cancelled()?;
            return Err(ImportPipelineError::crawl(error));
        }
        Err(error) => return Err(ImportPipelineError::crawl(error)),
    };
    let existing_occurrences = store
        .source_occurrence_documents_for_import_task(&task.id)
        .map_err(ImportPipelineError::store)?;
    if !existing_occurrences.is_empty() {
        for file in &mut report.files {
            let relative_path = Path::new(file.normalized_path.as_str())
                .strip_prefix(root)
                .map_err(|_| ImportPipelineError::store_invariant())?
                .to_string_lossy()
                .replace('\\', "/");
            if let Some(existing_document_id) = existing_occurrences.get(&relative_path) {
                file.document_id = existing_document_id.clone();
            }
        }
    }
    let scan_elapsed = scan_started.elapsed();
    ensure_not_cancelled()?;
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
        processed_documents: 0,
        deferred_pdf_documents: 0,
        source_truth_complete: false,
        scan_errors: report.errors.len(),
        ignored_entries: report.ignored_count,
        content_bytes_read: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        failure_counts: ImportFailureCounts::default(),
        deleted_documents: 0,
        scan_budget,
        stage_timings: ImportStageTimings {
            scan: scan_elapsed,
            ..ImportStageTimings::default()
        },
        milestone_timings: ImportMilestoneTimings::default(),
        worker_metrics: ImportWorkerMetrics::default(),
    };
    set_cancel_phase(ImportCancelCheckPhase::DbWrite);
    measure_result_stage(&mut summary.stage_timings.db, || {
        store.replace_import_scan_errors(&task.id, &scan_errors)
    })
    .map_err(ImportPipelineError::store)?;
    let progress_started = Instant::now();
    publish_import_progress(store, &task.id, &summary, now)?;
    summary.stage_timings.db += progress_started.elapsed();
    ensure_migration_rebuild_scan_is_complete(
        store,
        !report.errors.is_empty(),
        scan_budget_exhausted.is_some(),
    )?;
    ensure_not_cancelled()?;
    let mut pending_index_documents = Vec::new();
    let mut disposition_batches =
        ImportDispositionBatches::new(task.id.clone(), processing_contract.id().clone());
    let sectionizer = Sectionizer::default();
    let can_propagate_deletions = report.errors.is_empty() && scan_budget_exhausted.is_none();
    let discovered_doc_ids = report
        .files
        .iter()
        .map(|file| file.document_id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let mut pending_excluded_doc_ids = PendingProjectionRemovals::default();

    let (files_to_process, frozen_pdf_count) = match options.pdf_import {
        PdfImportPolicy::Enabled => (report.files, 0),
        PdfImportPolicy::Frozen => {
            let (pdfs, other): (Vec<_>, Vec<_>) = report
                .files
                .into_iter()
                .partition(|file| file.extension == FileExtension::Pdf);
            for pdf in &pdfs {
                store
                    .observe_existing_import_task_source_occurrence(
                        &task.id,
                        pdf.normalized_path.as_str(),
                        now,
                    )
                    .map_err(ImportPipelineError::store)?;
            }
            (other, pdfs.len())
        }
    };
    summary.ignored_entries = summary.ignored_entries.saturating_add(frozen_pdf_count);
    summary.processed_documents = summary.processed_documents.saturating_add(frozen_pdf_count);
    summary.deferred_pdf_documents = frozen_pdf_count;
    summary.source_truth_complete = can_propagate_deletions;
    let total_files = files_to_process.len();
    let mut current_import_index_documents = CurrentImportDocumentCache::default();
    if options.parse_workers.get() > 1 && total_files > 1 {
        process_files_with_parse_workers(
            data_dir,
            store,
            &task.id,
            files_to_process,
            now,
            &ensure_not_cancelled,
            &mut summary,
            &mut pending_index_documents,
            &mut pending_excluded_doc_ids,
            &mut disposition_batches,
            &mut current_import_index_documents,
            &set_cancel_phase,
            import_started,
            options.parse_workers,
            options.index_writer_heap_bytes,
            &options.search_vectorization,
            &options.linear_promotion,
        )?;
    } else {
        process_files_sequential(
            data_dir,
            store,
            &task.id,
            files_to_process,
            &sectionizer,
            now,
            &ensure_not_cancelled,
            &mut summary,
            &mut pending_index_documents,
            &mut pending_excluded_doc_ids,
            &mut disposition_batches,
            &mut current_import_index_documents,
            &set_cancel_phase,
            import_started,
            options.index_writer_heap_bytes,
            &options.search_vectorization,
            &options.linear_promotion,
        )?;
    }

    if summary.source_truth_complete {
        set_cancel_phase(ImportCancelCheckPhase::DbWrite);
        ensure_not_cancelled()?;
        let deleted_documents = measure_result_stage(&mut summary.stage_timings.db, || {
            mark_missing_documents_deleted(
                store,
                root,
                options.scan_profile,
                &scanned_directories,
                &skipped_directories,
                &discovered_doc_ids,
                now,
            )
        })?;
        summary.deleted_documents = deleted_documents.len();
        for document in deleted_documents {
            pending_excluded_doc_ids.schedule(
                document.id.clone(),
                SearchProjectionRemovalReason::ConfirmedSourceDeletion,
                Some(document),
            )?;
        }
        let progress_started = Instant::now();
        publish_import_progress(store, &task.id, &summary, now)?;
        summary.stage_timings.db += progress_started.elapsed();
    } else {
        summary.deleted_documents = 0;
    }
    set_cancel_phase(ImportCancelCheckPhase::IndexPublication);
    flush_pending_searchable_documents(
        store,
        now,
        &mut summary,
        &mut pending_index_documents,
        &mut pending_excluded_doc_ids,
        Some(&mut current_import_index_documents),
        CurrentImportCacheMode::Consume,
        &ensure_not_cancelled,
        &set_cancel_phase,
        import_started,
        options.index_writer_heap_bytes,
        &options.search_vectorization,
    )?;
    disposition_batches.searchable_staging_completed(
        SearchableStagingState::from_pending_documents(&pending_index_documents),
        store,
    )?;
    disposition_batches.flush_all(store)?;
    summary.milestone_timings.full_import_ready = Some(import_started.elapsed());
    if summary.milestone_timings.full_index_ready.is_none() {
        summary.milestone_timings.full_index_ready = Some(import_started.elapsed());
    }
    set_cancel_phase(ImportCancelCheckPhase::DbWrite);
    let progress_started = Instant::now();
    publish_import_progress(store, &task.id, &summary, now)?;
    summary.stage_timings.db += progress_started.elapsed();
    let cancel_metrics = cancel_metrics.into_inner();
    summary.worker_metrics.record_cancel_checks(
        cancel_metrics.count,
        cancel_metrics.max_gap,
        cancel_metrics.max_gap_phase,
    );

    Ok(summary)
}

pub(super) fn publish_import_progress(
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    summary: &ImportSummary,
    updated_at: UnixTimestamp,
) -> Result<()> {
    let Some(scope) = import_scan_scope_from_summary(store, task_id, summary, updated_at)? else {
        return Ok(());
    };

    store
        .upsert_import_scan_scope(&scope)
        .and_then(|()| {
            store.update_source_scan_progress_for_import_task(
                task_id,
                meta_store::SourceScanProgress {
                    discovered: summary.files_discovered as u64,
                    searchable: summary.searchable_documents as u64,
                    ocr: summary.ocr_required_documents as u64,
                    failed: summary.failed_documents as u64,
                    ignored: summary.ignored_entries as u64,
                    processed: summary.processed_documents as u64,
                    errors: summary.scan_errors as u64,
                },
                scope.updated_at,
            )
        })
        .map_err(ImportPipelineError::store)
}

pub(super) fn import_scan_scope_from_summary(
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    summary: &ImportSummary,
    updated_at: UnixTimestamp,
) -> Result<Option<meta_store::ImportScanScope>> {
    let Some(mut scope) = store
        .import_scan_scope_by_task_id(task_id)
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(None);
    };

    // A frozen PDF remains part of the authoritative directory scan and is
    // reported through ScanSnapshot, but it was deliberately not executed
    // under this processing contract and therefore cannot appear in the
    // sealed source-disposition manifest.
    scope.files_discovered = sealed_source_disposition_count(summary) as u64;
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
    Ok(Some(scope))
}

fn sealed_source_disposition_count(summary: &ImportSummary) -> usize {
    summary
        .files_discovered
        .saturating_sub(summary.deferred_pdf_documents)
}

fn ensure_import_not_cancelled(store: &OwnedMetaStore, task_id: &ImportTaskId) -> Result<()> {
    if store
        .is_import_task_cancelled(task_id)
        .map_err(ImportPipelineError::store)?
    {
        Err(ImportPipelineError::cancelled())
    } else {
        Ok(())
    }
}

fn ensure_import_can_continue(
    store: &OwnedMetaStore,
    task_id: &ImportTaskId,
    control: &PipelineRunControl,
) -> Result<()> {
    ensure_import_not_cancelled(store, task_id)?;
    if control.shutdown_requested() {
        Err(ImportPipelineError::interrupted())
    } else {
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct ImportCancelPoller {
    min_interval: Duration,
    last_probe: Option<Instant>,
    cached_cancelled: bool,
}

#[cfg(test)]
#[path = "orchestrator_tests.rs"]
mod tests;

impl ImportCancelPoller {
    pub(crate) fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_probe: None,
            cached_cancelled: false,
        }
    }

    pub(crate) fn poll(
        &mut self,
        now: Instant,
        probe: impl FnOnce() -> Result<bool>,
    ) -> Result<bool> {
        if self.cached_cancelled {
            return Ok(true);
        }

        if self.should_probe(now) {
            self.cached_cancelled = probe()?;
            self.last_probe = Some(now);
        }
        Ok(self.cached_cancelled)
    }

    fn should_probe(&self, now: Instant) -> bool {
        match self.last_probe {
            Some(last_probe) => now
                .checked_duration_since(last_probe)
                .is_none_or(|elapsed| elapsed >= self.min_interval),
            None => true,
        }
    }
}

pub(crate) fn current_timestamp_or(default: UnixTimestamp) -> UnixTimestamp {
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

#[derive(Debug, Default)]
pub(crate) struct CancelCheckMetrics {
    pub(crate) count: usize,
    previous_check: Option<Instant>,
    previous_phase: ImportCancelCheckPhase,
    pub(crate) max_gap: Duration,
    pub(crate) max_gap_phase: ImportCancelCheckPhase,
}

impl CancelCheckMetrics {
    pub(crate) fn record_check(&mut self, phase: ImportCancelCheckPhase) {
        let now = Instant::now();
        self.count += 1;
        if let Some(previous_check) = self.previous_check {
            let gap = now.duration_since(previous_check);
            if gap > self.max_gap {
                self.max_gap = gap;
                self.max_gap_phase = self.previous_phase;
            }
        }
        self.previous_check = Some(now);
        self.previous_phase = phase;
    }
}
