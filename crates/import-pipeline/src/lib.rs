// Import orchestration passes stage state explicitly; split this before tightening
// these shape lints for the crate.
#![allow(clippy::too_many_arguments, clippy::large_enum_variant)]

mod classification;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use core_domain::{EntityMentionId, SectionType};
use extractor_rules::{extract_strong_fields, FieldType, RuleMatch};
pub use fs_crawler::ScanProfile;
use fs_crawler::{
    crawl_directory_with_options_and_control, normalize_path, CrawlError, CrawlErrorKind,
    DiscoveredFile, FsOperation, NormalizedPath, ScanBudgetKind, ScanControl, ScanOptions,
};
use index_fulltext::{
    incremental_snapshot_documents, publish_snapshot_with_control,
    publish_trusted_redacted_snapshot_with_control, redact_contact_values, IndexDocument,
    IndexSection, SnapshotPublishControl, SnapshotPublishPhase,
};
use meta_store::{
    ClaimedOcrJob, ClassificationStatus, ContactHash, Document, DocumentId, DocumentStatus,
    EntityMention, EntityType, FileExtension, ImportScanBudgetKind as StoreImportScanBudgetKind,
    ImportScanError, ImportScanErrorKind, ImportScanErrorOperation, ImportTask, ImportTaskId,
    ImportTaskStatus, IndexState, IndexStateStatus, IngestJob, IngestJobStatus, MetaStore,
    OcrAttemptPublication, OcrAttemptSuccessOutcome, ResumeVersion, ResumeVersionId,
    ResumeVisibility, SearchableImportDocument, UnixTimestamp,
};
use parser_common::{ParseInput, ParseStatus, Parser, ParserErrorKind, ResourceBudget};
use parser_doc::DocParser;
use parser_docx::DocxParser;
use parser_pdf::{PdfParser, PdfTextExtractionTimings};
use parser_text::TxtParser;
use privacy::{ContactHasher, ContactKind};
pub use resume_classifier::LinearPromotionPolicy;
use sectionizer::{SectionChunk, Sectionizer};
use sysinfo::System;
use text_normalizer::TextNormalizer;

use classification::{is_current, AdmissionDecision};

const PARSE_VERSION: &str = "parser-v1";
const OCR_PARSE_VERSION: &str = "ocr-v1";
const SCHEMA_VERSION: &str = "resume-ir-s9-v1";
const INDEX_MANIFEST_VERSION: &str = "fulltext-s9-v1";
const IMPORT_TASK_OWNER_LOCKS_DIR: &str = "import-task-locks";
const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;
const MAX_IMPORT_PARSE_WORKERS: usize = 3;
const IMPORT_CANCEL_POLL_INTERVAL_MS: u64 = 25;
const PARSE_RESULT_CANCEL_POLL_INTERVAL_MS: u64 = 50;
const H0_MAX_PRIVATE_OR_ANONYMOUS_MB: u16 = 512;
const H1_MAX_PRIVATE_OR_ANONYMOUS_MB: u16 = 1024;
const H2_MAX_PRIVATE_OR_ANONYMOUS_MB: u16 = 1536;
const H0_INDEX_WRITER_HEAP_BYTES: usize = 64 * 1024 * 1024;
const H1_INDEX_WRITER_HEAP_BYTES: usize = 128 * 1024 * 1024;
const H2_INDEX_WRITER_HEAP_BYTES: usize = 256 * 1024 * 1024;
const H0_MEMORY_CEILING_BYTES: u64 = 8 * BYTES_PER_GIB;
const H1_MEMORY_CEILING_BYTES: u64 = 24 * BYTES_PER_GIB;

pub fn crate_name() -> &'static str {
    "import-pipeline"
}

pub type Result<T> = std::result::Result<T, ImportPipelineError>;

pub fn import_task_owner_lock_path(data_dir: &Path, task_id: &ImportTaskId) -> PathBuf {
    data_dir
        .join(IMPORT_TASK_OWNER_LOCKS_DIR)
        .join(format!("{}.lock", task_id))
}

pub struct ImportTaskOwnerLock {
    file: File,
}

impl ImportTaskOwnerLock {
    pub fn acquire(data_dir: &Path, task_id: &ImportTaskId) -> std::io::Result<Self> {
        let path = import_task_owner_lock_path(data_dir, task_id);
        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid import task owner lock",
            )
        })?;
        fs::create_dir_all(parent)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.lock()?;
        Ok(Self { file })
    }

    pub fn try_acquire(data_dir: &Path, task_id: &ImportTaskId) -> std::io::Result<Option<Self>> {
        let path = import_task_owner_lock_path(data_dir, task_id);
        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid import task owner lock",
            )
        })?;
        fs::create_dir_all(parent)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(error),
        }
    }
}

impl Drop for ImportTaskOwnerLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

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
        poll_cancelled().unwrap_or(true)
    };
    let ensure_not_cancelled = || {
        cancel_metrics.borrow_mut().record_check(cancel_phase.get());
        if poll_cancelled()? {
            Err(ImportPipelineError::cancelled())
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
    )
    .map_err(ImportPipelineError::crawl)?;
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
    ensure_not_cancelled()?;
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
    let mut current_import_index_documents = CurrentImportIndexDocuments::default();
    if options.parse_workers.get() > 1 && total_files > 1 {
        process_files_with_parse_workers(
            data_dir,
            store,
            &task.id,
            report.files,
            now,
            &ensure_not_cancelled,
            &mut summary,
            &mut pending_index_documents,
            &mut pending_excluded_doc_ids,
            &mut current_import_index_documents,
            &set_cancel_phase,
            import_started,
            options.parse_workers,
            options.index_writer_heap_bytes,
            &options.linear_promotion,
        )?;
    } else {
        process_files_sequential(
            data_dir,
            store,
            &task.id,
            report.files,
            &sectionizer,
            now,
            &ensure_not_cancelled,
            &mut summary,
            &mut pending_index_documents,
            &mut pending_excluded_doc_ids,
            &mut current_import_index_documents,
            &set_cancel_phase,
            import_started,
            options.index_writer_heap_bytes,
            &options.linear_promotion,
        )?;
    }

    if can_propagate_deletions {
        set_cancel_phase(ImportCancelCheckPhase::DbWrite);
        ensure_not_cancelled()?;
        let deleted_document_ids = measure_result_stage(&mut summary.stage_timings.db, || {
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
        summary.deleted_documents = deleted_document_ids.len();
        pending_excluded_doc_ids.extend(deleted_document_ids);
        let progress_started = Instant::now();
        publish_import_progress(store, &task.id, &summary, now)?;
        summary.stage_timings.db += progress_started.elapsed();
    } else {
        summary.deleted_documents = 0;
    }
    set_cancel_phase(ImportCancelCheckPhase::IndexPublication);
    flush_pending_searchable_documents(
        data_dir,
        store,
        now,
        &mut summary,
        &mut pending_index_documents,
        &mut pending_excluded_doc_ids,
        Some(&mut current_import_index_documents),
        CurrentImportIndexCacheMode::Consume,
        &ensure_not_cancelled,
        &set_cancel_phase,
        import_started,
        options.index_writer_heap_bytes,
    )?;
    summary.milestone_timings.full_import_ready = Some(import_started.elapsed());
    if summary.milestone_timings.full_index_ready.is_none() {
        summary.milestone_timings.full_index_ready = Some(import_started.elapsed());
    }
    set_cancel_phase(ImportCancelCheckPhase::DbWrite);
    let progress_started = Instant::now();
    publish_import_progress(store, &task.id, &summary, now)?;
    summary.stage_timings.db += progress_started.elapsed();
    summary
        .worker_metrics
        .record_cancel_checks(cancel_metrics.into_inner());

    Ok(summary)
}

fn process_files_sequential(
    data_dir: &Path,
    store: &MetaStore,
    task_id: &ImportTaskId,
    files: Vec<DiscoveredFile>,
    sectionizer: &Sectionizer,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<(Document, IndexDocument)>,
    pending_excluded_doc_ids: &mut BTreeSet<String>,
    current_import_index_documents: &mut CurrentImportIndexDocuments,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    index_writer_heap_bytes: usize,
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
            data_dir,
            store,
            task_id,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            index,
            total_files,
            &file,
            processed,
        )?;
    }

    Ok(())
}

fn process_files_with_parse_workers(
    data_dir: &Path,
    store: &MetaStore,
    task_id: &ImportTaskId,
    files: Vec<DiscoveredFile>,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<(Document, IndexDocument)>,
    pending_excluded_doc_ids: &mut BTreeSet<String>,
    current_import_index_documents: &mut CurrentImportIndexDocuments,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    parse_workers: ImportParseWorkers,
    index_writer_heap_bytes: usize,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<()> {
    let total_files = files.len();
    let worker_count = parse_workers.get().min(total_files);
    if worker_count <= 1 {
        summary
            .worker_metrics
            .record_parse_worker_count(worker_count);
        let sectionizer = Sectionizer::default();
        return process_files_sequential(
            data_dir,
            store,
            task_id,
            files,
            &sectionizer,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            linear_promotion,
        );
    }

    let sectionizer = Sectionizer::default();
    let mut remaining_files = Vec::new();
    let mut indexed_files = files.into_iter().enumerate();
    for (index, file) in &mut indexed_files {
        if summary.searchable_documents > 0 {
            remaining_files.push((index, file));
            break;
        }
        set_cancel_phase(ImportCancelCheckPhase::SequentialParse);
        ensure_not_cancelled()?;
        let processed = process_file(
            data_dir,
            store,
            &file,
            &sectionizer,
            now,
            ensure_not_cancelled,
            &mut summary.stage_timings,
            &mut summary.worker_metrics,
            &mut summary.content_bytes_read,
            linear_promotion,
        )?;
        finish_import_file(
            data_dir,
            store,
            task_id,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            index,
            total_files,
            &file,
            processed,
        )?;
    }
    remaining_files.extend(indexed_files);
    if remaining_files.is_empty() {
        summary.worker_metrics.record_parse_worker_count(1);
        return Ok(());
    }

    let worker_count = parse_workers.get().min(remaining_files.len());
    summary
        .worker_metrics
        .record_parse_worker_count(worker_count);
    if worker_count <= 1 {
        return process_indexed_files_sequential(
            data_dir,
            store,
            task_id,
            remaining_files,
            &sectionizer,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            total_files,
            linear_promotion,
        );
    }

    let mut parse_worker_clock = ParseWorkerClock::default();
    thread::scope(|scope| -> Result<()> {
        let (work_tx, work_rx) = mpsc::sync_channel::<ParseWorkItem>(worker_count);
        let work_rx = Arc::new(Mutex::new(work_rx));
        let (result_tx, result_rx) = mpsc::sync_channel::<ParseWorkResult>(worker_count);

        for _ in 0..worker_count {
            let work_rx = Arc::clone(&work_rx);
            let result_tx = result_tx.clone();
            let linear_promotion = linear_promotion.clone();
            scope.spawn(move || parse_worker_loop(work_rx, result_tx, &linear_promotion));
        }
        drop(result_tx);

        let mut pending_results = BTreeMap::<usize, ImportFileResult>::new();
        let mut next_commit_index = remaining_files[0].0;

        for (index, file) in remaining_files.into_iter() {
            set_cancel_phase(ImportCancelCheckPhase::WorkerResultCommit);
            drain_available_parse_results(&result_rx, &mut pending_results)?;
            commit_ready_import_file_results(
                data_dir,
                store,
                task_id,
                now,
                ensure_not_cancelled,
                summary,
                pending_index_documents,
                pending_excluded_doc_ids,
                current_import_index_documents,
                import_started,
                total_files,
                &mut pending_results,
                &mut next_commit_index,
                &mut parse_worker_clock,
                set_cancel_phase,
                index_writer_heap_bytes,
            )?;

            set_cancel_phase(ImportCancelCheckPhase::ParsePrepare);
            let prepared = prepare_file_for_parse(
                store,
                index,
                file,
                now,
                ensure_not_cancelled,
                &mut summary.stage_timings.db,
                &mut summary.worker_metrics.parse_prepare,
                &mut summary.content_bytes_read,
                linear_promotion,
            )?;
            match prepared {
                PreparedFile::Ready(result) => {
                    insert_import_file_result(
                        &mut pending_results,
                        index,
                        ImportFileResult::Processed(result),
                    )?;
                }
                PreparedFile::Parse(work) => {
                    send_parse_work_with_backpressure(
                        &work_tx,
                        &result_rx,
                        &mut pending_results,
                        &mut summary.worker_metrics,
                        ensure_not_cancelled,
                        set_cancel_phase,
                        work,
                    )?;
                }
            }

            set_cancel_phase(ImportCancelCheckPhase::WorkerResultCommit);
            drain_available_parse_results(&result_rx, &mut pending_results)?;
            commit_ready_import_file_results(
                data_dir,
                store,
                task_id,
                now,
                ensure_not_cancelled,
                summary,
                pending_index_documents,
                pending_excluded_doc_ids,
                current_import_index_documents,
                import_started,
                total_files,
                &mut pending_results,
                &mut next_commit_index,
                &mut parse_worker_clock,
                set_cancel_phase,
                index_writer_heap_bytes,
            )?;
        }

        drop(work_tx);
        while next_commit_index < total_files {
            set_cancel_phase(ImportCancelCheckPhase::WorkerResultCommit);
            if commit_ready_import_file_results(
                data_dir,
                store,
                task_id,
                now,
                ensure_not_cancelled,
                summary,
                pending_index_documents,
                pending_excluded_doc_ids,
                current_import_index_documents,
                import_started,
                total_files,
                &mut pending_results,
                &mut next_commit_index,
                &mut parse_worker_clock,
                set_cancel_phase,
                index_writer_heap_bytes,
            )? {
                continue;
            }

            set_cancel_phase(ImportCancelCheckPhase::ParseResultWait);
            let wait_started = Instant::now();
            let result = recv_parse_result_with_cancel_poll(&result_rx, ensure_not_cancelled)?;
            summary.worker_metrics.parse_result_wait += wait_started.elapsed();
            insert_parse_result(&mut pending_results, result)?;
        }

        Ok(())
    })?;

    summary
        .worker_metrics
        .record_parse_worker_clock(&parse_worker_clock);
    summary.stage_timings.parse += parse_worker_clock.worker_wall_elapsed();

    Ok(())
}

fn process_indexed_files_sequential(
    data_dir: &Path,
    store: &MetaStore,
    task_id: &ImportTaskId,
    files: Vec<(usize, DiscoveredFile)>,
    sectionizer: &Sectionizer,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<(Document, IndexDocument)>,
    pending_excluded_doc_ids: &mut BTreeSet<String>,
    current_import_index_documents: &mut CurrentImportIndexDocuments,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    index_writer_heap_bytes: usize,
    total_files: usize,
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
            data_dir,
            store,
            task_id,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
            index,
            total_files,
            &file,
            processed,
        )?;
    }

    Ok(())
}

fn finish_import_file(
    data_dir: &Path,
    store: &MetaStore,
    task_id: &ImportTaskId,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<(Document, IndexDocument)>,
    pending_excluded_doc_ids: &mut BTreeSet<String>,
    current_import_index_documents: &mut CurrentImportIndexDocuments,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    index_writer_heap_bytes: usize,
    index: usize,
    total_files: usize,
    file: &DiscoveredFile,
    processed: ProcessedFile,
) -> Result<()> {
    match processed {
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
        ProcessedFile::Excluded
        | ProcessedFile::UnchangedExcluded
        | ProcessedFile::UnchangedOcrRequired => {
            pending_excluded_doc_ids.insert(file.document_id.as_str().to_string());
        }
        ProcessedFile::UnchangedSearchable => {
            summary.searchable_documents += 1;
        }
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
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            Some(current_import_index_documents),
            CurrentImportIndexCacheMode::Retain,
            ensure_not_cancelled,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
        )?
    } else {
        false
    };
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

fn should_flush_searchable_documents(
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
    current_import_index_documents: Option<&mut CurrentImportIndexDocuments>,
    current_import_index_cache_mode: CurrentImportIndexCacheMode,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    index_writer_heap_bytes: usize,
) -> Result<bool> {
    if pending_index_documents.is_empty() && pending_excluded_doc_ids.is_empty() {
        return Ok(false);
    }

    set_cancel_phase(ImportCancelCheckPhase::IndexPublication);
    ensure_not_cancelled()?;
    let searchable_before = summary.searchable_documents;
    let (pending_documents, pending_replacements) =
        take_pending_searchable_documents(pending_index_documents);
    let phase_worker_metrics = RefCell::new(ImportWorkerMetrics::default());
    let record_phase_timing = |phase, elapsed| {
        phase_worker_metrics
            .borrow_mut()
            .record_index_publication_phase_timing(phase, elapsed);
    };
    let index_started = Instant::now();
    let write_result = write_incremental_full_text_index(
        data_dir,
        store,
        now,
        pending_replacements,
        pending_excluded_doc_ids,
        summary.ocr_required_documents,
        summary.deleted_documents,
        current_import_index_documents,
        current_import_index_cache_mode,
        Some(ensure_not_cancelled),
        Some(set_cancel_phase),
        Some(&record_phase_timing),
        index_writer_heap_bytes,
    );
    summary.stage_timings.index += index_started.elapsed();
    summary
        .worker_metrics
        .add_assign(&phase_worker_metrics.into_inner());
    let (snapshot_token, indexed_document_count) = write_result?;

    set_cancel_phase(ImportCancelCheckPhase::DbWrite);
    for mut document in pending_documents {
        ensure_not_cancelled()?;
        document.status = DocumentStatus::Searchable;
        document.updated_at = now;
        measure_result_stage(&mut summary.stage_timings.db, || {
            store.upsert_document(&document)
        })
        .map_err(ImportPipelineError::store)?;
        summary.searchable_documents += 1;
    }

    pending_excluded_doc_ids.clear();
    set_cancel_phase(ImportCancelCheckPhase::DbWrite);
    ensure_not_cancelled()?;
    measure_result_stage(&mut summary.stage_timings.db, || {
        update_index_state(store, now, snapshot_token, indexed_document_count)
    })?;
    let index_ready_elapsed = import_started.elapsed();
    record_searchable_milestones(
        &mut summary.milestone_timings,
        searchable_before,
        summary.searchable_documents,
        index_ready_elapsed,
    );
    Ok(true)
}

fn take_pending_searchable_documents(
    pending_index_documents: &mut Vec<(Document, IndexDocument)>,
) -> (Vec<Document>, Vec<IndexDocument>) {
    let pending = std::mem::take(pending_index_documents);
    let mut documents = Vec::with_capacity(pending.len());
    let mut index_documents = Vec::with_capacity(pending.len());
    for (document, index_document) in pending {
        documents.push(document);
        index_documents.push(index_document);
    }
    (documents, index_documents)
}

fn record_searchable_milestones(
    milestones: &mut ImportMilestoneTimings,
    searchable_before: usize,
    searchable_after: usize,
    elapsed: Duration,
) {
    milestones.full_index_ready = Some(elapsed);
    if searchable_before == searchable_after {
        return;
    }
    if milestones.first_searchable.is_none() && searchable_after > 0 {
        milestones.first_searchable = Some(elapsed);
    }
    if milestones.ttf100_searchable.is_none() && searchable_before < 100 && searchable_after >= 100
    {
        milestones.ttf100_searchable = Some(elapsed);
    }
    if milestones.ttf1000_searchable.is_none()
        && searchable_before < 1000
        && searchable_after >= 1000
    {
        milestones.ttf1000_searchable = Some(elapsed);
    }
}

fn measure_result_stage<T, E>(
    stage: &mut Duration,
    operation: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<T, E> {
    let started = Instant::now();
    let result = operation();
    *stage += started.elapsed();
    result
}

fn measure_stage<T>(stage: &mut Duration, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    *stage += started.elapsed();
    result
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

#[derive(Debug)]
struct ImportCancelPoller {
    min_interval: Duration,
    last_probe: Option<Instant>,
    cached_cancelled: bool,
}

impl ImportCancelPoller {
    fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            last_probe: None,
            cached_cancelled: false,
        }
    }

    fn poll(&mut self, now: Instant, probe: impl FnOnce() -> Result<bool>) -> Result<bool> {
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

#[derive(Clone, Debug)]
pub struct ImportOptions {
    pub scan_profile: ScanProfile,
    pub max_files: Option<usize>,
    pub parse_workers: ImportParseWorkers,
    pub index_writer_heap_bytes: usize,
    pub linear_promotion: LinearPromotionPolicy,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self::for_resource_policy(ImportResourcePolicy::detect())
    }
}

impl ImportOptions {
    pub fn for_resource_policy(resource_policy: ImportResourcePolicy) -> Self {
        Self {
            scan_profile: ScanProfile::default(),
            max_files: None,
            parse_workers: resource_policy.parse_workers,
            index_writer_heap_bytes: resource_policy.index_writer_heap_bytes,
            linear_promotion: LinearPromotionPolicy::default(),
        }
    }

    pub fn for_hardware_profile(hardware_profile: ImportHardwareProfile) -> Self {
        Self::for_resource_policy(ImportResourcePolicy::for_hardware(hardware_profile))
    }

    pub fn low_memory_default_for_available_parallelism(available_parallelism: usize) -> Self {
        Self {
            scan_profile: ScanProfile::default(),
            max_files: None,
            parse_workers: ImportParseWorkers::low_memory_default_for_available_parallelism(
                available_parallelism,
            ),
            index_writer_heap_bytes: H0_INDEX_WRITER_HEAP_BYTES,
            linear_promotion: LinearPromotionPolicy::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportHardwareProfile {
    pub total_memory_bytes: Option<u64>,
    pub available_parallelism: usize,
}

impl ImportHardwareProfile {
    pub fn new(total_memory_bytes: Option<u64>, available_parallelism: usize) -> Self {
        Self {
            total_memory_bytes,
            available_parallelism: available_parallelism.max(1),
        }
    }

    pub fn detect() -> Self {
        let available_parallelism = thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(1);
        Self::new(detect_total_memory_bytes(), available_parallelism)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportHardwareTier {
    H0Eco,
    H1Balanced,
    H2Aggressive,
}

impl ImportHardwareTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::H0Eco => "H0_Eco",
            Self::H1Balanced => "H1_Balanced",
            Self::H2Aggressive => "H2_Aggressive",
        }
    }

    fn default_parse_workers(self) -> usize {
        match self {
            Self::H0Eco => 1,
            Self::H1Balanced => 2,
            Self::H2Aggressive => MAX_IMPORT_PARSE_WORKERS,
        }
    }

    fn max_private_or_anonymous_mb(self) -> u16 {
        match self {
            Self::H0Eco => H0_MAX_PRIVATE_OR_ANONYMOUS_MB,
            Self::H1Balanced => H1_MAX_PRIVATE_OR_ANONYMOUS_MB,
            Self::H2Aggressive => H2_MAX_PRIVATE_OR_ANONYMOUS_MB,
        }
    }

    fn index_writer_heap_bytes(self) -> usize {
        match self {
            Self::H0Eco => H0_INDEX_WRITER_HEAP_BYTES,
            Self::H1Balanced => H1_INDEX_WRITER_HEAP_BYTES,
            Self::H2Aggressive => H2_INDEX_WRITER_HEAP_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportResourcePolicy {
    pub hardware_tier: ImportHardwareTier,
    pub parse_workers: ImportParseWorkers,
    pub index_writer_heap_bytes: usize,
    pub max_private_or_anonymous_mb: u16,
}

impl ImportResourcePolicy {
    pub fn detect() -> Self {
        Self::for_hardware(ImportHardwareProfile::detect())
    }

    pub fn for_hardware(hardware_profile: ImportHardwareProfile) -> Self {
        let hardware_tier = classify_import_hardware_tier(hardware_profile);
        let worker_limit = hardware_tier
            .default_parse_workers()
            .min(hardware_profile.available_parallelism);
        Self {
            hardware_tier,
            parse_workers: ImportParseWorkers::new(worker_limit),
            index_writer_heap_bytes: hardware_tier.index_writer_heap_bytes(),
            max_private_or_anonymous_mb: hardware_tier.max_private_or_anonymous_mb(),
        }
    }
}

fn classify_import_hardware_tier(hardware_profile: ImportHardwareProfile) -> ImportHardwareTier {
    match hardware_profile.total_memory_bytes {
        Some(memory_bytes) if memory_bytes > H1_MEMORY_CEILING_BYTES => {
            ImportHardwareTier::H2Aggressive
        }
        Some(memory_bytes) if memory_bytes > H0_MEMORY_CEILING_BYTES => {
            ImportHardwareTier::H1Balanced
        }
        _ => ImportHardwareTier::H0Eco,
    }
}

fn detect_total_memory_bytes() -> Option<u64> {
    let mut system = System::new();
    system.refresh_memory();
    let total_memory = system.total_memory();
    if total_memory == 0 {
        None
    } else {
        Some(total_memory)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportParseWorkers(NonZeroUsize);

impl ImportParseWorkers {
    pub fn new(count: usize) -> Self {
        let bounded = count.clamp(1, MAX_IMPORT_PARSE_WORKERS);
        Self(NonZeroUsize::new(bounded).expect("bounded worker count is non-zero"))
    }

    pub fn low_memory_default_for_available_parallelism(available_parallelism: usize) -> Self {
        Self::new(available_parallelism.clamp(1, MAX_IMPORT_PARSE_WORKERS))
    }

    pub fn sequential() -> Self {
        Self(NonZeroUsize::MIN)
    }

    pub fn get(self) -> usize {
        self.0.get()
    }
}

impl Default for ImportParseWorkers {
    fn default() -> Self {
        let available_parallelism = thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(1);
        Self::low_memory_default_for_available_parallelism(available_parallelism)
    }
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
        None,
        CurrentImportIndexCacheMode::Retain,
        None,
        None,
        None,
        ImportResourcePolicy::detect().index_writer_heap_bytes,
    )?;
    update_index_state(store, now, snapshot_token, indexed_documents)?;

    Ok(IndexRebuildSummary { indexed_documents })
}

pub fn index_claimed_ocr_text(
    data_dir: &Path,
    store: &MetaStore,
    claimed: &ClaimedOcrJob,
    ocr_text: &str,
    confidence: Option<f32>,
    page_count: Option<u32>,
    now: UnixTimestamp,
) -> Result<OcrTextIndexOutcome> {
    index_claimed_ocr_text_with_policy(
        data_dir,
        store,
        claimed,
        ocr_text,
        confidence,
        page_count,
        now,
        &LinearPromotionPolicy::default(),
    )
}

pub fn index_claimed_ocr_text_with_policy(
    data_dir: &Path,
    store: &MetaStore,
    claimed: &ClaimedOcrJob,
    ocr_text: &str,
    confidence: Option<f32>,
    page_count: Option<u32>,
    now: UnixTimestamp,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<OcrTextIndexOutcome> {
    let Some(mut document) = store
        .document_by_id(&claimed.job.document_id)
        .map_err(ImportPipelineError::store)?
    else {
        return Err(ImportPipelineError {
            kind: ImportPipelineErrorKind::Store,
            retryable: false,
        });
    };

    if document.content_hash.as_deref() != Some(claimed.source_fingerprint())
        || !store
            .ocr_claim_is_current(claimed)
            .map_err(ImportPipelineError::store)?
    {
        return Ok(OcrTextIndexOutcome::Superseded);
    }

    let clean_text = TextNormalizer::normalize_text_only(ocr_text);
    let sectionizer = Sectionizer::default();
    let sections = sectionizer.sectionize(&clean_text);
    let decision =
        AdmissionDecision::after_sectionization(&clean_text, &sections, linear_promotion);
    let admitted = decision.admits_search_index();
    let pending_doc_ids = BTreeSet::from([document.id.as_str().to_string()]);
    let version_id = ResumeVersionId::from_non_secret_parts(&[
        "ocr",
        document.id.as_str(),
        claimed.source_fingerprint(),
        OCR_PARSE_VERSION,
        SCHEMA_VERSION,
    ]);
    let version = (!clean_text.trim().is_empty()).then(|| ResumeVersion {
        id: version_id.clone(),
        document_id: document.id.clone(),
        candidate_id: None,
        parse_version: OCR_PARSE_VERSION.to_string(),
        schema_version: SCHEMA_VERSION.to_string(),
        language_set: language_set(&clean_text),
        page_count,
        raw_text: None,
        clean_text: Some(clean_text.clone()),
        quality_score: Some(confidence.unwrap_or(0.5)),
        visibility: if admitted {
            ResumeVisibility::Searchable
        } else {
            ResumeVisibility::Hidden
        },
    });
    document.status = if admitted {
        DocumentStatus::Searchable
    } else {
        DocumentStatus::OcrDone
    };
    document.updated_at = now;
    let mentions = if admitted {
        entity_mentions_from_rules(&version_id, &clean_text)
    } else {
        Vec::new()
    };
    let pending_index_documents = if admitted {
        vec![IndexDocument {
            doc_id: document.id.to_string(),
            version_id: version_id.to_string(),
            file_name: document.file_name.clone(),
            clean_text: clean_text.clone(),
            sections: sections_to_index(sections),
            is_deleted: document.is_deleted,
        }]
    } else {
        Vec::new()
    };
    let (snapshot_token, indexed_documents) = write_incremental_full_text_index(
        data_dir,
        store,
        now,
        pending_index_documents,
        &pending_doc_ids,
        0,
        0,
        None,
        CurrentImportIndexCacheMode::Retain,
        None,
        None,
        None,
        ImportResourcePolicy::detect().index_writer_heap_bytes,
    )?;
    let visible_epoch = store
        .index_state()
        .map_err(ImportPipelineError::store)?
        .map_or(1, |state| state.visible_epoch.saturating_add(1));
    let index_state = IndexState {
        manifest_version: INDEX_MANIFEST_VERSION.to_string(),
        snapshot_token: Some(snapshot_token),
        status: IndexStateStatus::Ready,
        updated_at: now,
        visible_epoch,
        manifest_document_count: indexed_documents as u64,
    };
    let (email_hash, phone_hash) = if admitted {
        contact_hashes_from_mentions(data_dir, &mentions)?
    } else {
        (None, None)
    };
    let classification = decision.into_record(document.id.clone(), now);
    let publication = OcrAttemptPublication {
        document: &document,
        classification: &classification,
        version: version.as_ref(),
        mentions: &mentions,
        email_hash: email_hash.as_ref(),
        phone_hash: phone_hash.as_ref(),
        index_state: &index_state,
    };
    match store
        .finish_ocr_attempt_success(claimed, publication, now)
        .map_err(ImportPipelineError::store)?
    {
        OcrAttemptSuccessOutcome::Completed => {
            Ok(OcrTextIndexOutcome::Committed(OcrTextIndexSummary {
                searchable: admitted,
                indexed_documents,
            }))
        }
        OcrAttemptSuccessOutcome::Superseded => Ok(OcrTextIndexOutcome::Superseded),
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrTextIndexOutcome {
    Committed(OcrTextIndexSummary),
    Superseded,
}

fn write_full_text_index<I>(
    data_dir: &Path,
    snapshot_token: &str,
    index_documents: I,
    input_redaction: FullTextIndexInputRedaction,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    set_cancel_phase: Option<&dyn Fn(ImportCancelCheckPhase)>,
    record_phase_timing: Option<&dyn Fn(SnapshotPublishPhase, Duration)>,
    index_writer_heap_bytes: usize,
) -> Result<()>
where
    I: IntoIterator<Item = IndexDocument>,
{
    let index_root = data_dir.join("search-index");
    let cancel_check =
        || ensure_not_cancelled.is_some_and(|ensure_not_cancelled| ensure_not_cancelled().is_err());
    let phase_observer = |phase| {
        if let Some(set_cancel_phase) = set_cancel_phase {
            set_cancel_phase(ImportCancelCheckPhase::from_snapshot_publish_phase(phase));
        }
    };
    let mut control = if ensure_not_cancelled.is_some() {
        SnapshotPublishControl::from_cancel_check(&cancel_check)
    } else {
        SnapshotPublishControl::disabled()
    };
    if set_cancel_phase.is_some() {
        control = control.with_phase_observer(&phase_observer);
    }
    if let Some(record_phase_timing) = record_phase_timing {
        control = control.with_phase_timing_observer(record_phase_timing);
    }
    control = control.with_writer_heap_bytes(index_writer_heap_bytes);
    match input_redaction {
        FullTextIndexInputRedaction::Redact => {
            publish_snapshot_with_control(&index_root, snapshot_token, index_documents, control)
        }
        FullTextIndexInputRedaction::TrustedRedacted => {
            publish_trusted_redacted_snapshot_with_control(
                &index_root,
                snapshot_token,
                index_documents,
                control,
            )
        }
    }
    .map_err(ImportPipelineError::index)
}

#[derive(Clone, Copy)]
enum FullTextIndexInputRedaction {
    Redact,
    TrustedRedacted,
}

fn write_incremental_full_text_index(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    replacement_documents: Vec<IndexDocument>,
    excluded_doc_ids: &BTreeSet<String>,
    ocr_required_documents: usize,
    deleted_documents: usize,
    current_import_index_documents: Option<&mut CurrentImportIndexDocuments>,
    current_import_index_cache_mode: CurrentImportIndexCacheMode,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    set_cancel_phase: Option<&dyn Fn(ImportCancelCheckPhase)>,
    record_phase_timing: Option<&dyn Fn(SnapshotPublishPhase, Duration)>,
    index_writer_heap_bytes: usize,
) -> Result<(String, usize)> {
    let index_root = data_dir.join("search-index");
    if let Some(current_import_index_documents) = current_import_index_documents {
        ensure_current_import_index_documents(&index_root, store, current_import_index_documents)?;
        apply_index_document_delta(
            &mut current_import_index_documents.documents,
            replacement_documents,
            excluded_doc_ids,
        );
        let indexed_documents = current_import_index_documents.documents.len();
        let snapshot_token = index_snapshot_token(
            now,
            indexed_documents,
            ocr_required_documents,
            deleted_documents,
        );
        let sectionizer = Sectionizer::default();
        match current_import_index_cache_mode {
            CurrentImportIndexCacheMode::Retain => write_current_import_full_text_index(
                data_dir,
                &snapshot_token,
                &current_import_index_documents.documents,
                &sectionizer,
                ensure_not_cancelled,
                set_cancel_phase,
                record_phase_timing,
                index_writer_heap_bytes,
            )?,
            CurrentImportIndexCacheMode::Consume => write_current_import_full_text_index_consuming(
                data_dir,
                &snapshot_token,
                &mut current_import_index_documents.documents,
                &sectionizer,
                ensure_not_cancelled,
                set_cancel_phase,
                record_phase_timing,
                index_writer_heap_bytes,
            )?,
        }

        return Ok((snapshot_token, indexed_documents));
    }

    let index_documents = incremental_snapshot_documents_with_fallback(
        &index_root,
        store,
        replacement_documents,
        excluded_doc_ids,
    )?;
    let indexed_documents = index_documents.len();
    let snapshot_token = index_snapshot_token(
        now,
        indexed_documents,
        ocr_required_documents,
        deleted_documents,
    );
    write_full_text_index(
        data_dir,
        &snapshot_token,
        index_documents,
        FullTextIndexInputRedaction::Redact,
        None,
        None,
        None,
        index_writer_heap_bytes,
    )?;

    Ok((snapshot_token, indexed_documents))
}

#[derive(Default)]
struct CurrentImportIndexDocuments {
    initialized: bool,
    documents: Vec<CachedIndexDocument>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CurrentImportIndexCacheMode {
    Retain,
    Consume,
}

#[derive(Clone, PartialEq, Eq)]
struct CachedIndexDocument {
    doc_id: String,
    version_id: String,
    file_name: String,
    clean_text: String,
    sections: Vec<IndexSection>,
    is_deleted: bool,
}

impl CachedIndexDocument {
    fn from_index_document(document: IndexDocument) -> Self {
        Self {
            doc_id: document.doc_id,
            version_id: document.version_id,
            file_name: redact_contact_values(&document.file_name),
            clean_text: redact_contact_values(&document.clean_text),
            sections: Vec::new(),
            is_deleted: document.is_deleted,
        }
    }

    fn to_index_document(&self, sectionizer: &Sectionizer) -> IndexDocument {
        IndexDocument {
            doc_id: self.doc_id.clone(),
            version_id: self.version_id.clone(),
            file_name: self.file_name.clone(),
            clean_text: self.clean_text.clone(),
            sections: sections_to_index(sectionizer.sectionize(&self.clean_text)),
            is_deleted: self.is_deleted,
        }
    }

    fn into_index_document(self, sectionizer: &Sectionizer) -> IndexDocument {
        let sections = sections_to_index(sectionizer.sectionize(&self.clean_text));
        IndexDocument {
            doc_id: self.doc_id,
            version_id: self.version_id,
            file_name: self.file_name,
            clean_text: self.clean_text,
            sections,
            is_deleted: self.is_deleted,
        }
    }
}

fn write_current_import_full_text_index(
    data_dir: &Path,
    snapshot_token: &str,
    documents: &[CachedIndexDocument],
    sectionizer: &Sectionizer,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    set_cancel_phase: Option<&dyn Fn(ImportCancelCheckPhase)>,
    record_phase_timing: Option<&dyn Fn(SnapshotPublishPhase, Duration)>,
    index_writer_heap_bytes: usize,
) -> Result<()> {
    write_full_text_index(
        data_dir,
        snapshot_token,
        documents
            .iter()
            .map(|document| document.to_index_document(sectionizer)),
        FullTextIndexInputRedaction::TrustedRedacted,
        ensure_not_cancelled,
        set_cancel_phase,
        record_phase_timing,
        index_writer_heap_bytes,
    )
}

fn write_current_import_full_text_index_consuming(
    data_dir: &Path,
    snapshot_token: &str,
    documents: &mut Vec<CachedIndexDocument>,
    sectionizer: &Sectionizer,
    ensure_not_cancelled: Option<&dyn Fn() -> Result<()>>,
    set_cancel_phase: Option<&dyn Fn(ImportCancelCheckPhase)>,
    record_phase_timing: Option<&dyn Fn(SnapshotPublishPhase, Duration)>,
    index_writer_heap_bytes: usize,
) -> Result<()> {
    let documents = std::mem::take(documents);
    write_full_text_index(
        data_dir,
        snapshot_token,
        documents
            .into_iter()
            .map(|document| document.into_index_document(sectionizer)),
        FullTextIndexInputRedaction::TrustedRedacted,
        ensure_not_cancelled,
        set_cancel_phase,
        record_phase_timing,
        index_writer_heap_bytes,
    )
}

fn ensure_current_import_index_documents(
    index_root: &Path,
    store: &MetaStore,
    current_import_index_documents: &mut CurrentImportIndexDocuments,
) -> Result<()> {
    if current_import_index_documents.initialized {
        return Ok(());
    }

    current_import_index_documents.documents = incremental_snapshot_documents_with_fallback(
        index_root,
        store,
        Vec::new(),
        &BTreeSet::new(),
    )?
    .into_iter()
    .map(CachedIndexDocument::from_index_document)
    .collect();
    current_import_index_documents.initialized = true;
    Ok(())
}

fn incremental_snapshot_documents_with_fallback(
    index_root: &Path,
    store: &MetaStore,
    replacement_documents: Vec<IndexDocument>,
    excluded_doc_ids: &BTreeSet<String>,
) -> Result<Vec<IndexDocument>> {
    let replacement_doc_ids = replacement_documents
        .iter()
        .map(|document| document.doc_id.clone())
        .collect::<BTreeSet<_>>();
    match incremental_snapshot_documents(
        index_root,
        replacement_documents.clone(),
        excluded_doc_ids,
    ) {
        Ok(mut index_documents) => {
            let admitted_doc_ids = current_candidate_document_ids(store)?;
            index_documents.retain(|document| {
                replacement_doc_ids.contains(&document.doc_id)
                    || admitted_doc_ids.contains(&document.doc_id)
            });
            Ok(index_documents)
        }
        Err(_) => {
            let sectionizer = Sectionizer::default();
            let mut rebuilt_documents =
                persisted_index_documents(store, &sectionizer, excluded_doc_ids)?;
            rebuilt_documents.extend(
                replacement_documents
                    .into_iter()
                    .filter(|document| !document.is_deleted),
            );
            sort_index_documents(&mut rebuilt_documents);
            Ok(rebuilt_documents)
        }
    }
}

fn current_candidate_document_ids(store: &MetaStore) -> Result<BTreeSet<String>> {
    let mut admitted = BTreeSet::new();
    for document in store
        .visible_documents()
        .map_err(ImportPipelineError::store)?
    {
        if !matches!(
            document.status,
            DocumentStatus::Searchable | DocumentStatus::IndexedPartial
        ) {
            continue;
        }
        let classification = store
            .document_classification_by_id(&document.id)
            .map_err(ImportPipelineError::store)?;
        if classification.as_ref().is_some_and(|record| {
            is_current(record) && record.status == ClassificationStatus::ResumeCandidate
        }) {
            admitted.insert(document.id.to_string());
        }
    }
    Ok(admitted)
}

fn apply_index_document_delta(
    documents: &mut Vec<CachedIndexDocument>,
    replacement_documents: Vec<IndexDocument>,
    excluded_doc_ids: &BTreeSet<String>,
) {
    let mut excluded_doc_ids = excluded_doc_ids.clone();
    for document in &replacement_documents {
        excluded_doc_ids.insert(document.doc_id.clone());
    }

    documents.retain(|document| !excluded_doc_ids.contains(&document.doc_id));
    documents.extend(
        replacement_documents
            .into_iter()
            .filter(|document| !document.is_deleted)
            .map(CachedIndexDocument::from_index_document),
    );
    sort_cached_index_documents(documents);
}

fn sort_index_documents(documents: &mut [IndexDocument]) {
    documents.sort_by(|left, right| {
        left.doc_id
            .cmp(&right.doc_id)
            .then_with(|| left.version_id.cmp(&right.version_id))
    });
}

fn sort_cached_index_documents(documents: &mut [CachedIndexDocument]) {
    documents.sort_by(|left, right| {
        left.doc_id
            .cmp(&right.doc_id)
            .then_with(|| left.version_id.cmp(&right.version_id))
    });
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
    write_full_text_index(
        data_dir,
        &snapshot_token,
        index_documents,
        FullTextIndexInputRedaction::Redact,
        None,
        None,
        None,
        ImportResourcePolicy::detect().index_writer_heap_bytes,
    )?;

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

        let classification = store
            .document_classification_by_id(&document.id)
            .map_err(ImportPipelineError::store)?;
        if !classification.as_ref().is_some_and(|record| {
            is_current(record) && record.status == ClassificationStatus::ResumeCandidate
        }) {
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
    if version.visibility != ResumeVisibility::Searchable {
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

fn prepare_file_for_parse(
    store: &MetaStore,
    index: usize,
    file: DiscoveredFile,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    db_timing: &mut Duration,
    parse_prepare_timing: &mut Duration,
    content_bytes_read: &mut u64,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<PreparedFile> {
    let started = Instant::now();
    let mut db_elapsed = Duration::ZERO;
    let result = prepare_file_for_parse_inner(
        store,
        index,
        file,
        now,
        ensure_not_cancelled,
        &mut db_elapsed,
        content_bytes_read,
        linear_promotion,
    );
    *db_timing += db_elapsed;
    *parse_prepare_timing += started.elapsed().saturating_sub(db_elapsed);
    result
}

fn prepare_file_for_parse_inner(
    store: &MetaStore,
    index: usize,
    file: DiscoveredFile,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    db_elapsed: &mut Duration,
    content_bytes_read: &mut u64,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<PreparedFile> {
    ensure_not_cancelled()?;
    if let Some(noop_kind) = measure_result_stage(db_elapsed, || {
        exact_rerun_noop_kind(store, &file, linear_promotion)
    })? {
        let processed = match noop_kind {
            ExactRerunNoopKind::Searchable => ProcessedFile::UnchangedSearchable,
            ExactRerunNoopKind::OcrRequired => ProcessedFile::UnchangedOcrRequired,
            ExactRerunNoopKind::Excluded => ProcessedFile::UnchangedExcluded,
        };
        return Ok(PreparedFile::Ready(ProcessedImportFile { file, processed }));
    }

    let mut document = document_from_discovered_file(&file, now, DocumentStatus::Discovered);
    measure_result_stage(db_elapsed, || store.upsert_document(&document))
        .map_err(ImportPipelineError::store)?;
    ensure_not_cancelled()?;

    if file.extension == FileExtension::Txt
        && file.byte_size > parser_text::DEFAULT_MAX_BYTES as u64
    {
        document.status = DocumentStatus::FailedPermanent;
        document.updated_at = now;
        measure_result_stage(db_elapsed, || persist_failed(store, &document, now))?;
        return Ok(PreparedFile::Ready(ProcessedImportFile {
            file,
            processed: ProcessedFile::Failed {
                kind: ImportFailureKind::TextTooLarge,
            },
        }));
    }

    let path = PathBuf::from(file.normalized_path.as_str());
    ensure_not_cancelled()?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            document.status = DocumentStatus::FailedRetryable;
            document.updated_at = now;
            measure_result_stage(db_elapsed, || persist_failed(store, &document, now))?;
            return Ok(PreparedFile::Ready(ProcessedImportFile {
                file,
                processed: ProcessedFile::Failed {
                    kind: ImportFailureKind::ReadError,
                },
            }));
        }
    };
    *content_bytes_read += bytes.len() as u64;
    ensure_not_cancelled()?;

    Ok(PreparedFile::Parse(ParseWorkItem {
        index,
        file,
        document,
        bytes,
    }))
}

fn parse_worker_loop(
    work_rx: Arc<Mutex<mpsc::Receiver<ParseWorkItem>>>,
    result_tx: mpsc::SyncSender<ParseWorkResult>,
    linear_promotion: &LinearPromotionPolicy,
) {
    loop {
        let work = match work_rx.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(work) = work else {
            return;
        };
        if result_tx
            .send(parse_work_item(work, linear_promotion))
            .is_err()
        {
            return;
        }
    }
}

fn parse_work_item(
    work: ParseWorkItem,
    linear_promotion: &LinearPromotionPolicy,
) -> ParseWorkResult {
    let ParseWorkItem {
        index,
        file,
        document,
        bytes,
    } = work;
    let parse_started = Instant::now();
    let output = parse_work_item_inner(&file, &document, &bytes, linear_promotion);
    let parse_finished = Instant::now();

    ParseWorkResult {
        index,
        file,
        document,
        parse_elapsed: parse_finished.saturating_duration_since(parse_started),
        parse_started,
        parse_finished,
        pdf_parse_timings: output.pdf_parse_timings,
        post_parser_timings: output.post_parser_timings,
        outcome: output.outcome,
    }
}

fn parse_work_item_inner(
    file: &DiscoveredFile,
    document: &Document,
    bytes: &[u8],
    linear_promotion: &LinearPromotionPolicy,
) -> ParseWorkItemOutput {
    let extension = file_extension_label(&file.extension);
    let mut pdf_parse_timings = PdfTextExtractionTimings::default();
    let mut post_parser_timings = ImportPostParserTimings::default();
    let parse_output = match file.extension {
        FileExtension::Docx => DocxParser.parse(
            ParseInput::from_bytes(Some(extension), bytes),
            ResourceBudget::default(),
        ),
        FileExtension::Doc => DocParser::default().parse(
            ParseInput::from_bytes(Some(extension), bytes),
            ResourceBudget::default(),
        ),
        FileExtension::Pdf => {
            match PdfParser.parse_with_timings(
                ParseInput::from_bytes(Some(extension), bytes),
                ResourceBudget::default(),
            ) {
                Ok((parse_output, timings)) => {
                    pdf_parse_timings = timings;
                    Ok(parse_output)
                }
                Err(error) => Err(error),
            }
        }
        FileExtension::Txt => TxtParser.parse(
            ParseInput::from_bytes(Some(extension), bytes),
            ResourceBudget::default().with_max_bytes(parser_text::DEFAULT_MAX_BYTES),
        ),
        _ => {
            return ParseWorkItemOutput {
                outcome: ParseWorkOutcome::Failed {
                    status: DocumentStatus::FailedPermanent,
                    kind: ImportFailureKind::UnsupportedExtension,
                },
                pdf_parse_timings,
                post_parser_timings,
            };
        }
    };

    let parse_output = match parse_output {
        Ok(parse_output) => parse_output,
        Err(error) => {
            let status = if error.retryable() {
                DocumentStatus::FailedRetryable
            } else if error.kind() == ParserErrorKind::OcrRequired {
                DocumentStatus::OcrRequired
            } else {
                DocumentStatus::FailedPermanent
            };
            let outcome = if status == DocumentStatus::OcrRequired {
                ParseWorkOutcome::OcrRequired
            } else {
                ParseWorkOutcome::Failed {
                    status,
                    kind: ImportFailureKind::from_parser_error(error.kind()),
                }
            };
            return ParseWorkItemOutput {
                outcome,
                pdf_parse_timings,
                post_parser_timings,
            };
        }
    };

    if parse_output.status() == ParseStatus::OcrRequired {
        return ParseWorkItemOutput {
            outcome: ParseWorkOutcome::OcrRequired,
            pdf_parse_timings,
            post_parser_timings,
        };
    }

    let clean_text = measure_stage(&mut post_parser_timings.normalization, || {
        TextNormalizer::normalize_text_only(parse_output.text())
    });
    if clean_text.trim().is_empty() {
        let outcome = if file.extension == FileExtension::Txt {
            ParseWorkOutcome::Failed {
                status: DocumentStatus::FailedPermanent,
                kind: ImportFailureKind::EmptyText,
            }
        } else {
            ParseWorkOutcome::OcrRequired
        };
        return ParseWorkItemOutput {
            outcome,
            pdf_parse_timings,
            post_parser_timings,
        };
    }

    let version_id = ResumeVersionId::from_non_secret_parts(&[
        "s9",
        document.id.as_str(),
        PARSE_VERSION,
        SCHEMA_VERSION,
    ]);
    let sections = measure_stage(&mut post_parser_timings.sectionization, || {
        Sectionizer::default().sectionize(&clean_text)
    });
    let decision =
        AdmissionDecision::after_sectionization(&clean_text, &sections, linear_promotion);
    let admitted = decision.admits_search_index();
    let version = ResumeVersion {
        id: version_id.clone(),
        document_id: document.id.clone(),
        candidate_id: None,
        parse_version: PARSE_VERSION.to_string(),
        schema_version: SCHEMA_VERSION.to_string(),
        language_set: language_set(&clean_text),
        page_count: parse_output
            .page_count()
            .and_then(|page_count| u32::try_from(page_count).ok()),
        raw_text: None,
        clean_text: Some(clean_text.clone()),
        quality_score: Some(0.8),
        visibility: if admitted {
            ResumeVisibility::Searchable
        } else {
            ResumeVisibility::Hidden
        },
    };
    let outcome = if admitted {
        ParseWorkOutcome::Searchable {
            decision,
            version: Box::new(version),
            mentions: entity_mentions_from_rules(&version_id, &clean_text),
            index_document: Box::new(IndexDocument {
                doc_id: document.id.to_string(),
                version_id: version_id.to_string(),
                file_name: file.file_name.clone(),
                clean_text,
                sections: sections_to_index(sections),
                is_deleted: false,
            }),
        }
    } else {
        ParseWorkOutcome::Excluded {
            decision,
            version: Box::new(version),
        }
    };

    ParseWorkItemOutput {
        outcome,
        pdf_parse_timings,
        post_parser_timings,
    }
}

fn send_parse_work_with_backpressure(
    work_tx: &mpsc::SyncSender<ParseWorkItem>,
    result_rx: &mpsc::Receiver<ParseWorkResult>,
    pending_results: &mut BTreeMap<usize, ImportFileResult>,
    worker_metrics: &mut ImportWorkerMetrics,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    mut work: ParseWorkItem,
) -> Result<()> {
    let mut queue_full_events = 0_usize;
    let mut queue_wait = Duration::ZERO;
    loop {
        match work_tx.try_send(work) {
            Ok(()) => {
                worker_metrics.parse_jobs_queued += 1;
                worker_metrics.parse_queue_full_events += queue_full_events;
                worker_metrics.parse_queue_wait += queue_wait;
                return Ok(());
            }
            Err(mpsc::TrySendError::Full(returned_work)) => {
                work = returned_work;
                queue_full_events += 1;
                set_cancel_phase(ImportCancelCheckPhase::ParseQueueWait);
                let wait_started = Instant::now();
                let result = recv_parse_result_with_cancel_poll(result_rx, ensure_not_cancelled)?;
                queue_wait += wait_started.elapsed();
                insert_parse_result(pending_results, result)?;
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err(parallel_parse_error());
            }
        }
    }
}

fn drain_available_parse_results(
    result_rx: &mpsc::Receiver<ParseWorkResult>,
    pending_results: &mut BTreeMap<usize, ImportFileResult>,
) -> Result<()> {
    loop {
        match result_rx.try_recv() {
            Ok(result) => insert_parse_result(pending_results, result)?,
            Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => {
                return Ok(());
            }
        }
    }
}

fn recv_parse_result_with_cancel_poll(
    result_rx: &mpsc::Receiver<ParseWorkResult>,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
) -> Result<ParseWorkResult> {
    loop {
        match result_rx.recv_timeout(Duration::from_millis(PARSE_RESULT_CANCEL_POLL_INTERVAL_MS)) {
            Ok(result) => return Ok(result),
            Err(mpsc::RecvTimeoutError::Timeout) => ensure_not_cancelled()?,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(parallel_parse_error()),
        }
    }
}

fn insert_parse_result(
    pending_results: &mut BTreeMap<usize, ImportFileResult>,
    result: ParseWorkResult,
) -> Result<()> {
    insert_import_file_result(
        pending_results,
        result.index,
        ImportFileResult::Parsed(result),
    )
}

fn insert_import_file_result(
    pending_results: &mut BTreeMap<usize, ImportFileResult>,
    index: usize,
    result: ImportFileResult,
) -> Result<()> {
    if pending_results.insert(index, result).is_some() {
        return Err(parallel_parse_error());
    }
    Ok(())
}

fn commit_ready_import_file_results(
    data_dir: &Path,
    store: &MetaStore,
    task_id: &ImportTaskId,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    summary: &mut ImportSummary,
    pending_index_documents: &mut Vec<(Document, IndexDocument)>,
    pending_excluded_doc_ids: &mut BTreeSet<String>,
    current_import_index_documents: &mut CurrentImportIndexDocuments,
    import_started: Instant,
    total_files: usize,
    pending_results: &mut BTreeMap<usize, ImportFileResult>,
    next_commit_index: &mut usize,
    parse_worker_clock: &mut ParseWorkerClock,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    index_writer_heap_bytes: usize,
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
            )?,
        };
        finish_import_file(
            data_dir,
            store,
            task_id,
            now,
            ensure_not_cancelled,
            summary,
            pending_index_documents,
            pending_excluded_doc_ids,
            current_import_index_documents,
            set_cancel_phase,
            import_started,
            index_writer_heap_bytes,
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

fn commit_parse_work_result(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    db_timing: &mut Duration,
    worker_metrics: &mut ImportWorkerMetrics,
    parse_worker_clock: &mut ParseWorkerClock,
    result: ParseWorkResult,
) -> Result<ProcessedImportFile> {
    parse_worker_clock.record_result(&result);
    worker_metrics
        .pdf_parse_timings
        .add_assign(&result.pdf_parse_timings);
    worker_metrics
        .post_parser_timings
        .add_assign(&result.post_parser_timings);
    let ParseWorkResult {
        file,
        mut document,
        outcome,
        ..
    } = result;

    let processed = match outcome {
        ParseWorkOutcome::Searchable {
            decision,
            version,
            mentions,
            index_document,
        } => {
            document.status = DocumentStatus::TextCleaned;
            document.updated_at = now;
            let version = *version;
            measure_result_stage(db_timing, || {
                persist_classification(store, &document.id, decision, now)?;
                let (email_hash, phone_hash) = contact_hashes_from_mentions(data_dir, &mentions)?;
                store
                    .upsert_searchable_import_document(SearchableImportDocument {
                        document: &document,
                        version: &version,
                        mentions: &mentions,
                        email_hash: email_hash.as_ref(),
                        phone_hash: phone_hash.as_ref(),
                    })
                    .map_err(ImportPipelineError::store)?;
                Ok(())
            })?;
            ProcessedFile::Searchable {
                document: Box::new(document),
                index_document,
            }
        }
        ParseWorkOutcome::Excluded { decision, version } => {
            document.status = DocumentStatus::TextCleaned;
            document.updated_at = now;
            let mut version = *version;
            version.visibility = ResumeVisibility::Hidden;
            measure_result_stage(db_timing, || {
                persist_non_searchable(store, &document, Some(&version), decision, now)
            })?;
            ProcessedFile::Excluded
        }
        ParseWorkOutcome::OcrRequired => ProcessedFile::OcrRequired {
            ocr_job_queued: measure_result_stage(db_timing, || {
                mark_ocr_required_and_enqueue(store, &mut document, now)
            })?,
        },
        ParseWorkOutcome::Failed { status, kind } => {
            document.status = status;
            document.updated_at = now;
            measure_result_stage(db_timing, || persist_failed(store, &document, now))?;
            ProcessedFile::Failed { kind }
        }
    };

    Ok(ProcessedImportFile { file, processed })
}

fn parallel_parse_error() -> ImportPipelineError {
    ImportPipelineError {
        kind: ImportPipelineErrorKind::Parser,
        retryable: true,
    }
}

fn process_file(
    data_dir: &Path,
    store: &MetaStore,
    file: &DiscoveredFile,
    sectionizer: &Sectionizer,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    stage_timings: &mut ImportStageTimings,
    worker_metrics: &mut ImportWorkerMetrics,
    content_bytes_read: &mut u64,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<ProcessedFile> {
    let started = Instant::now();
    let mut db_elapsed = Duration::ZERO;
    let result = process_file_inner(
        data_dir,
        store,
        file,
        sectionizer,
        now,
        ensure_not_cancelled,
        &mut db_elapsed,
        worker_metrics,
        content_bytes_read,
        linear_promotion,
    );
    stage_timings.db += db_elapsed;
    stage_timings.parse += started.elapsed().saturating_sub(db_elapsed);
    result
}

fn process_file_inner(
    data_dir: &Path,
    store: &MetaStore,
    file: &DiscoveredFile,
    sectionizer: &Sectionizer,
    now: UnixTimestamp,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    db_elapsed: &mut Duration,
    worker_metrics: &mut ImportWorkerMetrics,
    content_bytes_read: &mut u64,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<ProcessedFile> {
    ensure_not_cancelled()?;
    if let Some(noop_kind) = measure_result_stage(db_elapsed, || {
        exact_rerun_noop_kind(store, file, linear_promotion)
    })? {
        return Ok(match noop_kind {
            ExactRerunNoopKind::Searchable => ProcessedFile::UnchangedSearchable,
            ExactRerunNoopKind::OcrRequired => ProcessedFile::UnchangedOcrRequired,
            ExactRerunNoopKind::Excluded => ProcessedFile::UnchangedExcluded,
        });
    }

    let mut document = document_from_discovered_file(file, now, DocumentStatus::Discovered);
    measure_result_stage(db_elapsed, || store.upsert_document(&document))
        .map_err(ImportPipelineError::store)?;
    ensure_not_cancelled()?;

    if file.extension == FileExtension::Txt
        && file.byte_size > parser_text::DEFAULT_MAX_BYTES as u64
    {
        document.status = DocumentStatus::FailedPermanent;
        document.updated_at = now;
        measure_result_stage(db_elapsed, || persist_failed(store, &document, now))?;
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
            document.updated_at = now;
            measure_result_stage(db_elapsed, || persist_failed(store, &document, now))?;
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::ReadError,
            });
        }
    };
    *content_bytes_read += bytes.len() as u64;
    ensure_not_cancelled()?;

    let extension = file_extension_label(&file.extension);
    ensure_not_cancelled()?;
    let mut pdf_parse_timings = PdfTextExtractionTimings::default();
    let mut post_parser_timings = ImportPostParserTimings::default();
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
        FileExtension::Pdf => match PdfParser.parse_with_timings(
            ParseInput::from_bytes(Some(extension), &bytes),
            ResourceBudget::default(),
        ) {
            Ok((parse_output, timings)) => {
                pdf_parse_timings = timings;
                Ok(parse_output)
            }
            Err(error) => Err((error, document.clone())),
        },
        FileExtension::Txt => TxtParser
            .parse(
                ParseInput::from_bytes(Some(extension), &bytes),
                ResourceBudget::default().with_max_bytes(parser_text::DEFAULT_MAX_BYTES),
            )
            .map_err(|error| (error, document.clone())),
        _ => {
            document.status = DocumentStatus::FailedPermanent;
            document.updated_at = now;
            measure_result_stage(db_elapsed, || persist_failed(store, &document, now))?;
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::UnsupportedExtension,
            });
        }
    };
    worker_metrics
        .pdf_parse_timings
        .add_assign(&pdf_parse_timings);
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
                    ocr_job_queued: measure_result_stage(db_elapsed, || {
                        mark_ocr_required_and_enqueue(store, &mut document, now)
                    })?,
                }
            } else {
                measure_result_stage(db_elapsed, || persist_failed(store, &document, now))?;
                ProcessedFile::Failed {
                    kind: ImportFailureKind::from_parser_error(error.kind()),
                }
            });
        }
    };

    if parse_output.status() == ParseStatus::OcrRequired {
        ensure_not_cancelled()?;
        return Ok(ProcessedFile::OcrRequired {
            ocr_job_queued: measure_result_stage(db_elapsed, || {
                mark_ocr_required_and_enqueue(store, &mut document, now)
            })?,
        });
    }

    ensure_not_cancelled()?;
    let clean_text = measure_stage(&mut post_parser_timings.normalization, || {
        TextNormalizer::normalize_text_only(parse_output.text())
    });
    worker_metrics
        .post_parser_timings
        .add_assign(&post_parser_timings);
    if clean_text.trim().is_empty() {
        if file.extension == FileExtension::Txt {
            document.status = DocumentStatus::FailedPermanent;
            document.updated_at = now;
            measure_result_stage(db_elapsed, || persist_failed(store, &document, now))?;
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::EmptyText,
            });
        }

        ensure_not_cancelled()?;
        return Ok(ProcessedFile::OcrRequired {
            ocr_job_queued: measure_result_stage(db_elapsed, || {
                mark_ocr_required_and_enqueue(store, &mut document, now)
            })?,
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
    let sections = measure_stage(&mut post_parser_timings.sectionization, || {
        sectionizer.sectionize(&clean_text)
    });
    worker_metrics.post_parser_timings.sectionization += post_parser_timings.sectionization;
    let decision =
        AdmissionDecision::after_sectionization(&clean_text, &sections, linear_promotion);
    let admitted = decision.admits_search_index();
    let version = ResumeVersion {
        id: version_id.clone(),
        document_id: document.id.clone(),
        candidate_id: None,
        parse_version: PARSE_VERSION.to_string(),
        schema_version: SCHEMA_VERSION.to_string(),
        language_set: language_set(&clean_text),
        page_count: parse_output
            .page_count()
            .and_then(|page_count| u32::try_from(page_count).ok()),
        raw_text: None,
        clean_text: Some(clean_text.clone()),
        quality_score: Some(0.8),
        visibility: if admitted {
            ResumeVisibility::Searchable
        } else {
            ResumeVisibility::Hidden
        },
    };
    if !admitted {
        measure_result_stage(db_elapsed, || {
            persist_non_searchable(store, &document, Some(&version), decision, now)
        })?;
        return Ok(ProcessedFile::Excluded);
    }
    let mentions = entity_mentions_from_rules(&version_id, &clean_text);
    ensure_not_cancelled()?;
    measure_result_stage(db_elapsed, || {
        persist_classification(store, &document.id, decision, now)?;
        let (email_hash, phone_hash) = contact_hashes_from_mentions(data_dir, &mentions)?;
        store
            .upsert_searchable_import_document(SearchableImportDocument {
                document: &document,
                version: &version,
                mentions: &mentions,
                email_hash: email_hash.as_ref(),
                phone_hash: phone_hash.as_ref(),
            })
            .map_err(ImportPipelineError::store)?;
        Ok(())
    })?;

    ensure_not_cancelled()?;
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

#[allow(dead_code)]
fn assign_candidate_from_contact_mentions(
    data_dir: &Path,
    store: &MetaStore,
    version_id: &ResumeVersionId,
    mentions: &[EntityMention],
) -> Result<()> {
    let (email_hash, phone_hash) = contact_hashes_from_mentions(data_dir, mentions)?;
    if email_hash.is_none() && phone_hash.is_none() {
        return Ok(());
    }

    store
        .assign_candidate_from_hashed_contacts(version_id, email_hash.as_ref(), phone_hash.as_ref())
        .map_err(ImportPipelineError::store)?;

    Ok(())
}

fn contact_hashes_from_mentions(
    data_dir: &Path,
    mentions: &[EntityMention],
) -> Result<(Option<ContactHash>, Option<ContactHash>)> {
    let email = best_normalized_contact(mentions, EntityType::Email);
    let phone = best_normalized_contact(mentions, EntityType::Phone);
    if email.is_none() && phone.is_none() {
        return Ok((None, None));
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

    Ok((email_hash, phone_hash))
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

fn persist_classification(
    store: &MetaStore,
    document_id: &DocumentId,
    decision: AdmissionDecision,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .upsert_document_classification(&decision.into_record(document_id.clone(), now))
        .map_err(ImportPipelineError::store)
}

fn persist_non_searchable(
    store: &MetaStore,
    document: &Document,
    version: Option<&ResumeVersion>,
    decision: AdmissionDecision,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .upsert_document(document)
        .map_err(ImportPipelineError::store)?;
    store
        .quarantine_document_searchability(&document.id)
        .map_err(ImportPipelineError::store)?;
    if let Some(version) = version {
        store
            .upsert_resume_version(version)
            .map_err(ImportPipelineError::store)?;
    }
    persist_classification(store, &document.id, decision, now)
}

fn persist_failed(store: &MetaStore, document: &Document, now: UnixTimestamp) -> Result<()> {
    persist_non_searchable(store, document, None, AdmissionDecision::failed(), now)
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
    store
        .quarantine_document_searchability(&document.id)
        .map_err(ImportPipelineError::store)?;
    persist_classification(store, &document.id, AdmissionDecision::ocr_backlog(), now)?;
    let enqueue = store
        .enqueue_ocr_job_for_document(&document.id, now)
        .map_err(ImportPipelineError::store)?;

    Ok(enqueue.scheduled)
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

fn exact_rerun_noop_kind(
    store: &MetaStore,
    file: &DiscoveredFile,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<Option<ExactRerunNoopKind>> {
    let Some(document) = store
        .document_by_id(&file.document_id)
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(None);
    };

    if document.is_deleted
        || document.normalized_path != file.normalized_path.as_str()
        || document.file_name != file.file_name
        || document.extension != file.extension
        || document.byte_size != file.byte_size
        || document.mtime != file.mtime
        || document.content_hash.as_deref() != Some(file.fingerprint.as_str())
    {
        return Ok(None);
    }

    let Some(classification) = store
        .document_classification_by_id(&document.id)
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(None);
    };
    if classification
        .classifier_epoch
        .starts_with(resume_classifier::PROMOTED_EPOCH_PREFIX)
        && linear_promotion.classifier_epoch() != Some(classification.classifier_epoch.as_str())
    {
        return Ok(None);
    }

    match document.status {
        DocumentStatus::Searchable | DocumentStatus::IndexedPartial
            if is_current(&classification)
                && classification.status == ClassificationStatus::ResumeCandidate =>
        {
            let has_searchable_version = store
                .resume_versions_for_document(&document.id)
                .map_err(ImportPipelineError::store)?
                .iter()
                .any(|version| version.visibility == ResumeVisibility::Searchable);
            Ok(has_searchable_version.then_some(ExactRerunNoopKind::Searchable))
        }
        DocumentStatus::OcrRequired
            if is_current(&classification)
                && classification.status == ClassificationStatus::OcrBacklog =>
        {
            let job = store
                .ocr_job_for_document(&document.id)
                .map_err(ImportPipelineError::store)?;
            Ok(job
                .as_ref()
                .is_some_and(ocr_job_is_actionable)
                .then_some(ExactRerunNoopKind::OcrRequired))
        }
        DocumentStatus::TextCleaned | DocumentStatus::OcrDone
            if is_current(&classification)
                && matches!(
                    classification.status,
                    ClassificationStatus::NonResume | ClassificationStatus::NeedsReview
                ) =>
        {
            if linear_promotion.enabled()
                && classification.status == ClassificationStatus::NeedsReview
            {
                return Ok(None);
            }
            Ok(Some(ExactRerunNoopKind::Excluded))
        }
        _ => Ok(None),
    }
}

fn ocr_job_is_actionable(job: &IngestJob) -> bool {
    match job.status {
        IngestJobStatus::Queued | IngestJobStatus::Running => true,
        IngestJobStatus::Interrupted | IngestJobStatus::FailedRetryable => {
            job.attempt_count < job.max_attempts
        }
        IngestJobStatus::Completed | IngestJobStatus::FailedPermanent => false,
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
    classify_language_set(text)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn classify_language_set(text: &str) -> Vec<&'static str> {
    let mut has_english = false;
    let mut has_chinese = false;
    for character in text.chars() {
        has_english |= character.is_ascii_alphabetic();
        has_chinese |= is_cjk_character(character);
        if has_english && has_chinese {
            break;
        }
    }

    let mut languages = Vec::new();
    if has_english {
        languages.push("en");
    }
    if has_chinese {
        languages.push("zh");
    }
    if languages.is_empty() {
        languages.push("unknown");
    }
    languages
}

fn is_cjk_character(character: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&character) || ('\u{3400}'..='\u{4dbf}').contains(&character)
}

enum ProcessedFile {
    Searchable {
        document: Box<Document>,
        index_document: Box<IndexDocument>,
    },
    UnchangedSearchable,
    UnchangedOcrRequired,
    UnchangedExcluded,
    Excluded,
    OcrRequired {
        ocr_job_queued: bool,
    },
    Failed {
        kind: ImportFailureKind,
    },
}

enum PreparedFile {
    Ready(ProcessedImportFile),
    Parse(ParseWorkItem),
}

struct ProcessedImportFile {
    file: DiscoveredFile,
    processed: ProcessedFile,
}

enum ImportFileResult {
    Processed(ProcessedImportFile),
    Parsed(ParseWorkResult),
}

struct ParseWorkItem {
    index: usize,
    file: DiscoveredFile,
    document: Document,
    bytes: Vec<u8>,
}

struct ParseWorkResult {
    index: usize,
    file: DiscoveredFile,
    document: Document,
    parse_elapsed: Duration,
    parse_started: Instant,
    parse_finished: Instant,
    pdf_parse_timings: PdfTextExtractionTimings,
    post_parser_timings: ImportPostParserTimings,
    outcome: ParseWorkOutcome,
}

struct ParseWorkItemOutput {
    outcome: ParseWorkOutcome,
    pdf_parse_timings: PdfTextExtractionTimings,
    post_parser_timings: ImportPostParserTimings,
}

enum ParseWorkOutcome {
    Searchable {
        decision: AdmissionDecision,
        version: Box<ResumeVersion>,
        mentions: Vec<EntityMention>,
        index_document: Box<IndexDocument>,
    },
    Excluded {
        decision: AdmissionDecision,
        version: Box<ResumeVersion>,
    },
    OcrRequired,
    Failed {
        status: DocumentStatus,
        kind: ImportFailureKind,
    },
}

#[derive(Default)]
struct ParseWorkerClock {
    first_started: Option<Instant>,
    last_finished: Option<Instant>,
    active_elapsed: Duration,
}

impl ParseWorkerClock {
    fn record_result(&mut self, result: &ParseWorkResult) {
        self.active_elapsed += result.parse_elapsed;
        self.first_started = Some(match self.first_started {
            Some(first_started) => first_started.min(result.parse_started),
            None => result.parse_started,
        });
        self.last_finished = Some(match self.last_finished {
            Some(last_finished) => last_finished.max(result.parse_finished),
            None => result.parse_finished,
        });
    }

    fn worker_wall_elapsed(&self) -> Duration {
        match (self.first_started, self.last_finished) {
            (Some(started), Some(finished)) => finished.saturating_duration_since(started),
            _ => Duration::ZERO,
        }
    }
}

enum ExactRerunNoopKind {
    Searchable,
    OcrRequired,
    Excluded,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub files_discovered: usize,
    pub scan_errors: usize,
    pub ignored_entries: usize,
    pub content_bytes_read: u64,
    pub searchable_documents: usize,
    pub ocr_required_documents: usize,
    pub ocr_jobs_queued: usize,
    pub failed_documents: usize,
    pub failure_counts: ImportFailureCounts,
    pub deleted_documents: usize,
    pub scan_budget: Option<ImportScanBudget>,
    pub stage_timings: ImportStageTimings,
    pub milestone_timings: ImportMilestoneTimings,
    pub worker_metrics: ImportWorkerMetrics,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportStageTimings {
    pub scan: Duration,
    pub parse: Duration,
    pub db: Duration,
    pub index: Duration,
    pub ocr: Duration,
    pub embedding: Duration,
}

impl ImportStageTimings {
    pub fn add_assign(&mut self, next: &Self) {
        self.scan += next.scan;
        self.parse += next.parse;
        self.db += next.db;
        self.index += next.index;
        self.ocr += next.ocr;
        self.embedding += next.embedding;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportMilestoneTimings {
    pub first_searchable: Option<Duration>,
    pub ttf100_searchable: Option<Duration>,
    pub ttf1000_searchable: Option<Duration>,
    pub full_import_ready: Option<Duration>,
    pub full_index_ready: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImportCancelCheckPhase {
    #[default]
    Unattributed,
    ImportSetup,
    Scan,
    SequentialParse,
    ParsePrepare,
    ParseQueueWait,
    ParseResultWait,
    WorkerResultCommit,
    DbWrite,
    IndexPublication,
    IndexPublicationSetup,
    IndexPublicationDocuments,
    IndexPublicationCommit,
    IndexPublicationPlaintextValidation,
    IndexPublicationEncryptedPublication,
    IndexPublicationEncryptedValidation,
    IndexPublicationActiveSnapshot,
}

impl ImportCancelCheckPhase {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Unattributed => "unattributed",
            Self::ImportSetup => "import_setup",
            Self::Scan => "scan",
            Self::SequentialParse => "sequential_parse",
            Self::ParsePrepare => "parse_prepare",
            Self::ParseQueueWait => "parse_queue_wait",
            Self::ParseResultWait => "parse_result_wait",
            Self::WorkerResultCommit => "worker_result_commit",
            Self::DbWrite => "db_write",
            Self::IndexPublication => "index_publication",
            Self::IndexPublicationSetup => "index_publication_setup",
            Self::IndexPublicationDocuments => "index_publication_documents",
            Self::IndexPublicationCommit => "index_publication_commit",
            Self::IndexPublicationPlaintextValidation => "index_publication_plaintext_validation",
            Self::IndexPublicationEncryptedPublication => "index_publication_encrypted_publication",
            Self::IndexPublicationEncryptedValidation => "index_publication_encrypted_validation",
            Self::IndexPublicationActiveSnapshot => "index_publication_active_snapshot",
        }
    }

    fn from_snapshot_publish_phase(phase: SnapshotPublishPhase) -> Self {
        match phase {
            SnapshotPublishPhase::Setup => Self::IndexPublicationSetup,
            SnapshotPublishPhase::DocumentIndexing => Self::IndexPublicationDocuments,
            SnapshotPublishPhase::TantivyCommit => Self::IndexPublicationCommit,
            SnapshotPublishPhase::PlaintextValidation => Self::IndexPublicationPlaintextValidation,
            SnapshotPublishPhase::EncryptedPublication => {
                Self::IndexPublicationEncryptedPublication
            }
            SnapshotPublishPhase::EncryptedValidation => Self::IndexPublicationEncryptedValidation,
            SnapshotPublishPhase::ActiveSnapshotWrite => Self::IndexPublicationActiveSnapshot,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportWorkerMetrics {
    pub parse_worker_count: usize,
    pub parse_jobs_queued: usize,
    pub parse_prepare: Duration,
    pub parse_worker_wall: Duration,
    pub parse_worker_active: Duration,
    pub parse_queue_full_events: usize,
    pub parse_queue_wait: Duration,
    pub parse_result_wait: Duration,
    pub cancel_check_count: usize,
    pub cancel_check_max_gap: Duration,
    pub cancel_check_max_gap_phase: ImportCancelCheckPhase,
    pub index_publication_timings: ImportIndexPublicationTimings,
    pub pdf_parse_timings: PdfTextExtractionTimings,
    pub post_parser_timings: ImportPostParserTimings,
}

impl ImportWorkerMetrics {
    pub fn add_assign(&mut self, next: &Self) {
        self.parse_worker_count = self.parse_worker_count.max(next.parse_worker_count);
        self.parse_jobs_queued += next.parse_jobs_queued;
        self.parse_prepare += next.parse_prepare;
        self.parse_worker_wall += next.parse_worker_wall;
        self.parse_worker_active += next.parse_worker_active;
        self.parse_queue_full_events += next.parse_queue_full_events;
        self.parse_queue_wait += next.parse_queue_wait;
        self.parse_result_wait += next.parse_result_wait;
        self.cancel_check_count += next.cancel_check_count;
        if next.cancel_check_max_gap > self.cancel_check_max_gap {
            self.cancel_check_max_gap = next.cancel_check_max_gap;
            self.cancel_check_max_gap_phase = next.cancel_check_max_gap_phase;
        }
        self.index_publication_timings
            .add_assign(&next.index_publication_timings);
        self.pdf_parse_timings.add_assign(&next.pdf_parse_timings);
        self.post_parser_timings
            .add_assign(&next.post_parser_timings);
    }

    fn record_parse_worker_count(&mut self, count: usize) {
        self.parse_worker_count = self.parse_worker_count.max(count);
    }

    fn record_parse_worker_clock(&mut self, clock: &ParseWorkerClock) {
        self.parse_worker_wall += clock.worker_wall_elapsed();
        self.parse_worker_active += clock.active_elapsed;
    }

    fn record_cancel_checks(&mut self, checks: CancelCheckMetrics) {
        self.cancel_check_count += checks.count;
        if checks.max_gap > self.cancel_check_max_gap {
            self.cancel_check_max_gap = checks.max_gap;
            self.cancel_check_max_gap_phase = checks.max_gap_phase;
        }
    }

    fn record_index_publication_phase_timing(
        &mut self,
        phase: SnapshotPublishPhase,
        elapsed: Duration,
    ) {
        self.index_publication_timings.record(phase, elapsed);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportPostParserTimings {
    pub normalization: Duration,
    pub sectionization: Duration,
}

impl ImportPostParserTimings {
    fn add_assign(&mut self, next: &Self) {
        self.normalization += next.normalization;
        self.sectionization += next.sectionization;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportIndexPublicationTimings {
    pub setup: Duration,
    pub documents: Duration,
    pub commit: Duration,
    pub plaintext_validation: Duration,
    pub encrypted_publication: Duration,
    pub encrypted_validation: Duration,
    pub active_snapshot: Duration,
}

impl ImportIndexPublicationTimings {
    fn record(&mut self, phase: SnapshotPublishPhase, elapsed: Duration) {
        match phase {
            SnapshotPublishPhase::Setup => self.setup += elapsed,
            SnapshotPublishPhase::DocumentIndexing => self.documents += elapsed,
            SnapshotPublishPhase::TantivyCommit => self.commit += elapsed,
            SnapshotPublishPhase::PlaintextValidation => self.plaintext_validation += elapsed,
            SnapshotPublishPhase::EncryptedPublication => self.encrypted_publication += elapsed,
            SnapshotPublishPhase::EncryptedValidation => self.encrypted_validation += elapsed,
            SnapshotPublishPhase::ActiveSnapshotWrite => self.active_snapshot += elapsed,
        }
    }

    fn add_assign(&mut self, next: &Self) {
        self.setup += next.setup;
        self.documents += next.documents;
        self.commit += next.commit;
        self.plaintext_validation += next.plaintext_validation;
        self.encrypted_publication += next.encrypted_publication;
        self.encrypted_validation += next.encrypted_validation;
        self.active_snapshot += next.active_snapshot;
    }
}

#[derive(Debug, Default)]
struct CancelCheckMetrics {
    count: usize,
    previous_check: Option<Instant>,
    previous_phase: ImportCancelCheckPhase,
    max_gap: Duration,
    max_gap_phase: ImportCancelCheckPhase,
}

impl CancelCheckMetrics {
    fn record_check(&mut self, phase: ImportCancelCheckPhase) {
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

    fn index(error: index_fulltext::FullTextError) -> Self {
        if matches!(error, index_fulltext::FullTextError::Cancelled) {
            return Self::cancelled();
        }

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
    use std::collections::BTreeSet;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    };
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use fs_crawler::{crawl_directory, normalize_path, NormalizedPath, ScanProfile};
    use meta_store::{
        Document, DocumentId, DocumentStatus, FileExtension, ImportRootKind, ImportScanProfile,
        ImportScanScope, ImportTask, ImportTaskStatus, IngestJobKind, IngestJobStatus, MetaStore,
        OcrAttemptFailure, UnixTimestamp,
    };

    use super::{
        classify_language_set, current_timestamp_or, document_path_is_deletion_candidate,
        import_root_with_options, index_claimed_ocr_text, persist_classification,
        recv_parse_result_with_cancel_poll, should_flush_searchable_documents,
        take_pending_searchable_documents, write_incremental_full_text_index, AdmissionDecision,
        CachedIndexDocument, CurrentImportIndexCacheMode, CurrentImportIndexDocuments,
        ImportCancelCheckPhase, ImportHardwareProfile, ImportHardwareTier, ImportOptions,
        ImportParseWorkers, ImportPipelineErrorKind, ImportResourcePolicy, IndexDocument,
        IndexSection, OcrTextIndexOutcome, ParseWorkOutcome, ParseWorkResult, SnapshotPublishPhase,
        BYTES_PER_GIB, H2_INDEX_WRITER_HEAP_BYTES,
    };

    #[cfg(unix)]
    static DOC_CONVERTER_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn claim_ocr_document(
        store: &MetaStore,
        document: &Document,
        now: UnixTimestamp,
    ) -> meta_store::ClaimedOcrJob {
        let mut document = document.clone();
        document.status = DocumentStatus::OcrRequired;
        store.upsert_document(&document).unwrap();
        store
            .quarantine_document_searchability(&document.id)
            .unwrap();
        persist_classification(store, &document.id, AdmissionDecision::ocr_backlog(), now).unwrap();
        store
            .enqueue_ocr_job_for_document(&document.id, now)
            .unwrap();
        store.claim_next_ocr_job(now).unwrap().unwrap()
    }

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

    #[test]
    fn searchable_flush_policy_publishes_first_match_then_batches_followups() {
        assert!(should_flush_searchable_documents(0, 100, 1, 0));
        assert!(!should_flush_searchable_documents(8, 100, 8, 1));
        assert!(!should_flush_searchable_documents(31, 1000, 32, 1));
        assert!(should_flush_searchable_documents(99, 1000, 99, 1));
        assert!(!should_flush_searchable_documents(100, 1000, 1, 100));
        assert!(!should_flush_searchable_documents(126, 1000, 27, 100));
        assert!(!should_flush_searchable_documents(127, 1000, 28, 100));
        assert!(!should_flush_searchable_documents(510, 1000, 411, 100));
        assert!(!should_flush_searchable_documents(511, 1000, 412, 100));
        assert!(should_flush_searchable_documents(998, 2000, 900, 100));
        assert!(!should_flush_searchable_documents(999, 2000, 1, 1000));
        assert!(!should_flush_searchable_documents(1022, 2000, 1023, 1000));
        assert!(should_flush_searchable_documents(1023, 2000, 1024, 1000));
        assert!(!should_flush_searchable_documents(999, 1000, 1, 1));
    }

    #[test]
    fn current_import_index_cache_publishes_later_flush_without_active_snapshot_read() {
        let temp = TestDir::new("import-pipeline-current-import-index-cache");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let empty_exclusions = BTreeSet::new();
        let mut current_import_index_documents = CurrentImportIndexDocuments::default();

        let (first_snapshot_token, first_indexed_documents) = write_incremental_full_text_index(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            vec![test_index_document_with_section("doc-1")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportIndexCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
        )
        .unwrap();
        assert_eq!(first_indexed_documents, 1);
        assert_eq!(current_import_index_documents.documents.len(), 1);
        assert_eq!(
            retained_section_text_bytes(&current_import_index_documents),
            0
        );

        fs::remove_dir_all(
            data_dir
                .join("search-index")
                .join("snapshots")
                .join(&first_snapshot_token),
        )
        .unwrap();

        let (_, second_indexed_documents) = write_incremental_full_text_index(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_051),
            vec![test_index_document_with_section("doc-2")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportIndexCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
        )
        .unwrap();

        assert_eq!(second_indexed_documents, 2);
        let cached_doc_ids = current_import_index_documents
            .documents
            .iter()
            .map(|document| document.doc_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(cached_doc_ids, vec!["doc-1", "doc-2"]);
        assert_eq!(
            retained_section_text_bytes(&current_import_index_documents),
            0
        );
    }

    #[test]
    fn current_import_index_cache_consumes_final_flush_documents() {
        let temp = TestDir::new("import-pipeline-current-import-index-cache-final");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let empty_exclusions = BTreeSet::new();
        let mut current_import_index_documents = CurrentImportIndexDocuments::default();

        write_incremental_full_text_index(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            vec![test_index_document_with_section("doc-1")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportIndexCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
        )
        .unwrap();
        assert_eq!(current_import_index_documents.documents.len(), 1);

        let (_, indexed_documents) = write_incremental_full_text_index(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_051),
            vec![test_index_document_with_section("doc-2")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportIndexCacheMode::Consume,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
        )
        .unwrap();

        assert_eq!(indexed_documents, 2);
        assert!(current_import_index_documents.documents.is_empty());
    }

    #[test]
    fn current_import_index_cache_redacts_contact_text_before_retaining() {
        let cached = CachedIndexDocument::from_index_document(IndexDocument {
            doc_id: "doc-contact".to_string(),
            version_id: "ver-contact".to_string(),
            file_name: "person@example.test resume.pdf".to_string(),
            clean_text:
                "Email person@example.test phone +1 650-555-1234 file /Users/private/resume.pdf"
                    .to_string(),
            sections: Vec::new(),
            is_deleted: false,
        });

        assert!(cached.file_name.contains("<redacted-email>"));
        assert!(cached.clean_text.contains("<redacted-email>"));
        assert!(cached.clean_text.contains("<redacted-phone>"));
        assert!(cached.clean_text.contains("<redacted-path>"));
        assert!(!cached.file_name.contains("person@example.test"));
        assert!(!cached.clean_text.contains("person@example.test"));
        assert!(!cached.clean_text.contains("650-555-1234"));
        assert!(!cached.clean_text.contains("/Users/private"));
    }

    #[test]
    fn pending_searchable_documents_are_moved_into_flush_inputs() {
        let mut pending = vec![
            (
                test_document("doc-2", DocumentStatus::TextCleaned),
                test_index_document("doc-2"),
            ),
            (
                test_document("doc-1", DocumentStatus::TextCleaned),
                test_index_document("doc-1"),
            ),
        ];

        let (documents, replacements) = take_pending_searchable_documents(&mut pending);

        assert!(pending.is_empty());
        assert_eq!(
            documents
                .iter()
                .map(|document| document.file_name.as_str())
                .collect::<Vec<_>>(),
            vec!["doc-2.txt", "doc-1.txt"]
        );
        assert_eq!(
            replacements
                .iter()
                .map(|document| document.doc_id.as_str())
                .collect::<Vec<_>>(),
            vec!["doc-2", "doc-1"]
        );
    }

    #[test]
    fn language_set_classifier_preserves_order_and_unknown_fallback() {
        assert_eq!(classify_language_set("Rust 中文简历"), vec!["en", "zh"]);
        assert_eq!(classify_language_set("中文简历"), vec!["zh"]);
        assert_eq!(classify_language_set("Rust resume"), vec!["en"]);
        assert_eq!(classify_language_set("  123 !!!  "), vec!["unknown"]);
    }

    #[test]
    fn import_options_low_memory_default_caps_parse_workers() {
        assert_eq!(
            ImportOptions::low_memory_default_for_available_parallelism(8)
                .parse_workers
                .get(),
            3
        );
        assert_eq!(
            ImportOptions::low_memory_default_for_available_parallelism(2)
                .parse_workers
                .get(),
            2
        );
        assert_eq!(
            ImportOptions::low_memory_default_for_available_parallelism(1)
                .parse_workers
                .get(),
            1
        );
        assert_eq!(ImportParseWorkers::new(99).get(), 3);
    }

    #[test]
    fn import_resource_policy_classifies_ram_and_cpu_tiers() {
        let h0 = ImportResourcePolicy::for_hardware(ImportHardwareProfile::new(
            Some(8 * BYTES_PER_GIB),
            10,
        ));
        assert_eq!(h0.hardware_tier, ImportHardwareTier::H0Eco);
        assert_eq!(h0.parse_workers.get(), 1);
        assert_eq!(h0.index_writer_heap_bytes, 64 * 1024 * 1024);
        assert_eq!(h0.max_private_or_anonymous_mb, 512);

        let h1 = ImportResourcePolicy::for_hardware(ImportHardwareProfile::new(
            Some(16 * BYTES_PER_GIB),
            10,
        ));
        assert_eq!(h1.hardware_tier, ImportHardwareTier::H1Balanced);
        assert_eq!(h1.parse_workers.get(), 2);
        assert_eq!(h1.index_writer_heap_bytes, 128 * 1024 * 1024);
        assert_eq!(h1.max_private_or_anonymous_mb, 1024);

        let h2 = ImportResourcePolicy::for_hardware(ImportHardwareProfile::new(
            Some(32 * BYTES_PER_GIB),
            10,
        ));
        assert_eq!(h2.hardware_tier, ImportHardwareTier::H2Aggressive);
        assert_eq!(h2.parse_workers.get(), 3);
        assert_eq!(h2.index_writer_heap_bytes, 256 * 1024 * 1024);
        assert_eq!(h2.max_private_or_anonymous_mb, 1536);

        let high_memory_single_core = ImportResourcePolicy::for_hardware(
            ImportHardwareProfile::new(Some(32 * BYTES_PER_GIB), 1),
        );
        assert_eq!(
            high_memory_single_core.hardware_tier,
            ImportHardwareTier::H2Aggressive
        );
        assert_eq!(high_memory_single_core.parse_workers.get(), 1);

        let h2_options = ImportOptions::for_resource_policy(h2);
        assert_eq!(h2_options.index_writer_heap_bytes, 256 * 1024 * 1024);
    }

    #[test]
    fn snapshot_publish_phases_map_to_import_cancel_subphase_labels() {
        for (phase, expected_label) in [
            (SnapshotPublishPhase::Setup, "index_publication_setup"),
            (
                SnapshotPublishPhase::DocumentIndexing,
                "index_publication_documents",
            ),
            (
                SnapshotPublishPhase::TantivyCommit,
                "index_publication_commit",
            ),
            (
                SnapshotPublishPhase::PlaintextValidation,
                "index_publication_plaintext_validation",
            ),
            (
                SnapshotPublishPhase::EncryptedPublication,
                "index_publication_encrypted_publication",
            ),
            (
                SnapshotPublishPhase::EncryptedValidation,
                "index_publication_encrypted_validation",
            ),
            (
                SnapshotPublishPhase::ActiveSnapshotWrite,
                "index_publication_active_snapshot",
            ),
        ] {
            assert_eq!(
                ImportCancelCheckPhase::from_snapshot_publish_phase(phase).as_label(),
                expected_label
            );
        }
    }

    #[test]
    fn import_root_persists_clean_text_without_duplicate_raw_text_body() {
        let temp = TestDir::new("import-pipeline-no-duplicate-raw-text");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust Search"),
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_075);
        let task = import_task("no-duplicate-raw-text-import", root.to_str().unwrap(), now);
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

        assert_eq!(summary.searchable_documents, 1);
        let document = store.visible_documents().unwrap().remove(0);
        let version = store
            .latest_visible_resume_version_for_document(&document.id)
            .unwrap()
            .unwrap();
        assert!(version
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Rust Search"));
        assert_eq!(version.raw_text, None);
        let claim = claim_ocr_document(&store, &document, now);
        let OcrTextIndexOutcome::Committed(rejected) = index_claimed_ocr_text(
            &data_dir,
            &store,
            &claim,
            "INVOICE\nSubtotal 10\nPayment terms net 30",
            None,
            Some(1),
            UnixTimestamp::from_unix_seconds(1_700_000_077),
        )
        .unwrap() else {
            panic!("current OCR attempt was superseded");
        };
        assert!(!rejected.searchable);
        assert_eq!(
            store.document_classification_counts().unwrap().non_resume,
            1
        );
        assert!(store
            .latest_visible_resume_version_for_document(&document.id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn classifier_gate_persists_all_five_states_before_search_admission() {
        let temp = TestDir::new("classifier-gate-five-states");
        let root = temp.path().join("mixed");
        fs::create_dir_all(&root).unwrap();
        for (name, body) in [
            (
                "resume.txt",
                synthetic_resume_text("Synthetic Candidate", "Rust Search"),
            ),
            (
                "invoice.txt",
                "INVOICE\nInvoice number 7\nSubtotal 10\nPayment terms net 30".to_string(),
            ),
            (
                "review.txt",
                "Project notes\nUnstructured material".to_string(),
            ),
            ("empty.txt", String::new()),
        ] {
            fs::write(root.join(name), body).unwrap();
        }
        fs::write(root.join("scan.pdf"), scanned_pdf_bytes()).unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_078);
        for workers in [1, 2] {
            let data_dir = temp.path().join(format!("data-{workers}"));
            fs::create_dir_all(&data_dir).unwrap();
            let store = MetaStore::open_in_memory().unwrap();
            store.run_migrations().unwrap();
            let task = import_task(&format!("gate-{workers}"), root.to_str().unwrap(), now);
            store.insert_import_task(&task).unwrap();
            let options = ImportOptions::low_memory_default_for_available_parallelism(workers);
            import_root_with_options(&data_dir, &store, &task, &root, now, options).unwrap();
            let counts = store.document_classification_counts().unwrap();
            assert_eq!(
                (
                    counts.resume_candidate,
                    counts.non_resume,
                    counts.needs_review,
                    counts.ocr_backlog,
                    counts.failed
                ),
                (1, 1, 1, 1, 1)
            );
            assert_eq!(store.searchable_document_ids().unwrap().len(), 1);
        }
    }

    #[test]
    fn parallel_parse_workers_preserve_searchable_and_ocr_counts() {
        let temp = TestDir::new("import-pipeline-parallel-parse-counts");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("00-alpha.txt"),
            synthetic_resume_text("Alpha Candidate", "Rust Search"),
        )
        .unwrap();
        fs::write(root.join("01-scanned.pdf"), scanned_pdf_bytes()).unwrap();
        fs::write(
            root.join("02-beta.txt"),
            synthetic_resume_text("Beta Candidate", "PDF Search"),
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_078);
        let task = import_task("parallel-parse-import", root.to_str().unwrap(), now);
        store.insert_import_task(&task).unwrap();

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions {
                parse_workers: ImportParseWorkers::new(2),
                ..ImportOptions::default()
            },
        )
        .unwrap();

        assert_eq!(summary.files_discovered, 3);
        assert_eq!(summary.searchable_documents, 2);
        assert_eq!(summary.ocr_required_documents, 1);
        assert_eq!(summary.ocr_jobs_queued, 1);
        assert_eq!(summary.failed_documents, 0);
        assert!(summary.milestone_timings.first_searchable.is_some());
        assert!(summary.milestone_timings.full_import_ready.is_some());

        let status = store.status_summary().unwrap();
        assert_eq!(status.searchable_documents, 2);
        assert_eq!(status.ocr_queue_depth, 1);
        let documents = store.visible_documents().unwrap();
        let mut visible_text = String::new();
        for document in documents {
            if let Some(version) = store
                .latest_visible_resume_version_for_document(&document.id)
                .unwrap()
            {
                visible_text.push_str(version.clean_text.as_deref().unwrap_or_default());
            }
        }
        assert!(visible_text.contains("Alpha Candidate"));
        assert!(visible_text.contains("Beta Candidate"));
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn parallel_parse_workers_record_queue_and_cancel_evidence() {
        let temp = TestDir::new("import-pipeline-parallel-parse-evidence");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("00-alpha.txt"),
            synthetic_resume_text("Alpha Candidate", "Rust Search"),
        )
        .unwrap();
        fs::write(root.join("01-scanned.pdf"), scanned_pdf_bytes()).unwrap();
        fs::write(
            root.join("02-beta.txt"),
            synthetic_resume_text("Beta Candidate", "PDF Search"),
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_079);
        let task = import_task(
            "parallel-parse-evidence-import",
            root.to_str().unwrap(),
            now,
        );
        store.insert_import_task(&task).unwrap();

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions {
                parse_workers: ImportParseWorkers::new(2),
                ..ImportOptions::default()
            },
        )
        .unwrap();

        assert_eq!(summary.worker_metrics.parse_worker_count, 2);
        assert_eq!(summary.worker_metrics.parse_jobs_queued, 2);
        assert!(summary.worker_metrics.parse_prepare > Duration::ZERO);
        assert!(summary.worker_metrics.parse_worker_wall > Duration::ZERO);
        assert!(summary.worker_metrics.parse_worker_active > Duration::ZERO);
        assert!(summary.stage_timings.parse >= summary.worker_metrics.parse_worker_wall);
        assert!(summary.worker_metrics.cancel_check_count > 0);
        assert!(summary.worker_metrics.cancel_check_max_gap >= Duration::ZERO);
        assert_ne!(
            summary.worker_metrics.cancel_check_max_gap_phase,
            ImportCancelCheckPhase::Unattributed
        );
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn parallel_parse_workers_record_pdf_and_post_parser_phase_timings() {
        let temp = TestDir::new("import-pipeline-parse-phase-evidence");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("00-scanned.pdf"), scanned_pdf_bytes()).unwrap();
        fs::write(root.join("01-alpha.pdf"), tounicode_cmap_pdf_bytes()).unwrap();
        fs::write(
            root.join("02-beta.txt"),
            synthetic_resume_text("Beta Candidate", "Rust Search"),
        )
        .unwrap();
        fs::write(
            root.join("03-gamma.txt"),
            synthetic_resume_text("Gamma Candidate", "PDF Search"),
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_080);
        let task = import_task("parse-phase-evidence-import", root.to_str().unwrap(), now);
        store.insert_import_task(&task).unwrap();

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions {
                parse_workers: ImportParseWorkers::new(2),
                ..ImportOptions::default()
            },
        )
        .unwrap();

        assert_eq!(summary.worker_metrics.parse_worker_count, 2);
        assert_eq!(summary.searchable_documents, 3);
        for (label, elapsed) in [
            (
                "document_load",
                summary.worker_metrics.pdf_parse_timings.document_load,
            ),
            (
                "page_content_fetch",
                summary.worker_metrics.pdf_parse_timings.page_content_fetch,
            ),
            (
                "text_operator_prefilter",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .text_operator_prefilter,
            ),
            (
                "font_encoding",
                summary.worker_metrics.pdf_parse_timings.font_encoding,
            ),
            (
                "content_decode",
                summary.worker_metrics.pdf_parse_timings.content_decode,
            ),
            (
                "content_string_parse",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .content_string_parse,
            ),
            (
                "text_collection",
                summary.worker_metrics.pdf_parse_timings.text_collection,
            ),
            (
                "text_byte_decode",
                summary.worker_metrics.pdf_parse_timings.text_byte_decode,
            ),
            (
                "text_accumulation",
                summary.worker_metrics.pdf_parse_timings.text_accumulation,
            ),
            (
                "normalization",
                summary.worker_metrics.post_parser_timings.normalization,
            ),
            (
                "sectionization",
                summary.worker_metrics.post_parser_timings.sectionization,
            ),
        ] {
            assert!(
                elapsed > Duration::ZERO,
                "{label} timing should be recorded: {summary:?}"
            );
        }
        for (label, count) in [
            (
                "content_string_operands",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .content_string_operands,
            ),
            (
                "content_string_bytes",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .content_string_bytes,
            ),
            (
                "text_decode_runs",
                summary.worker_metrics.pdf_parse_timings.text_decode_runs,
            ),
            (
                "text_decode_input_bytes",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .text_decode_input_bytes,
            ),
        ] {
            assert!(count > 0, "{label} counter should be recorded: {summary:?}");
        }
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn parse_worker_clock_reports_wall_clock_separate_from_active_sum() {
        let temp = TestDir::new("import-pipeline-parse-worker-clock");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("clock.txt"),
            synthetic_resume_text("Clock Candidate", "Rust Search"),
        )
        .unwrap();
        let file = crawl_directory(&root).unwrap().files.remove(0);
        let document = test_document("clock", DocumentStatus::Searchable);
        let started = Instant::now();
        let mut clock = super::ParseWorkerClock::default();

        clock.record_result(&ParseWorkResult {
            index: 0,
            file: file.clone(),
            document: document.clone(),
            parse_elapsed: Duration::from_millis(100),
            parse_started: started,
            parse_finished: started + Duration::from_millis(100),
            pdf_parse_timings: parser_pdf::PdfTextExtractionTimings::default(),
            post_parser_timings: crate::ImportPostParserTimings::default(),
            outcome: ParseWorkOutcome::OcrRequired,
        });
        clock.record_result(&ParseWorkResult {
            index: 1,
            file,
            document,
            parse_elapsed: Duration::from_millis(100),
            parse_started: started + Duration::from_millis(10),
            parse_finished: started + Duration::from_millis(110),
            pdf_parse_timings: parser_pdf::PdfTextExtractionTimings::default(),
            post_parser_timings: crate::ImportPostParserTimings::default(),
            outcome: ParseWorkOutcome::OcrRequired,
        });

        assert_eq!(clock.active_elapsed, Duration::from_millis(200));
        assert_eq!(clock.worker_wall_elapsed(), Duration::from_millis(110));
    }

    #[test]
    fn cancel_check_max_gap_is_attributed_to_previous_phase() {
        let mut metrics = super::CancelCheckMetrics::default();

        metrics.record_check(ImportCancelCheckPhase::SequentialParse);
        thread::sleep(Duration::from_millis(2));
        metrics.record_check(ImportCancelCheckPhase::DbWrite);

        assert_eq!(metrics.count, 2);
        assert_eq!(
            metrics.max_gap_phase,
            ImportCancelCheckPhase::SequentialParse
        );
    }

    #[test]
    fn cancel_poller_reuses_cached_state_within_interval() {
        let started = Instant::now();
        let mut poller = super::ImportCancelPoller::new(Duration::from_millis(25));
        let probes = AtomicUsize::new(0);

        assert!(!poller
            .poll(started, || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(false)
            })
            .unwrap());
        assert!(!poller
            .poll(started + Duration::from_millis(10), || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            })
            .unwrap());
        assert!(poller
            .poll(started + Duration::from_millis(30), || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            })
            .unwrap());

        assert_eq!(probes.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn cancel_poller_keeps_cancelled_state_without_requery() {
        let started = Instant::now();
        let mut poller = super::ImportCancelPoller::new(Duration::from_millis(25));
        let probes = AtomicUsize::new(0);

        assert!(poller
            .poll(started, || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            })
            .unwrap());
        assert!(poller
            .poll(started + Duration::from_millis(30), || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(false)
            })
            .unwrap());

        assert_eq!(probes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn recv_parse_result_polls_cancel_while_waiting() {
        let temp = TestDir::new("import-pipeline-parse-result-cancel-poll");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("wait.txt"),
            synthetic_resume_text("Wait Candidate", "Rust Search"),
        )
        .unwrap();
        let file = crawl_directory(&root).unwrap().files.remove(0);
        let document = test_document("wait", DocumentStatus::Searchable);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let cancel_polls = Arc::new(AtomicUsize::new(0));
        let observed_cancel_polls = Arc::clone(&cancel_polls);
        let sender = thread::spawn(move || {
            release_rx.recv().unwrap();
            let parse_started = Instant::now();
            result_tx
                .send(ParseWorkResult {
                    index: 7,
                    file,
                    document,
                    parse_elapsed: Duration::from_millis(1),
                    parse_started,
                    parse_finished: parse_started + Duration::from_millis(1),
                    pdf_parse_timings: parser_pdf::PdfTextExtractionTimings::default(),
                    post_parser_timings: crate::ImportPostParserTimings::default(),
                    outcome: ParseWorkOutcome::OcrRequired,
                })
                .unwrap();
        });

        let result = recv_parse_result_with_cancel_poll(&result_rx, &|| {
            let poll = observed_cancel_polls.fetch_add(1, Ordering::SeqCst) + 1;
            if poll == 2 {
                release_tx.send(()).unwrap();
            }
            Ok(())
        })
        .unwrap();
        sender.join().unwrap();

        assert_eq!(result.index, 7);
        assert!(
            cancel_polls.load(Ordering::SeqCst) >= 2,
            "expected repeated cancellation checks while waiting for parse result"
        );
    }

    #[test]
    fn index_ocr_text_persists_clean_text_without_duplicate_raw_text_body() {
        let temp = TestDir::new("import-pipeline-ocr-no-duplicate-raw-text");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let document = test_document("ocr-doc", DocumentStatus::OcrRequired);
        let stale = claim_ocr_document(
            &store,
            &document,
            UnixTimestamp::from_unix_seconds(1_700_000_075),
        );
        store
            .finish_ocr_attempt_failure(
                &stale,
                OcrAttemptFailure::Retryable,
                UnixTimestamp::from_unix_seconds(1_700_000_075),
            )
            .unwrap();
        let current = store
            .claim_next_ocr_job(UnixTimestamp::from_unix_seconds(1_700_000_076))
            .unwrap()
            .unwrap();
        assert_eq!(
            index_claimed_ocr_text(
                &data_dir,
                &store,
                &stale,
                "stale OCR output",
                Some(0.99),
                Some(1),
                UnixTimestamp::from_unix_seconds(1_700_000_076),
            )
            .unwrap(),
            OcrTextIndexOutcome::Superseded
        );

        let OcrTextIndexOutcome::Committed(summary) = index_claimed_ocr_text(
            &data_dir,
            &store,
            &current,
            &synthetic_resume_text("OCR Candidate", "Rust Search"),
            Some(0.91),
            Some(1),
            UnixTimestamp::from_unix_seconds(1_700_000_076),
        )
        .unwrap() else {
            panic!("current OCR attempt was superseded");
        };

        assert!(summary.searchable);
        let version = store
            .latest_visible_resume_version_for_document(&document.id)
            .unwrap()
            .unwrap();
        assert!(version
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Rust Search"));
        assert_eq!(version.raw_text, None);
    }

    fn test_index_document(doc_id: &str) -> IndexDocument {
        IndexDocument {
            doc_id: doc_id.to_string(),
            version_id: format!("{doc_id}-version"),
            file_name: format!("{doc_id}.txt"),
            clean_text: format!("Synthetic Candidate {doc_id}\\nSkills: Rust Search"),
            sections: Vec::new(),
            is_deleted: false,
        }
    }

    fn test_index_document_with_section(doc_id: &str) -> IndexDocument {
        let mut document = test_index_document(doc_id);
        document.sections = vec![IndexSection {
            section_type: "skills".to_string(),
            text: format!("Rust Search section for {doc_id}"),
        }];
        document
    }

    fn retained_section_text_bytes(documents: &CurrentImportIndexDocuments) -> usize {
        documents
            .documents
            .iter()
            .flat_map(|document| document.sections.iter())
            .map(|section| section.text.len())
            .sum()
    }

    fn test_document(doc_id: &str, status: DocumentStatus) -> Document {
        Document {
            id: DocumentId::from_non_secret_parts(&[doc_id]),
            source_uri: format!("file:///fixture/{doc_id}.txt"),
            normalized_path: format!("/fixture/{doc_id}.txt"),
            file_name: format!("{doc_id}.txt"),
            extension: FileExtension::Txt,
            byte_size: 128,
            mtime: UnixTimestamp::from_unix_seconds(1_700_000_001),
            content_hash: Some(format!("{doc_id}-content")),
            text_hash: None,
            is_deleted: false,
            created_at: UnixTimestamp::from_unix_seconds(1_700_000_000),
            updated_at: UnixTimestamp::from_unix_seconds(1_700_000_000),
            status,
        }
    }

    fn synthetic_resume_text(candidate: &str, skills: &str) -> String {
        format!("SUMMARY\n{candidate}\nEXPERIENCE\nBuilt {skills} systems\nSKILLS\n{skills}")
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
            synthetic_resume_text("Synthetic Candidate", "Rust"),
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
            synthetic_resume_text("Synthetic Candidate", "Rust"),
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
            synthetic_resume_text("Synthetic Candidate", "Rust"),
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
        assert_eq!(second_summary.searchable_documents, 1);
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
    fn import_root_rerun_with_unchanged_ocr_required_file_requeues_only_terminal_job() {
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
        assert!(second_index_state.visible_epoch > first_index_state.visible_epoch);
        assert_eq!(
            second_index_state.manifest_document_count,
            first_index_state.manifest_document_count
        );
        assert_ne!(
            second_index_state.snapshot_token,
            first_index_state.snapshot_token
        );
        assert!(!format!("{second_index_state:?}").contains(root.to_str().unwrap()));

        let claimed_at = UnixTimestamp::from_unix_seconds(1_700_000_197);
        let claimed = store
            .claim_next_job_by_kind(IngestJobKind::OcrDocument, claimed_at)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.status, IngestJobStatus::Running);
        assert_eq!(claimed.attempt_count, 1);
        store
            .update_job_status(
                &claimed.id,
                IngestJobStatus::Completed,
                UnixTimestamp::from_unix_seconds(1_700_000_198),
            )
            .unwrap();

        let third_now = UnixTimestamp::from_unix_seconds(1_700_000_199);
        let third_task = import_task(
            "zero-change-ocr-third-import",
            root.to_str().unwrap(),
            third_now,
        );
        store.insert_import_task(&third_task).unwrap();
        let third_summary = import_root_with_options(
            &data_dir,
            &store,
            &third_task,
            &root,
            third_now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(third_summary.ocr_required_documents, 1);
        assert_eq!(third_summary.ocr_jobs_queued, 1);
        let requeued = store.ingest_job_by_id(&claimed.id).unwrap().unwrap();
        assert_eq!(requeued.status, IngestJobStatus::Queued);
        assert_eq!(requeued.attempt_count, 1);
        assert_eq!(requeued.queued_at, third_now);
        let reclaimed = store
            .claim_next_job_by_kind(
                IngestJobKind::OcrDocument,
                UnixTimestamp::from_unix_seconds(1_700_000_200),
            )
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.attempt_count, 2);
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
                    "SUMMARY\nSynthetic Candidate {index}\nEXPERIENCE\nBuilt Rust systems\nSKILLS\nRust"
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
        assert!(index_state_debug.contains("visible_epoch: 1"));
        assert!(index_state_debug.contains("manifest_document_count: 1"));
        assert_eq!(summary.searchable_documents, 33);
        assert!(final_index_state_debug.contains("visible_epoch: 2"));
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
            synthetic_resume_text("Synthetic Candidate", "Rust"),
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
        assert!(summary.milestone_timings.first_searchable.is_some());
        assert!(summary.milestone_timings.full_import_ready.is_some());
        assert!(summary.milestone_timings.full_index_ready.is_some());
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
        let mut content = b"BT /F1 12 Tf 72 720 Td (SUMMARY) Tj T* (EXPERIENCE) Tj T* (Built systems) Tj T* (SKILLS) Tj T* (".to_vec();
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
        let content = b"BT /F2 12 Tf 72 720 Td (SUMMARY) Tj T* (EXPERIENCE) Tj T* (Built systems) Tj T* (SKILLS) Tj T* /F1 12 Tf <0001000200030004> Tj ET\n";

        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R /F2 9 0 R >> >> /MediaBox [0 0 612 792] /Contents 7 0 R >>".to_vec(),
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
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
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
printf 'SUMMARY\nSynthetic Legacy Candidate\nEXPERIENCE\nBuilt Rust Search systems\nSKILLS\nRust Search\n' > "$out"
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
printf 'SUMMARY\nSlow Synthetic Legacy Candidate\nEXPERIENCE\nBuilt Rust Search systems\nSKILLS\nRust Search\n' > "$out"
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
