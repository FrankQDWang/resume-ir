// Import orchestration passes stage state explicitly; split this before tightening
// these shape lints for the crate.
#![allow(clippy::too_many_arguments, clippy::large_enum_variant)]

mod classification;
mod immutable_ingest;
mod index_publication;
mod index_recovery;
mod search_artifact_cache;
mod search_artifacts;
mod search_publication;
mod search_vectorizer;

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
use index_fulltext::{IndexDocument, IndexSection, SnapshotPublishPhase};
use meta_store::{
    ClaimedOcrJob, ClassificationStatus, ContactHash, ContentDigest, CurrentClassifierEpoch,
    Document, DocumentId, DocumentStatus, EntityMention, EntityType, FileExtension,
    ImportScanBudgetKind as StoreImportScanBudgetKind, ImportScanError, ImportScanErrorKind,
    ImportScanErrorOperation, ImportTask, ImportTaskId, ImportTaskStatus, IngestJob,
    IngestJobStatus, MetaStore, OcrAttemptPublication, OcrAttemptSuccessOutcome, ResumeVersion,
    ResumeVersionClassification, ResumeVersionId, SourceRevision, UnixTimestamp,
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

use classification::AdmissionDecision;
use immutable_ingest::{resume_version, source_revision, StagedDerivedData, StagedResume};
use index_publication::SearchPublicationLock;
pub use index_recovery::{reconcile_search_artifacts, SearchArtifactRecoverySummary};
use search_artifact_cache::{CurrentImportCacheMode, CurrentImportDocumentCache};
use search_artifacts::{write_incremental_search_artifacts, write_rebuilt_search_artifacts};
use search_publication::commit_prepared_search_publication;
pub use search_vectorizer::{
    SearchPublicationEmbeddingFailure, SearchPublicationEmbeddingInput,
    SearchPublicationEmbeddingOutput, SearchPublicationVectorization, SearchPublicationVectorizer,
};

const PARSE_VERSION: &str = "parser-v1";
const OCR_PARSE_VERSION: &str = "ocr-v1";
const SCHEMA_VERSION: &str = "resume-ir-s9-v1";
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
const H0_MEMORY_CEILING_BYTES: u64 = 12 * BYTES_PER_GIB;
const H1_MEMORY_CEILING_BYTES: u64 = 20 * BYTES_PER_GIB;

pub fn crate_name() -> &'static str {
    "import-pipeline"
}

pub type Result<T> = std::result::Result<T, ImportPipelineError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchProjectionRemovalReason {
    ConfirmedSourceDeletion,
    PermanentClassificationExclusion,
    PrivacyRevocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchProjectionRemoval {
    pub document_id: DocumentId,
    pub reason: SearchProjectionRemovalReason,
}

struct ScheduledProjectionRemoval {
    reason: SearchProjectionRemovalReason,
    document_update: Option<Document>,
}

#[derive(Default)]
struct PendingProjectionRemovals(BTreeMap<DocumentId, ScheduledProjectionRemoval>);

impl PendingProjectionRemovals {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn schedule(
        &mut self,
        document_id: DocumentId,
        reason: SearchProjectionRemovalReason,
        document_update: Option<Document>,
    ) -> Result<()> {
        if let Some(existing) = self.0.get_mut(&document_id) {
            if existing.reason != reason {
                return Err(ImportPipelineError::store_invariant());
            }
            match (&existing.document_update, document_update) {
                (Some(existing), Some(replacement)) if existing != &replacement => {
                    return Err(ImportPipelineError::store_invariant());
                }
                (None, Some(replacement)) => existing.document_update = Some(replacement),
                _ => {}
            }
            return Ok(());
        }
        self.0.insert(
            document_id,
            ScheduledProjectionRemoval {
                reason,
                document_update,
            },
        );
        Ok(())
    }

    fn document_ids(&self) -> BTreeSet<String> {
        self.0
            .keys()
            .map(|document_id| document_id.as_str().to_string())
            .collect()
    }

    fn clear(&mut self) {
        self.0.clear();
    }

    fn publication_documents(&self) -> impl Iterator<Item = &Document> {
        self.0
            .values()
            .filter_map(|removal| removal.document_update.as_ref())
    }
}

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
    let mut pending_excluded_doc_ids = PendingProjectionRemovals::default();

    let total_files = report.files.len();
    let mut current_import_index_documents = CurrentImportDocumentCache::default();
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
            &options.search_vectorization,
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
            &options.search_vectorization,
            &options.linear_promotion,
        )?;
    }

    if can_propagate_deletions {
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
        data_dir,
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
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
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
            search_vectorization,
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
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
    current_import_index_documents: &mut CurrentImportDocumentCache,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    parse_workers: ImportParseWorkers,
    index_writer_heap_bytes: usize,
    search_vectorization: &SearchPublicationVectorization,
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
            search_vectorization,
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
            search_vectorization,
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
            search_vectorization,
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
                search_vectorization,
                linear_promotion,
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
                search_vectorization,
                linear_promotion,
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
                search_vectorization,
                linear_promotion,
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
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
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
            search_vectorization,
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
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
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
    match processed {
        ProcessedFile::Searchable { pending } => {
            pending_index_documents.push(*pending);
        }
        ProcessedFile::OcrRequired { ocr_job_queued } => {
            summary.ocr_required_documents += 1;
            if ocr_job_queued {
                summary.ocr_jobs_queued += 1;
            }
        }
        ProcessedFile::Failed { kind } => {
            summary.failed_documents += 1;
            summary.failure_counts.increment(kind);
        }
        ProcessedFile::Excluded { document } => {
            pending_excluded_doc_ids.schedule(
                file.document_id.clone(),
                SearchProjectionRemovalReason::PermanentClassificationExclusion,
                Some(*document),
            )?;
        }
        ProcessedFile::UnchangedExcluded => {
            pending_excluded_doc_ids.schedule(
                file.document_id.clone(),
                SearchProjectionRemovalReason::PermanentClassificationExclusion,
                None,
            )?;
        }
        ProcessedFile::UnchangedOcrRequired => {}
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
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
    current_import_index_documents: Option<&mut CurrentImportDocumentCache>,
    current_import_index_cache_mode: CurrentImportCacheMode,
    ensure_not_cancelled: &dyn Fn() -> Result<()>,
    set_cancel_phase: &dyn Fn(ImportCancelCheckPhase),
    import_started: Instant,
    index_writer_heap_bytes: usize,
    search_vectorization: &SearchPublicationVectorization,
) -> Result<bool> {
    let has_delta = !pending_index_documents.is_empty() || !pending_excluded_doc_ids.is_empty();
    let needs_initial_publication = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?
        .generation
        .is_none();
    if !has_delta && !needs_initial_publication {
        return Ok(false);
    }
    let classifier_epoch = publication_classifier_epoch(store, pending_index_documents)?;

    if !pending_index_documents.is_empty() {
        set_cancel_phase(ImportCancelCheckPhase::DbWrite);
        ensure_not_cancelled()?;
        measure_result_stage(&mut summary.stage_timings.db, || {
            for pending in pending_index_documents.iter() {
                immutable_ingest::stage(
                    store,
                    StagedResume {
                        document: &pending.document,
                        source_revision: &pending.source_revision,
                        derived: StagedDerivedData::ClassifiedVersion {
                            version: &pending.version,
                            classification: &pending.classification,
                            mentions: &pending.mentions,
                            email_hash: pending.email_hash.as_ref(),
                            phone_hash: pending.phone_hash.as_ref(),
                        },
                    },
                )
                .map_err(ImportPipelineError::store)?;
            }
            Ok(())
        })?;
    }

    set_cancel_phase(ImportCancelCheckPhase::IndexPublication);
    ensure_not_cancelled()?;
    let removed_document_ids = pending_excluded_doc_ids.document_ids();
    let searchable_before = summary.searchable_documents;
    let (mut pending_documents, pending_replacements) =
        take_pending_searchable_documents(pending_index_documents);
    let phase_worker_metrics = RefCell::new(ImportWorkerMetrics::default());
    let record_phase_timing = |phase, elapsed| {
        phase_worker_metrics
            .borrow_mut()
            .record_index_publication_phase_timing(phase, elapsed);
    };
    let index_started = Instant::now();
    let write_result = write_incremental_search_artifacts(
        data_dir,
        store,
        now,
        &classifier_epoch,
        pending_replacements,
        &removed_document_ids,
        summary.ocr_required_documents,
        summary.deleted_documents,
        current_import_index_documents,
        current_import_index_cache_mode,
        Some(ensure_not_cancelled),
        Some(set_cancel_phase),
        Some(&record_phase_timing),
        index_writer_heap_bytes,
        search_vectorization,
    );
    summary.stage_timings.index += index_started.elapsed();
    summary
        .worker_metrics
        .add_assign(&phase_worker_metrics.into_inner());
    let publication = write_result?;

    set_cancel_phase(ImportCancelCheckPhase::DbWrite);
    for document in &mut pending_documents {
        ensure_not_cancelled()?;
        document.status = DocumentStatus::Searchable;
        document.updated_at = now;
    }
    let new_searchable_count = pending_documents.len();
    pending_documents.extend(pending_excluded_doc_ids.publication_documents().cloned());

    set_cancel_phase(ImportCancelCheckPhase::DbWrite);
    ensure_not_cancelled()?;
    let committed_publication = measure_result_stage(&mut summary.stage_timings.db, || {
        commit_prepared_search_publication(store, now, publication, &pending_documents)
    })?;
    committed_publication.release();
    summary.searchable_documents += new_searchable_count;
    pending_excluded_doc_ids.clear();
    let index_ready_elapsed = import_started.elapsed();
    record_searchable_milestones(
        &mut summary.milestone_timings,
        searchable_before,
        summary.searchable_documents,
        index_ready_elapsed,
    );
    Ok(true)
}

fn publication_classifier_epoch(
    store: &MetaStore,
    pending: &[PendingSearchableDocument],
) -> Result<String> {
    let pending_epochs = pending
        .iter()
        .map(|document| document.classification.classifier_epoch.as_str())
        .collect::<BTreeSet<_>>();
    if pending_epochs.len() > 1 {
        return Err(ImportPipelineError::store_invariant());
    }
    let pending_epoch = pending_epochs.first().copied();
    let current_epoch = store
        .search_projection_state()
        .map_err(ImportPipelineError::store)?
        .publication
        .map(|publication| publication.classifier_epoch.clone());
    if let (Some(pending_epoch), Some(current_epoch)) = (pending_epoch, current_epoch.as_deref()) {
        if pending_epoch != current_epoch {
            return Err(ImportPipelineError::store_invariant());
        }
    }
    Ok(pending_epoch
        .map(str::to_string)
        .or(current_epoch)
        .unwrap_or_else(|| resume_classifier::CLASSIFIER_EPOCH.to_string()))
}

fn take_pending_searchable_documents(
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
) -> (Vec<Document>, Vec<IndexDocument>) {
    let pending = std::mem::take(pending_index_documents);
    let mut documents = Vec::with_capacity(pending.len());
    let mut index_documents = Vec::with_capacity(pending.len());
    for pending in pending {
        documents.push(pending.document);
        index_documents.push(pending.index_document);
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
    pub search_vectorization: SearchPublicationVectorization,
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
            search_vectorization: SearchPublicationVectorization::default(),
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
            search_vectorization: SearchPublicationVectorization::default(),
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

pub fn rebuild_search_artifacts(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
) -> Result<SearchArtifactPublicationSummary> {
    let publication_lock =
        SearchPublicationLock::acquire(data_dir).map_err(|_| ImportPipelineError::index_io())?;
    let classifier_epoch = publication_classifier_epoch(store, &[])?;
    let publication = write_rebuilt_search_artifacts(
        data_dir,
        store,
        now,
        &classifier_epoch,
        publication_lock,
        &BTreeSet::new(),
        Vec::new(),
        vectorization,
    )?;
    let publication = commit_prepared_search_publication(store, now, publication, &[])?.release();

    Ok(SearchArtifactPublicationSummary {
        active_projection_count: publication.projections.len(),
    })
}

pub fn publish_search_projection_removals(
    data_dir: &Path,
    store: &MetaStore,
    removals: &[SearchProjectionRemoval],
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
) -> Result<SearchArtifactPublicationSummary> {
    let mut documents = Vec::with_capacity(removals.len());
    for removal in removals {
        let Some(mut document) = store
            .document_by_id(&removal.document_id)
            .map_err(ImportPipelineError::store)?
        else {
            continue;
        };
        if matches!(
            removal.reason,
            SearchProjectionRemovalReason::ConfirmedSourceDeletion
                | SearchProjectionRemovalReason::PrivacyRevocation
        ) {
            document.is_deleted = true;
            document.status = DocumentStatus::Deleted;
        }
        document.updated_at = now;
        documents.push(document);
    }
    let document_ids = removals
        .iter()
        .map(|removal| removal.document_id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let publication = write_incremental_search_artifacts(
        data_dir,
        store,
        now,
        &publication_classifier_epoch(store, &[])?,
        Vec::new(),
        &document_ids,
        0,
        removals.len(),
        None,
        CurrentImportCacheMode::Retain,
        None,
        None,
        None,
        ImportResourcePolicy::detect().index_writer_heap_bytes,
        vectorization,
    )?;
    let publication =
        commit_prepared_search_publication(store, now, publication, &documents)?.release();

    Ok(SearchArtifactPublicationSummary {
        active_projection_count: publication.projections.len(),
    })
}

pub fn index_claimed_ocr_text(
    data_dir: &Path,
    store: &MetaStore,
    claimed: &ClaimedOcrJob,
    ocr_text: &str,
    confidence: Option<f32>,
    page_count: Option<u32>,
    now: UnixTimestamp,
    vectorization: &SearchPublicationVectorization,
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
        vectorization,
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
    vectorization: &SearchPublicationVectorization,
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
    let content_hash = claimed
        .source_fingerprint()
        .parse::<ContentDigest>()
        .map_err(|_| ImportPipelineError {
            kind: ImportPipelineErrorKind::Store,
            retryable: false,
        })?;
    let source_revision =
        SourceRevision::for_content(document.id.clone(), content_hash, document.byte_size);
    let version = resume_version(
        &document,
        &source_revision,
        clean_text.clone(),
        OCR_PARSE_VERSION,
        SCHEMA_VERSION,
        language_set(&clean_text),
        page_count,
        Some(confidence.unwrap_or(0.5)),
    );
    document.status = if admitted {
        DocumentStatus::FieldsExtracted
    } else {
        DocumentStatus::OcrDone
    };
    document.updated_at = now;
    let mentions = if admitted {
        entity_mentions_from_rules(&version.id, &clean_text)
    } else {
        Vec::new()
    };
    let pending_index_documents = if admitted {
        vec![IndexDocument {
            doc_id: document.id.to_string(),
            resume_version_id: version.id.to_string(),
            file_name: document.file_name.clone(),
            clean_text: clean_text.clone(),
            sections: sections_to_index(sections),
        }]
    } else {
        Vec::new()
    };
    let (email_hash, phone_hash) = if admitted {
        contact_hashes_from_mentions(data_dir, &mentions)?
    } else {
        (None, None)
    };
    let classification = decision.into_version_classification(version.id.clone(), now);
    let publication = OcrAttemptPublication {
        document: &document,
        classification: &classification,
        source_revision: &source_revision,
        version: &version,
        mentions: &mentions,
        email_hash: email_hash.as_ref(),
        phone_hash: phone_hash.as_ref(),
    };
    match store
        .finish_ocr_attempt_success(claimed, publication, now)
        .map_err(ImportPipelineError::store)?
    {
        OcrAttemptSuccessOutcome::Completed => {
            let search_publication = write_incremental_search_artifacts(
                data_dir,
                store,
                now,
                &classification.classifier_epoch,
                pending_index_documents,
                &pending_doc_ids,
                0,
                0,
                None,
                CurrentImportCacheMode::Retain,
                None,
                None,
                None,
                ImportResourcePolicy::detect().index_writer_heap_bytes,
                vectorization,
            )?;
            if admitted {
                document.status = DocumentStatus::Searchable;
            } else {
                document.status = DocumentStatus::Excluded;
            }
            let search_publication = commit_prepared_search_publication(
                store,
                now,
                search_publication,
                std::slice::from_ref(&document),
            )?
            .release();
            Ok(OcrTextIndexOutcome::Committed(OcrTextIndexSummary {
                searchable: admitted,
                indexed_documents: search_publication.fulltext.document_count(),
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
) -> Result<Vec<Document>> {
    let documents = store
        .visible_documents()
        .map_err(ImportPipelineError::store)?;
    let mut deleted_documents = Vec::new();

    for mut document in documents {
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
        document.is_deleted = true;
        document.status = DocumentStatus::Deleted;
        document.updated_at = now;
        deleted_documents.push(document);
    }

    Ok(deleted_documents)
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
    let mut document = document_from_discovered_file(&file, now, DocumentStatus::Discovered);
    if file.extension == FileExtension::Txt
        && file.byte_size > parser_text::DEFAULT_MAX_BYTES as u64
    {
        document.status = DocumentStatus::FailedPermanent;
        document.updated_at = now;
        measure_result_stage(db_elapsed, || {
            persist_document_failure_without_revision(store, &document)
        })?;
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
            measure_result_stage(db_elapsed, || {
                persist_document_failure_without_revision(store, &document)
            })?;
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

    let source_revision = source_revision(&document, &bytes);
    if let Some(noop_kind) = measure_result_stage(db_elapsed, || {
        exact_rerun_noop_kind(
            store,
            &file,
            &source_revision.content_hash,
            linear_promotion,
        )
    })? {
        let processed = match noop_kind {
            ExactRerunNoopKind::Searchable => ProcessedFile::UnchangedSearchable,
            ExactRerunNoopKind::OcrRequired => ProcessedFile::UnchangedOcrRequired,
            ExactRerunNoopKind::Excluded => ProcessedFile::UnchangedExcluded,
        };
        return Ok(PreparedFile::Ready(ProcessedImportFile { file, processed }));
    }

    document.content_hash = Some(source_revision.content_hash.as_str().to_string());
    document.byte_size = source_revision.byte_size;
    ensure_not_cancelled()?;

    Ok(PreparedFile::Parse(ParseWorkItem {
        index,
        file,
        document,
        source_revision,
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
        source_revision,
        bytes,
    } = work;
    let parse_started = Instant::now();
    let output =
        parse_work_item_inner(&file, &document, &source_revision, &bytes, linear_promotion);
    let parse_finished = Instant::now();

    ParseWorkResult {
        index,
        file,
        document,
        source_revision,
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
    source_revision: &SourceRevision,
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

    let sections = measure_stage(&mut post_parser_timings.sectionization, || {
        Sectionizer::default().sectionize(&clean_text)
    });
    let decision =
        AdmissionDecision::after_sectionization(&clean_text, &sections, linear_promotion);
    let admitted = decision.admits_search_index();
    let version = resume_version(
        document,
        source_revision,
        clean_text.clone(),
        PARSE_VERSION,
        SCHEMA_VERSION,
        language_set(&clean_text),
        parse_output
            .page_count()
            .and_then(|page_count| u32::try_from(page_count).ok()),
        Some(0.8),
    );
    let version_id = version.id.clone();
    let outcome = if admitted {
        ParseWorkOutcome::Searchable {
            decision,
            version: Box::new(version),
            mentions: entity_mentions_from_rules(&version_id, &clean_text),
            index_document: Box::new(IndexDocument {
                doc_id: document.id.to_string(),
                resume_version_id: version_id.to_string(),
                file_name: file.file_name.clone(),
                clean_text,
                sections: sections_to_index(sections),
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
    pending_index_documents: &mut Vec<PendingSearchableDocument>,
    pending_excluded_doc_ids: &mut PendingProjectionRemovals,
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

fn commit_parse_work_result(
    data_dir: &Path,
    store: &MetaStore,
    now: UnixTimestamp,
    db_timing: &mut Duration,
    worker_metrics: &mut ImportWorkerMetrics,
    parse_worker_clock: &mut ParseWorkerClock,
    result: ParseWorkResult,
    linear_promotion: &LinearPromotionPolicy,
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
        source_revision,
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
            measure_result_stage(db_timing, || {
                prepare_pending_searchable_document(
                    data_dir,
                    document,
                    source_revision,
                    decision,
                    *version,
                    mentions,
                    *index_document,
                    now,
                )
            })?
        }
        ParseWorkOutcome::Excluded { decision, version } => {
            document.status = DocumentStatus::Excluded;
            document.updated_at = now;
            measure_result_stage(db_timing, || {
                persist_non_searchable(store, &document, &source_revision, &version, decision, now)
            })?;
            ProcessedFile::Excluded {
                document: Box::new(document),
            }
        }
        ParseWorkOutcome::OcrRequired => ProcessedFile::OcrRequired {
            ocr_job_queued: measure_result_stage(db_timing, || {
                mark_ocr_required_and_enqueue(
                    store,
                    &mut document,
                    &source_revision,
                    now,
                    linear_promotion,
                )
            })?,
        },
        ParseWorkOutcome::Failed { status, kind } => {
            document.status = status;
            document.updated_at = now;
            measure_result_stage(db_timing, || {
                persist_source_revision_failure(
                    store,
                    &document,
                    &source_revision,
                    now,
                    linear_promotion,
                )
            })?;
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
    let mut document = document_from_discovered_file(file, now, DocumentStatus::Discovered);
    if file.extension == FileExtension::Txt
        && file.byte_size > parser_text::DEFAULT_MAX_BYTES as u64
    {
        document.status = DocumentStatus::FailedPermanent;
        document.updated_at = now;
        measure_result_stage(db_elapsed, || {
            persist_document_failure_without_revision(store, &document)
        })?;
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
            measure_result_stage(db_elapsed, || {
                persist_document_failure_without_revision(store, &document)
            })?;
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::ReadError,
            });
        }
    };
    *content_bytes_read += bytes.len() as u64;
    ensure_not_cancelled()?;

    let source_revision = source_revision(&document, &bytes);
    if let Some(noop_kind) = measure_result_stage(db_elapsed, || {
        exact_rerun_noop_kind(store, file, &source_revision.content_hash, linear_promotion)
    })? {
        return Ok(match noop_kind {
            ExactRerunNoopKind::Searchable => ProcessedFile::UnchangedSearchable,
            ExactRerunNoopKind::OcrRequired => ProcessedFile::UnchangedOcrRequired,
            ExactRerunNoopKind::Excluded => ProcessedFile::UnchangedExcluded,
        });
    }

    document.content_hash = Some(source_revision.content_hash.as_str().to_string());
    document.byte_size = source_revision.byte_size;
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
            measure_result_stage(db_elapsed, || {
                persist_source_revision_failure(
                    store,
                    &document,
                    &source_revision,
                    now,
                    linear_promotion,
                )
            })?;
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
                        mark_ocr_required_and_enqueue(
                            store,
                            &mut document,
                            &source_revision,
                            now,
                            linear_promotion,
                        )
                    })?,
                }
            } else {
                measure_result_stage(db_elapsed, || {
                    persist_source_revision_failure(
                        store,
                        &document,
                        &source_revision,
                        now,
                        linear_promotion,
                    )
                })?;
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
                mark_ocr_required_and_enqueue(
                    store,
                    &mut document,
                    &source_revision,
                    now,
                    linear_promotion,
                )
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
            measure_result_stage(db_elapsed, || {
                persist_source_revision_failure(
                    store,
                    &document,
                    &source_revision,
                    now,
                    linear_promotion,
                )
            })?;
            return Ok(ProcessedFile::Failed {
                kind: ImportFailureKind::EmptyText,
            });
        }

        ensure_not_cancelled()?;
        return Ok(ProcessedFile::OcrRequired {
            ocr_job_queued: measure_result_stage(db_elapsed, || {
                mark_ocr_required_and_enqueue(
                    store,
                    &mut document,
                    &source_revision,
                    now,
                    linear_promotion,
                )
            })?,
        });
    }

    ensure_not_cancelled()?;
    document.status = DocumentStatus::TextCleaned;
    let sections = measure_stage(&mut post_parser_timings.sectionization, || {
        sectionizer.sectionize(&clean_text)
    });
    worker_metrics.post_parser_timings.sectionization += post_parser_timings.sectionization;
    let decision =
        AdmissionDecision::after_sectionization(&clean_text, &sections, linear_promotion);
    let admitted = decision.admits_search_index();
    let version = resume_version(
        &document,
        &source_revision,
        clean_text.clone(),
        PARSE_VERSION,
        SCHEMA_VERSION,
        language_set(&clean_text),
        parse_output
            .page_count()
            .and_then(|page_count| u32::try_from(page_count).ok()),
        Some(0.8),
    );
    let version_id = version.id.clone();
    if !admitted {
        document.status = DocumentStatus::Excluded;
        measure_result_stage(db_elapsed, || {
            persist_non_searchable(store, &document, &source_revision, &version, decision, now)
        })?;
        return Ok(ProcessedFile::Excluded {
            document: Box::new(document),
        });
    }
    let mentions = entity_mentions_from_rules(&version_id, &clean_text);
    let index_document = IndexDocument {
        doc_id: document.id.to_string(),
        resume_version_id: version_id.to_string(),
        file_name: file.file_name.clone(),
        clean_text,
        sections: sections_to_index(sections),
    };
    ensure_not_cancelled()?;
    measure_result_stage(db_elapsed, || {
        prepare_pending_searchable_document(
            data_dir,
            document,
            source_revision,
            decision,
            version,
            mentions,
            index_document,
            now,
        )
    })
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

fn prepare_pending_searchable_document(
    data_dir: &Path,
    document: Document,
    source_revision: SourceRevision,
    decision: AdmissionDecision,
    version: ResumeVersion,
    mentions: Vec<EntityMention>,
    index_document: IndexDocument,
    now: UnixTimestamp,
) -> Result<ProcessedFile> {
    let classification = decision.into_version_classification(version.id.clone(), now);
    let (email_hash, phone_hash) = contact_hashes_from_mentions(data_dir, &mentions)?;
    Ok(ProcessedFile::Searchable {
        pending: Box::new(PendingSearchableDocument {
            document,
            source_revision,
            classification,
            version,
            mentions,
            email_hash,
            phone_hash,
            index_document,
        }),
    })
}

fn persist_non_searchable(
    store: &MetaStore,
    document: &Document,
    source_revision: &SourceRevision,
    version: &ResumeVersion,
    decision: AdmissionDecision,
    now: UnixTimestamp,
) -> Result<()> {
    let classification = decision.into_version_classification(version.id.clone(), now);
    immutable_ingest::stage(
        store,
        StagedResume {
            document,
            source_revision,
            derived: StagedDerivedData::ClassifiedVersion {
                version,
                classification: &classification,
                mentions: &[],
                email_hash: None,
                phone_hash: None,
            },
        },
    )
    .map_err(ImportPipelineError::store)
}

fn persist_document_failure_without_revision(store: &MetaStore, document: &Document) -> Result<()> {
    let has_active_projection = store
        .active_search_projection_for_document(&document.id)
        .map_err(ImportPipelineError::store)?
        .is_some();
    if has_active_projection {
        return Ok(());
    }
    store
        .upsert_document(document)
        .map_err(ImportPipelineError::store)
}

fn persist_source_revision_failure(
    store: &MetaStore,
    document: &Document,
    source_revision: &SourceRevision,
    now: UnixTimestamp,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<()> {
    let triage = AdmissionDecision::failed(linear_promotion)
        .into_source_triage(source_revision.id.clone(), now);
    immutable_ingest::stage(
        store,
        StagedResume {
            document,
            source_revision,
            derived: StagedDerivedData::SourceTriage(&triage),
        },
    )
    .map_err(ImportPipelineError::store)
}

fn mark_ocr_required_and_enqueue(
    store: &MetaStore,
    document: &mut Document,
    source_revision: &SourceRevision,
    now: UnixTimestamp,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<bool> {
    document.status = DocumentStatus::OcrRequired;
    document.updated_at = now;
    let triage = AdmissionDecision::ocr_backlog(linear_promotion)
        .into_source_triage(source_revision.id.clone(), now);
    immutable_ingest::stage(
        store,
        StagedResume {
            document,
            source_revision,
            derived: StagedDerivedData::SourceTriage(&triage),
        },
    )
    .map_err(ImportPipelineError::store)?;
    let triage_epoch = CurrentClassifierEpoch::parse(&triage.triage_epoch)
        .ok_or_else(ImportPipelineError::store_invariant)?;
    let enqueue = store
        .enqueue_ocr_job_for_source_triage(&source_revision.id, triage_epoch, now)
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
    strong_content_hash: &ContentDigest,
    linear_promotion: &LinearPromotionPolicy,
) -> Result<Option<ExactRerunNoopKind>> {
    let Some(mut document) = store
        .document_by_id(&file.document_id)
        .map_err(ImportPipelineError::store)?
    else {
        return Ok(None);
    };

    if document.is_deleted
        || document.extension != file.extension
        || document.byte_size != file.byte_size
        || document.mtime != file.mtime
        || document.content_hash.as_deref() != Some(strong_content_hash.as_str())
    {
        return Ok(None);
    }

    if document.normalized_path != file.normalized_path.as_str()
        || document.file_name != file.file_name
    {
        document.source_uri = format!("file://{}", file.normalized_path.as_str());
        document.normalized_path = file.normalized_path.as_str().to_string();
        document.file_name = file.file_name.clone();
        store
            .upsert_document(&document)
            .map_err(ImportPipelineError::store)?;
    }

    let source_revision = SourceRevision::for_content(
        document.id.clone(),
        strong_content_hash.clone(),
        file.byte_size,
    );
    let classifier_epoch = linear_promotion
        .classifier_epoch()
        .unwrap_or(meta_store::CLASSIFIER_EPOCH);

    match document.status {
        DocumentStatus::Searchable | DocumentStatus::IndexedPartial => {
            let Some(active_projection) = store
                .active_search_projection_for_document(&document.id)
                .map_err(ImportPipelineError::store)?
            else {
                return Ok(None);
            };
            let Some(version) = store
                .resume_version_by_id(&active_projection.resume_version_id)
                .map_err(ImportPipelineError::store)?
            else {
                return Err(ImportPipelineError::store_invariant());
            };
            if version.source_revision_id != source_revision.id
                || version.schema_version != SCHEMA_VERSION
                || !matches!(
                    version.parse_version.as_str(),
                    PARSE_VERSION | OCR_PARSE_VERSION
                )
            {
                return Ok(None);
            }
            let Some(classification) = store
                .resume_version_classification(&version.id, classifier_epoch)
                .map_err(ImportPipelineError::store)?
            else {
                return Ok(None);
            };
            Ok(
                (classification_epoch_matches(classifier_epoch, &classification.classifier_epoch)
                    && classification.status == ClassificationStatus::ResumeCandidate)
                    .then_some(ExactRerunNoopKind::Searchable),
            )
        }
        DocumentStatus::OcrRequired => {
            let Some(triage) = store
                .source_revision_triage(&source_revision.id, classifier_epoch)
                .map_err(ImportPipelineError::store)?
            else {
                return Ok(None);
            };
            if !classification_epoch_matches(classifier_epoch, &triage.triage_epoch)
                || triage.status != ClassificationStatus::OcrBacklog
            {
                return Ok(None);
            }
            let triage_epoch = CurrentClassifierEpoch::parse(classifier_epoch)
                .ok_or_else(ImportPipelineError::store_invariant)?;
            let job = store
                .ocr_job_for_source_triage(&source_revision.id, triage_epoch)
                .map_err(ImportPipelineError::store)?;
            Ok(job
                .as_ref()
                .is_some_and(ocr_job_is_actionable)
                .then_some(ExactRerunNoopKind::OcrRequired))
        }
        DocumentStatus::Excluded => {
            let mut matching = store
                .resume_versions_for_document(&document.id)
                .map_err(ImportPipelineError::store)?
                .into_iter()
                .filter(|version| {
                    version.source_revision_id == source_revision.id
                        && matches!(
                            version.parse_version.as_str(),
                            PARSE_VERSION | OCR_PARSE_VERSION
                        )
                        && version.schema_version == SCHEMA_VERSION
                });
            let Some(version) = matching.next() else {
                return Ok(None);
            };
            if matching.next().is_some() {
                return Err(ImportPipelineError::store_invariant());
            }
            let Some(classification) = store
                .resume_version_classification(&version.id, classifier_epoch)
                .map_err(ImportPipelineError::store)?
            else {
                return Ok(None);
            };
            if !classification_epoch_matches(classifier_epoch, &classification.classifier_epoch)
                || !matches!(
                    classification.status,
                    ClassificationStatus::NonResume | ClassificationStatus::NeedsReview
                )
            {
                return Ok(None);
            }
            Ok(Some(ExactRerunNoopKind::Excluded))
        }
        _ => Ok(None),
    }
}

fn classification_epoch_matches(expected: &str, actual: &str) -> bool {
    CurrentClassifierEpoch::parse(actual).is_some_and(|epoch| epoch.as_str() == expected)
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
        content_hash: None,
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
        pending: Box<PendingSearchableDocument>,
    },
    UnchangedSearchable,
    UnchangedOcrRequired,
    UnchangedExcluded,
    Excluded {
        document: Box<Document>,
    },
    OcrRequired {
        ocr_job_queued: bool,
    },
    Failed {
        kind: ImportFailureKind,
    },
}

struct PendingSearchableDocument {
    document: Document,
    source_revision: SourceRevision,
    classification: ResumeVersionClassification,
    version: ResumeVersion,
    mentions: Vec<EntityMention>,
    email_hash: Option<ContactHash>,
    phone_hash: Option<ContactHash>,
    index_document: IndexDocument,
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
    source_revision: SourceRevision,
    bytes: Vec<u8>,
}

struct ParseWorkResult {
    index: usize,
    file: DiscoveredFile,
    document: Document,
    source_revision: SourceRevision,
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
    IndexPublicationAtomicPublication,
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
            Self::IndexPublicationAtomicPublication => "index_publication_atomic_publication",
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
            SnapshotPublishPhase::AtomicPublication => Self::IndexPublicationAtomicPublication,
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
    pub atomic_publication: Duration,
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
            SnapshotPublishPhase::AtomicPublication => self.atomic_publication += elapsed,
        }
    }

    fn add_assign(&mut self, next: &Self) {
        self.setup += next.setup;
        self.documents += next.documents;
        self.commit += next.commit;
        self.plaintext_validation += next.plaintext_validation;
        self.encrypted_publication += next.encrypted_publication;
        self.encrypted_validation += next.encrypted_validation;
        self.atomic_publication += next.atomic_publication;
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
pub struct SearchArtifactPublicationSummary {
    pub active_projection_count: usize,
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

    fn store_invariant() -> Self {
        Self {
            kind: ImportPipelineErrorKind::Store,
            retryable: false,
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

    fn index_io() -> Self {
        Self {
            kind: ImportPipelineErrorKind::Index,
            retryable: true,
        }
    }

    fn vector(error: index_vector::VectorIndexError) -> Self {
        let kind = match error {
            index_vector::VectorIndexError::InvalidDimension { .. }
            | index_vector::VectorIndexError::InvalidVectorValue
            | index_vector::VectorIndexError::InvalidModelId
            | index_vector::VectorIndexError::InvalidIdentity
            | index_vector::VectorIndexError::InvalidGeneration
            | index_vector::VectorIndexError::InvalidModelContract
            | index_vector::VectorIndexError::SemanticUnavailable
            | index_vector::VectorIndexError::PublicationProjectionMismatch
            | index_vector::VectorIndexError::DuplicateVectorId
            | index_vector::VectorIndexError::ConflictingDocumentVersion => {
                ImportPipelineErrorKind::VectorContract
            }
            index_vector::VectorIndexError::GenerationAlreadyExists
            | index_vector::VectorIndexError::GenerationNotFound
            | index_vector::VectorIndexError::LeaseRootMismatch
            | index_vector::VectorIndexError::SchemaMismatch
            | index_vector::VectorIndexError::CorruptSnapshot
            | index_vector::VectorIndexError::StorageLayoutInvalid
            | index_vector::VectorIndexError::Storage => ImportPipelineErrorKind::VectorStorage,
        };
        Self {
            kind,
            retryable: true,
        }
    }

    fn vector_io() -> Self {
        Self {
            kind: ImportPipelineErrorKind::EmbeddingRuntime,
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

    pub fn class(&self) -> ImportPipelineErrorClass {
        match self.kind {
            ImportPipelineErrorKind::Cancelled => ImportPipelineErrorClass::Cancelled,
            ImportPipelineErrorKind::Store => ImportPipelineErrorClass::Metadata,
            ImportPipelineErrorKind::Crawl => ImportPipelineErrorClass::Scan,
            ImportPipelineErrorKind::Index => ImportPipelineErrorClass::FullText,
            ImportPipelineErrorKind::VectorContract => ImportPipelineErrorClass::VectorContract,
            ImportPipelineErrorKind::VectorStorage => ImportPipelineErrorClass::VectorStorage,
            ImportPipelineErrorKind::EmbeddingRuntime => ImportPipelineErrorClass::EmbeddingRuntime,
            ImportPipelineErrorKind::Privacy => ImportPipelineErrorClass::Privacy,
            ImportPipelineErrorKind::Parser => ImportPipelineErrorClass::Parser,
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
            ImportPipelineErrorKind::VectorContract => {
                formatter.write_str("vector publication contract failed")
            }
            ImportPipelineErrorKind::VectorStorage => {
                formatter.write_str("vector index storage failed")
            }
            ImportPipelineErrorKind::EmbeddingRuntime => {
                formatter.write_str("document embedding failed")
            }
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
    VectorContract,
    VectorStorage,
    EmbeddingRuntime,
    Privacy,
    Parser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportPipelineErrorClass {
    Cancelled,
    Metadata,
    Scan,
    FullText,
    VectorContract,
    VectorStorage,
    EmbeddingRuntime,
    Privacy,
    Parser,
}

impl ImportPipelineErrorClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Metadata => "metadata",
            Self::Scan => "scan",
            Self::FullText => "fulltext",
            Self::VectorContract => "vector_contract",
            Self::VectorStorage => "vector_storage",
            Self::EmbeddingRuntime => "embedding_runtime",
            Self::Privacy => "privacy",
            Self::Parser => "parser",
        }
    }
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
    use index_fulltext::{
        incremental_snapshot_documents, FullTextIndex, SearchQuery, SnapshotReadLease,
    };
    use index_vector::{QueryVector, VectorModelContract, VectorSnapshotRoot};
    use meta_store::{
        ActiveSearchProjection, ClassificationStatus, ContentDigest, CurrentClassifierEpoch,
        Document, DocumentId, DocumentStatus, EntityMention, EntityMentionId, EntityType,
        FileExtension, ImportRootKind, ImportScanProfile, ImportScanScope, ImportTask,
        ImportTaskStatus, IngestJobStatus, MetaStore, OcrAttemptFailure, ReasonCode, ResumeVersion,
        ResumeVersionClassification, ResumeVersionId, ReviewDisposition, SearchMetadataHead,
        SearchPublicationState, SearchSelection, SearchSelectionResolution, SourceRevision,
        UnixTimestamp, CLASSIFIER_EPOCH,
    };
    use resume_classifier::LinearPromotionPolicy;

    use super::search_artifact_cache::CachedSearchDocument;
    use super::{
        classify_language_set, commit_prepared_search_publication, current_timestamp_or,
        document_path_is_deletion_candidate, flush_pending_searchable_documents,
        import_root_with_options, index_claimed_ocr_text, reconcile_search_artifacts,
        recv_parse_result_with_cancel_poll, should_flush_searchable_documents,
        take_pending_searchable_documents, write_incremental_search_artifacts, AdmissionDecision,
        CurrentImportCacheMode, CurrentImportDocumentCache, ImportCancelCheckPhase,
        ImportHardwareProfile, ImportHardwareTier, ImportOptions, ImportParseWorkers,
        ImportPipelineErrorKind, ImportResourcePolicy, ImportSummary, IndexDocument, IndexSection,
        OcrTextIndexOutcome, ParseWorkOutcome, ParseWorkResult, PendingProjectionRemovals,
        PendingSearchableDocument, SearchProjectionRemoval, SearchProjectionRemovalReason,
        SearchPublicationEmbeddingFailure, SearchPublicationEmbeddingInput,
        SearchPublicationEmbeddingOutput, SearchPublicationVectorization,
        SearchPublicationVectorizer, SnapshotPublishPhase, BYTES_PER_GIB,
        H2_INDEX_WRITER_HEAP_BYTES,
    };

    struct TestPublicationVectorizer {
        fail: bool,
    }

    impl SearchPublicationVectorizer for TestPublicationVectorizer {
        fn model_id(&self) -> &str {
            "synthetic-publication-v1"
        }

        fn dimension(&self) -> usize {
            2
        }

        fn max_batch_inputs(&self) -> usize {
            4
        }

        fn max_text_bytes(&self) -> usize {
            65_536
        }

        fn embed_batch(
            &self,
            inputs: &[SearchPublicationEmbeddingInput],
            _is_cancelled: &dyn Fn() -> bool,
        ) -> std::result::Result<
            Vec<SearchPublicationEmbeddingOutput>,
            SearchPublicationEmbeddingFailure,
        > {
            if self.fail {
                return Err(SearchPublicationEmbeddingFailure::RuntimeUnavailable);
            }
            Ok(inputs
                .iter()
                .map(|input| {
                    SearchPublicationEmbeddingOutput::new(
                        input.id(),
                        self.model_id(),
                        vec![1.0, input.text().len() as f32],
                    )
                })
                .collect())
        }
    }

    #[cfg(unix)]
    static DOC_CONVERTER_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn claim_ocr_document(
        store: &MetaStore,
        document: &Document,
        now: UnixTimestamp,
    ) -> meta_store::ClaimedOcrJob {
        let mut document = document.clone();
        document.status = DocumentStatus::OcrRequired;
        let content_hash = document
            .content_hash
            .as_deref()
            .and_then(|value| value.parse::<ContentDigest>().ok())
            .unwrap_or_else(|| ContentDigest::from_bytes(document.file_name.as_bytes()));
        document.content_hash = Some(content_hash.as_str().to_string());
        let source_revision =
            SourceRevision::for_content(document.id.clone(), content_hash, document.byte_size);
        store.upsert_document(&document).unwrap();
        store.insert_source_revision(&source_revision).unwrap();
        let triage = AdmissionDecision::ocr_backlog(&LinearPromotionPolicy::default())
            .into_source_triage(source_revision.id.clone(), now);
        store.insert_source_revision_triage(&triage).unwrap();
        let triage_epoch = CurrentClassifierEpoch::parse(&triage.triage_epoch).unwrap();
        store
            .enqueue_ocr_job_for_source_triage(&source_revision.id, triage_epoch, now)
            .unwrap();
        store.claim_next_ocr_job(now).unwrap().unwrap()
    }

    fn active_resume_version(store: &MetaStore, document: &Document) -> Option<ResumeVersion> {
        let projection = store
            .active_search_projection_for_document(&document.id)
            .unwrap()?;
        store
            .resume_version_by_id(&projection.resume_version_id)
            .unwrap()
    }

    fn ready_search_head(store: &MetaStore) -> SearchMetadataHead {
        store
            .with_search_metadata_snapshot(|snapshot| Ok::<_, ()>(snapshot.head().clone()))
            .unwrap()
    }

    fn open_fulltext_generation(data_dir: &Path, generation: &str) -> FullTextIndex {
        let index_root = data_dir.join("search-index");
        let lease = SnapshotReadLease::acquire(&index_root)
            .unwrap()
            .expect("ready publication must expose a full-text root lease");
        FullTextIndex::open_snapshot_with_lease(&index_root, generation, lease)
            .unwrap()
            .expect("ready publication must expose its exact full-text generation")
    }

    fn resolve_selection(
        store: &MetaStore,
        selection: &SearchSelection,
    ) -> SearchSelectionResolution {
        store
            .with_search_metadata_snapshot(|snapshot| snapshot.resolve_search_selection(selection))
            .unwrap()
    }

    fn test_source_revision(document: &Document) -> SourceRevision {
        SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(document.file_name.as_bytes()),
            document.byte_size,
        )
    }

    #[test]
    fn content_addressed_version_changes_when_source_revision_changes() {
        let mut document = test_document("content-identity", DocumentStatus::TextCleaned);
        let revision_a = SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(b"source-a"),
            8,
        );
        let revision_b = SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(b"source-b"),
            8,
        );
        let clean_text = "Synthetic Candidate\nEXPERIENCE\nRust Search";
        document.content_hash = Some(revision_a.content_hash.as_str().to_string());
        let version_a = super::resume_version(
            &document,
            &revision_a,
            clean_text.to_string(),
            "parser-v1",
            "schema-v27",
            vec!["en".to_string()],
            Some(1),
            Some(0.8),
        );
        document.content_hash = Some(revision_b.content_hash.as_str().to_string());
        let version_b = super::resume_version(
            &document,
            &revision_b,
            clean_text.to_string(),
            "parser-v1",
            "schema-v27",
            vec!["en".to_string()],
            Some(1),
            Some(0.8),
        );

        assert_ne!(revision_a.id, revision_b.id);
        assert_ne!(version_a.id, version_b.id);
        assert_eq!(
            version_a.normalized_text_hash,
            ContentDigest::from_bytes(clean_text.as_bytes())
        );
        assert_eq!(
            version_a.normalized_text_hash,
            version_b.normalized_text_hash
        );
    }

    #[test]
    fn staged_versions_are_invisible_and_serial_publications_advance_atomically() {
        let temp = TestDir::new("staged-publication-cas");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let first = test_pending_searchable_document("publication-first");
        let second = test_pending_searchable_document("publication-second");
        for pending in [&first, &second] {
            super::immutable_ingest::stage(
                &store,
                super::StagedResume {
                    document: &pending.document,
                    source_revision: &pending.source_revision,
                    derived: super::StagedDerivedData::ClassifiedVersion {
                        version: &pending.version,
                        classification: &pending.classification,
                        mentions: &pending.mentions,
                        email_hash: None,
                        phone_hash: None,
                    },
                },
            )
            .unwrap();
            assert_eq!(
                store
                    .active_search_projection_for_document(&pending.document.id)
                    .unwrap(),
                None
            );
        }

        let now = UnixTimestamp::from_unix_seconds(1_700_200_000);
        let first_publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            now,
            CLASSIFIER_EPOCH,
            vec![first.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let mut first_document = first.document.clone();
        first_document.status = DocumentStatus::Searchable;
        let first_publication = commit_prepared_search_publication(
            &store,
            now,
            first_publication,
            std::slice::from_ref(&first_document),
        )
        .unwrap()
        .release();
        let second_now = UnixTimestamp::from_unix_seconds(now.as_unix_seconds() + 1);
        let second_publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            second_now,
            CLASSIFIER_EPOCH,
            vec![second.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let mut second_document = second.document.clone();
        second_document.status = DocumentStatus::Searchable;
        let second_publication = commit_prepared_search_publication(
            &store,
            second_now,
            second_publication,
            std::slice::from_ref(&second_document),
        )
        .unwrap()
        .release();
        assert_eq!(first_publication.projections.len(), 1);
        assert_eq!(second_publication.projections.len(), 2);
        assert_eq!(
            store
                .active_search_projection_for_document(&first.document.id)
                .unwrap(),
            Some(ActiveSearchProjection {
                document_id: first.document.id.clone(),
                resume_version_id: first.version.id.clone(),
            })
        );
        assert_eq!(
            store
                .active_search_projection_for_document(&second.document.id)
                .unwrap(),
            Some(ActiveSearchProjection {
                document_id: second.document.id.clone(),
                resume_version_id: second.version.id.clone(),
            })
        );
    }

    #[test]
    fn vector_publication_is_exact_atomic_and_retained_across_removal() {
        let temp = TestDir::new("vector-publication-atomic");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let first = test_pending_searchable_document("vector-first");
        let second = test_pending_searchable_document("vector-second");
        for pending in [&first, &second] {
            super::immutable_ingest::stage(
                &store,
                super::StagedResume {
                    document: &pending.document,
                    source_revision: &pending.source_revision,
                    derived: super::StagedDerivedData::ClassifiedVersion {
                        version: &pending.version,
                        classification: &pending.classification,
                        mentions: &pending.mentions,
                        email_hash: None,
                        phone_hash: None,
                    },
                },
            )
            .unwrap();
        }

        let enabled =
            SearchPublicationVectorization::enabled(Arc::new(TestPublicationVectorizer {
                fail: false,
            }));
        let first_now = UnixTimestamp::from_unix_seconds(1_700_210_000);
        let first_publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            first_now,
            CLASSIFIER_EPOCH,
            vec![first.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &enabled,
        )
        .unwrap();
        let mut first_document = first.document.clone();
        first_document.status = DocumentStatus::Searchable;
        commit_prepared_search_publication(
            &store,
            first_now,
            first_publication,
            std::slice::from_ref(&first_document),
        )
        .unwrap()
        .release();
        let first_head = ready_search_head(&store);

        let failing =
            SearchPublicationVectorization::enabled(Arc::new(TestPublicationVectorizer {
                fail: true,
            }));
        let failed = write_incremental_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_210_001),
            CLASSIFIER_EPOCH,
            vec![second.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &failing,
        );
        assert!(failed.is_err());
        assert_eq!(ready_search_head(&store).generation, first_head.generation);
        assert_eq!(
            store
                .active_search_projection_for_document(&second.document.id)
                .unwrap(),
            None
        );

        let second_now = UnixTimestamp::from_unix_seconds(1_700_210_002);
        let second_publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            second_now,
            CLASSIFIER_EPOCH,
            vec![second.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &enabled,
        )
        .unwrap();
        let mut second_document = second.document.clone();
        second_document.status = DocumentStatus::Searchable;
        commit_prepared_search_publication(
            &store,
            second_now,
            second_publication,
            std::slice::from_ref(&second_document),
        )
        .unwrap()
        .release();
        assert_vector_generation(&data_dir, &store, 2);

        super::publish_search_projection_removals(
            &data_dir,
            &store,
            &[SearchProjectionRemoval {
                document_id: second.document.id.clone(),
                reason: SearchProjectionRemovalReason::ConfirmedSourceDeletion,
            }],
            UnixTimestamp::from_unix_seconds(1_700_210_003),
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        assert_vector_generation(&data_dir, &store, 1);
    }

    #[test]
    fn reconcile_promotes_a_usable_disabled_snapshot_to_the_configured_vector_contract() {
        let temp = TestDir::new("vector-publication-reconcile-contract");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let pending = test_pending_searchable_document("vector-reconcile");
        super::immutable_ingest::stage(
            &store,
            super::StagedResume {
                document: &pending.document,
                source_revision: &pending.source_revision,
                derived: super::StagedDerivedData::ClassifiedVersion {
                    version: &pending.version,
                    classification: &pending.classification,
                    mentions: &pending.mentions,
                    email_hash: None,
                    phone_hash: None,
                },
            },
        )
        .unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_220_000);
        let publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            now,
            CLASSIFIER_EPOCH,
            vec![pending.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let mut document = pending.document.clone();
        document.status = DocumentStatus::Searchable;
        commit_prepared_search_publication(
            &store,
            now,
            publication,
            std::slice::from_ref(&document),
        )
        .unwrap()
        .release();

        let enabled =
            SearchPublicationVectorization::enabled(Arc::new(TestPublicationVectorizer {
                fail: false,
            }));
        let summary = reconcile_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_220_001),
            &enabled,
        )
        .unwrap();

        assert!(summary.active_generation_rebuilt);
        assert_vector_generation(&data_dir, &store, 1);
    }

    fn assert_vector_generation(data_dir: &Path, store: &MetaStore, expected_documents: usize) {
        let head = ready_search_head(store);
        let vector = head.publication.vector.as_ref().unwrap();
        let contract = VectorModelContract::enabled("synthetic-publication-v1", 2).unwrap();
        let root = VectorSnapshotRoot::new(data_dir.join("vector-index")).unwrap();
        let reader = root
            .open_generation_with_lease(
                vector.generation(),
                &contract,
                root.acquire_read_lease().unwrap(),
            )
            .unwrap();
        assert_eq!(reader.summary().vector_document_count(), expected_documents);
        assert_eq!(reader.exact_projection().len(), expected_documents);
        assert_eq!(
            reader
                .knn(QueryVector::new(vec![1.0, 1.0]).unwrap(), 10)
                .unwrap()
                .len(),
            expected_documents
        );
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
    fn current_import_index_cache_refreshes_after_intervening_snapshot_publication() {
        let temp = TestDir::new("import-pipeline-current-import-index-cache-refresh");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let empty_exclusions = BTreeSet::new();
        let mut current_import_index_documents = CurrentImportDocumentCache::default();

        let first_publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-1")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let first_publication = commit_prepared_search_publication(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            first_publication,
            &[terminal_searchable_document(&store, "doc-1")],
        )
        .unwrap()
        .release();
        assert_eq!(first_publication.fulltext.document_count(), 1);
        assert_eq!(current_import_index_documents.documents.len(), 1);
        assert_eq!(
            retained_section_text_bytes(&current_import_index_documents),
            0
        );

        let intervening_index_document = stage_test_index_document(&store, "doc-2");
        let intervening_doc_id = intervening_index_document.doc_id.clone();
        let intervening_publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_051),
            CLASSIFIER_EPOCH,
            vec![intervening_index_document],
            &empty_exclusions,
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let intervening_publication = commit_prepared_search_publication(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_051),
            intervening_publication,
            &[terminal_searchable_document(&store, "doc-2")],
        )
        .unwrap()
        .release();
        assert_eq!(intervening_publication.fulltext.document_count(), 2);

        let second_publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_052),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-3")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();

        assert_eq!(second_publication.fulltext.document_count(), 3);
        let cached_doc_ids = current_import_index_documents
            .documents
            .iter()
            .map(|document| document.doc_id.clone())
            .collect::<Vec<_>>();
        let mut expected_doc_ids = vec![
            DocumentId::from_non_secret_parts(&["doc-1"]).to_string(),
            intervening_doc_id.clone(),
            DocumentId::from_non_secret_parts(&["doc-3"]).to_string(),
        ];
        expected_doc_ids.sort();
        assert_eq!(cached_doc_ids, expected_doc_ids);
        let active_doc_ids = incremental_snapshot_documents(
            &data_dir.join("search-index"),
            Some(second_publication.fulltext.generation()),
            Vec::new(),
            &BTreeSet::new(),
        )
        .unwrap()
        .into_iter()
        .map(|document| document.doc_id)
        .collect::<Vec<_>>();
        assert_eq!(active_doc_ids, expected_doc_ids);
        assert_eq!(
            retained_section_text_bytes(&current_import_index_documents),
            0
        );
    }
    #[test]
    fn current_import_cache_ignores_uncommitted_generations_and_recovery_abandons_them() {
        let temp = TestDir::new("import-pipeline-uncommitted-generation");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let empty_exclusions = BTreeSet::new();
        let mut current_import_documents = CurrentImportDocumentCache::default();
        let ready_now = UnixTimestamp::from_unix_seconds(1_700_000_060);

        let ready = write_incremental_search_artifacts(
            &data_dir,
            &store,
            ready_now,
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-1")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let ready = commit_prepared_search_publication(
            &store,
            ready_now,
            ready,
            &[terminal_searchable_document(&store, "doc-1")],
        )
        .unwrap()
        .release();

        let uncommitted = write_incremental_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_061),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-2")],
            &empty_exclusions,
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let uncommitted_generation = uncommitted.fulltext.generation().to_string();
        drop(uncommitted);
        let next = write_incremental_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_062),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-3")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let next_generation = next.fulltext.generation().to_string();

        let indexed_doc_ids = incremental_snapshot_documents(
            &data_dir.join("search-index"),
            Some(&next_generation),
            Vec::new(),
            &BTreeSet::new(),
        )
        .unwrap()
        .into_iter()
        .map(|document| document.doc_id)
        .collect::<Vec<_>>();
        assert_eq!(
            indexed_doc_ids,
            vec![
                DocumentId::from_non_secret_parts(&["doc-1"]).to_string(),
                DocumentId::from_non_secret_parts(&["doc-3"]).to_string(),
            ]
        );
        drop(next);

        let recovery = reconcile_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_063),
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        assert_eq!(recovery.interrupted_publications_abandoned, 2);
        assert!(!recovery.active_generation_rebuilt);
        assert_eq!(
            ready_search_head(&store).generation,
            ready.fulltext.generation()
        );
        for generation in [&uncommitted_generation, &next_generation] {
            assert_eq!(
                store.search_publication(generation).unwrap().unwrap().state,
                SearchPublicationState::Abandoned
            );
        }
        let ready_reader = open_fulltext_generation(&data_dir, ready.fulltext.generation());
        assert_eq!(
            ready_reader
                .search(SearchQuery::new("synthetic").with_limit(5))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn recovery_rebuilds_an_exact_fulltext_vector_pair_from_metadata() {
        let temp = TestDir::new("import-pipeline-search-artifact-recovery");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("candidate.txt"),
            synthetic_resume_text("Synthetic Recovery Candidate", "Rust recovery"),
        )
        .unwrap();
        let store = MetaStore::open_data_dir(&data_dir).unwrap();
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_070);
        let task = import_task(
            "search-artifact-recovery",
            root.to_str().unwrap(),
            first_now,
        );
        store.insert_import_task(&task).unwrap();
        import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();

        let first_head = ready_search_head(&store);
        let first_projection = store
            .active_search_projection_for_document(&store.visible_documents().unwrap().remove(0).id)
            .unwrap()
            .unwrap();
        let first_selection = SearchSelection {
            document_id: first_projection.document_id.clone(),
            resume_version_id: first_projection.resume_version_id.clone(),
            visible_epoch: first_head.visible_epoch,
        };
        fs::remove_file(
            data_dir
                .join("search-index")
                .join("snapshots")
                .join(&first_head.generation)
                .join("snapshot-manifest.json"),
        )
        .unwrap();

        let recovery = reconcile_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_071),
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        assert!(recovery.active_generation_rebuilt);
        let recovered_head = ready_search_head(&store);
        assert_ne!(recovered_head.generation, first_head.generation);
        let fulltext = recovered_head.publication.fulltext.as_ref().unwrap();
        let vector = recovered_head.publication.vector.as_ref().unwrap();
        assert_eq!(fulltext.generation(), recovered_head.generation);
        assert_eq!(vector.generation(), recovered_head.generation);
        assert_eq!(fulltext.projection_digest(), vector.projection_digest());
        assert_eq!(fulltext.document_count(), vector.projection_count());

        let recovered = open_fulltext_generation(&data_dir, &recovered_head.generation);
        assert_eq!(
            recovered
                .search(SearchQuery::new("Rust recovery").with_limit(5))
                .unwrap()
                .len(),
            1
        );
        let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index")).unwrap();
        let vector_reader = vector_root
            .open_generation_with_lease(
                &recovered_head.generation,
                &VectorModelContract::Disabled,
                vector_root.acquire_read_lease().unwrap(),
            )
            .unwrap();
        assert_eq!(
            vector_reader.summary().projection_digest(),
            fulltext.projection_digest()
        );
        assert_eq!(
            resolve_selection(&store, &first_selection),
            SearchSelectionResolution::Current {
                selection: first_selection
            }
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
        let mut current_import_index_documents = CurrentImportDocumentCache::default();

        let ready_publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-1")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        commit_prepared_search_publication(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            ready_publication,
            &[terminal_searchable_document(&store, "doc-1")],
        )
        .unwrap()
        .release();
        assert_eq!(current_import_index_documents.documents.len(), 1);

        let final_publication = write_incremental_search_artifacts(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_051),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-2")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportCacheMode::Consume,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();

        assert_eq!(final_publication.fulltext.document_count(), 2);
        assert!(current_import_index_documents.documents.is_empty());
    }

    #[test]
    fn current_import_index_cache_redacts_contact_text_before_retaining() {
        let cached = CachedSearchDocument::from_index_document(IndexDocument {
            doc_id: "doc-contact".to_string(),
            resume_version_id: "ver-contact".to_string(),
            file_name: "person@example.test resume.pdf".to_string(),
            clean_text:
                "Email person@example.test phone +1 650-555-1234 file /Users/private/resume.pdf"
                    .to_string(),
            sections: Vec::new(),
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
            test_pending_searchable_document("doc-2"),
            test_pending_searchable_document("doc-1"),
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
                .map(|document| document.doc_id.clone())
                .collect::<Vec<_>>(),
            vec![
                DocumentId::from_non_secret_parts(&["doc-2"]).to_string(),
                DocumentId::from_non_secret_parts(&["doc-1"]).to_string(),
            ]
        );
    }

    #[test]
    fn failed_staging_batch_does_not_publish_projection_or_index() {
        let temp = TestDir::new("searchable-metadata-batch-rollback");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = MetaStore::open_data_dir(&data_dir).unwrap();
        store.run_migrations().unwrap();

        let mut first = test_pending_searchable_document("batch-first");
        let mut second = test_pending_searchable_document("batch-second");
        let duplicate_id = EntityMentionId::from_non_secret_parts(&["test", "duplicate"]);
        first.mentions = vec![test_entity_mention(
            duplicate_id.clone(),
            first.version.id.clone(),
        )];
        second.mentions = vec![test_entity_mention(duplicate_id, second.version.id.clone())];
        let documents = [
            (first.document.id.clone(), first.version.id.clone()),
            (second.document.id.clone(), second.version.id.clone()),
        ];
        let mut pending = vec![first, second];
        let mut excluded = PendingProjectionRemovals::default();
        let mut summary = ImportSummary::default();

        let error = flush_pending_searchable_documents(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_000),
            &mut summary,
            &mut pending,
            &mut excluded,
            None,
            CurrentImportCacheMode::Retain,
            &|| Ok(()),
            &|_| {},
            Instant::now(),
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap_err();

        assert_eq!(error.kind, ImportPipelineErrorKind::Store);
        assert_eq!(pending.len(), 2);
        for (document_id, _) in documents {
            assert_eq!(
                store
                    .active_search_projection_for_document(&document_id)
                    .unwrap(),
                None
            );
        }
        assert!(!data_dir.join("search-index").exists());
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
    fn import_resource_policy_uses_inclusive_12_and_20_gib_boundaries() {
        for (total_memory_bytes, expected_tier) in [
            (None, ImportHardwareTier::H0Eco),
            (Some(12 * BYTES_PER_GIB), ImportHardwareTier::H0Eco),
            (Some(12 * BYTES_PER_GIB + 1), ImportHardwareTier::H1Balanced),
            (Some(20 * BYTES_PER_GIB), ImportHardwareTier::H1Balanced),
            (
                Some(20 * BYTES_PER_GIB + 1),
                ImportHardwareTier::H2Aggressive,
            ),
        ] {
            let policy = ImportResourcePolicy::for_hardware(ImportHardwareProfile::new(
                total_memory_bytes,
                10,
            ));
            assert_eq!(policy.hardware_tier, expected_tier);
        }
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
        let version = active_resume_version(&store, &document).unwrap();
        assert!(version
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Rust Search"));
        assert_eq!(version.raw_text, None);
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
            let counts = store.classification_counts(CLASSIFIER_EPOCH).unwrap();
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
    fn failed_reparse_stages_failure_without_withdrawing_active_projection() {
        let temp = TestDir::new("failed-reparse-retains-active-projection");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        let path = root.join("candidate.txt");
        fs::write(
            &path,
            synthetic_resume_text("Stable Candidate", "Rust Search"),
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_210_000);
        let first_task = import_task("failure-retention-first", root.to_str().unwrap(), first_now);
        store.insert_import_task(&first_task).unwrap();
        import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let document = store.visible_documents().unwrap().remove(0);
        let active_before = store
            .active_search_projection_for_document(&document.id)
            .unwrap()
            .unwrap();
        let generation_before = store.search_projection_state().unwrap().generation;

        fs::write(&path, []).unwrap();
        let second_now = UnixTimestamp::from_unix_seconds(1_700_210_001);
        let second_task = import_task(
            "failure-retention-second",
            root.to_str().unwrap(),
            second_now,
        );
        store.insert_import_task(&second_task).unwrap();
        let summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.failed_documents, 1);
        assert_eq!(
            store
                .active_search_projection_for_document(&document.id)
                .unwrap(),
            Some(active_before.clone())
        );
        assert_eq!(
            store.search_projection_state().unwrap().generation,
            generation_before
        );
        assert!(store
            .resume_version_by_id(&active_before.resume_version_id)
            .unwrap()
            .unwrap()
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Stable Candidate"));
        let failed_revision =
            SourceRevision::for_content(document.id, ContentDigest::from_bytes(&[]), 0);
        assert_eq!(
            store
                .source_revision_triage(&failed_revision.id, CLASSIFIER_EPOCH)
                .unwrap()
                .unwrap()
                .status,
            ClassificationStatus::Failed
        );
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
            if let Some(version) = active_resume_version(&store, &document) {
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
        let source_revision = test_source_revision(&document);
        let started = Instant::now();
        let mut clock = super::ParseWorkerClock::default();

        clock.record_result(&ParseWorkResult {
            index: 0,
            file: file.clone(),
            document: document.clone(),
            source_revision: source_revision.clone(),
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
            source_revision,
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
        let source_revision = test_source_revision(&document);
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
                    source_revision,
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
                &SearchPublicationVectorization::default(),
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
            &SearchPublicationVectorization::default(),
        )
        .unwrap() else {
            panic!("current OCR attempt was superseded");
        };

        assert!(summary.searchable);
        let version = active_resume_version(&store, &document).unwrap();
        assert!(version
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Rust Search"));
        assert_eq!(version.raw_text, None);
    }

    fn test_pending_searchable_document(doc_id: &str) -> PendingSearchableDocument {
        let mut document = test_document(doc_id, DocumentStatus::TextCleaned);
        let source_bytes = format!("source bytes for {doc_id}");
        let source_revision = SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(source_bytes.as_bytes()),
            source_bytes.len() as u64,
        );
        document.content_hash = Some(source_revision.content_hash.as_str().to_string());
        document.byte_size = source_revision.byte_size;
        let clean_text = format!("Synthetic Candidate {doc_id}\\nSkills: Rust Search");
        let version = super::resume_version(
            &document,
            &source_revision,
            clean_text,
            "parser-v1",
            "schema-v1",
            vec!["en".to_string()],
            Some(1),
            Some(0.8),
        );
        let classification = ResumeVersionClassification {
            resume_version_id: version.id.clone(),
            status: ClassificationStatus::ResumeCandidate,
            classifier_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
            classified_at: document.updated_at,
            review_disposition: ReviewDisposition::NotRequired,
        };
        let index_document = IndexDocument {
            doc_id: document.id.to_string(),
            resume_version_id: version.id.to_string(),
            file_name: format!("{doc_id}.txt"),
            clean_text: version.clean_text.clone().unwrap(),
            sections: Vec::new(),
        };
        PendingSearchableDocument {
            document,
            source_revision,
            classification,
            version,
            mentions: Vec::new(),
            email_hash: None,
            phone_hash: None,
            index_document,
        }
    }

    fn test_entity_mention(
        id: EntityMentionId,
        resume_version_id: ResumeVersionId,
    ) -> EntityMention {
        EntityMention {
            id,
            resume_version_id,
            section_id: None,
            entity_type: EntityType::Skill,
            raw_value: "Rust".to_string(),
            normalized_value: Some("Rust".to_string()),
            span_start: Some(0),
            span_end: Some(4),
            confidence: 0.9,
            extractor: "rules-v1".to_string(),
        }
    }

    fn stage_test_index_document(store: &MetaStore, doc_id: &str) -> IndexDocument {
        let pending = test_pending_searchable_document(doc_id);
        super::immutable_ingest::stage(
            store,
            super::StagedResume {
                document: &pending.document,
                source_revision: &pending.source_revision,
                derived: super::StagedDerivedData::ClassifiedVersion {
                    version: &pending.version,
                    classification: &pending.classification,
                    mentions: &pending.mentions,
                    email_hash: None,
                    phone_hash: None,
                },
            },
        )
        .unwrap();
        let mut index_document = pending.index_document;
        index_document.sections = vec![IndexSection {
            section_type: "skills".to_string(),
            text: format!("Rust Search section for {doc_id}"),
        }];
        index_document
    }

    fn terminal_searchable_document(store: &MetaStore, doc_id: &str) -> Document {
        let document_id = DocumentId::from_non_secret_parts(&[doc_id]);
        let mut document = store.document_by_id(&document_id).unwrap().unwrap();
        document.status = DocumentStatus::Searchable;
        document
    }

    fn retained_section_text_bytes(documents: &CurrentImportDocumentCache) -> usize {
        documents
            .documents
            .iter()
            .flat_map(|document| document.sections.iter())
            .map(|section| section.text.len())
            .sum()
    }

    fn test_document(doc_id: &str, status: DocumentStatus) -> Document {
        let content_hash = ContentDigest::from_bytes(doc_id.as_bytes());
        Document {
            id: DocumentId::from_non_secret_parts(&[doc_id]),
            source_uri: format!("file:///fixture/{doc_id}.txt"),
            normalized_path: format!("/fixture/{doc_id}.txt"),
            file_name: format!("{doc_id}.txt"),
            extension: FileExtension::Txt,
            byte_size: 128,
            mtime: UnixTimestamp::from_unix_seconds(1_700_000_001),
            content_hash: Some(content_hash.as_str().to_string()),
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
    fn import_root_rerun_with_unchanged_searchable_file_keeps_publication_stable() {
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
        let first_head = ready_search_head(&store);
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
        let second_head = ready_search_head(&store);
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
        assert_eq!(second_head.visible_epoch, first_head.visible_epoch);
        assert_eq!(
            second_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            first_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count()
        );
        assert_eq!(second_head.generation, first_head.generation);
        assert!(!format!("{second_head:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn import_root_rename_updates_path_without_reparse_or_index_rebuild() {
        let temp = TestDir::new("import-pipeline-rename-rerun");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(root.join("before")).unwrap();
        fs::write(
            root.join("before/synthetic-resume.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust"),
        )
        .unwrap();

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_192);
        let first_task = import_task("rename-first-import", root.to_str().unwrap(), first_now);
        store.insert_import_task(&first_task).unwrap();
        import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let first_document = store.visible_documents().unwrap().remove(0);
        let first_head = ready_search_head(&store);

        fs::create_dir_all(root.join("after")).unwrap();
        fs::rename(
            root.join("before/synthetic-resume.txt"),
            root.join("after/renamed-resume.txt"),
        )
        .unwrap();
        let second_now = UnixTimestamp::from_unix_seconds(1_700_000_193);
        let second_task = import_task("rename-second-import", root.to_str().unwrap(), second_now);
        store.insert_import_task(&second_task).unwrap();
        let summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();
        let second_document = store.visible_documents().unwrap().remove(0);
        let second_head = ready_search_head(&store);

        assert_eq!(summary.deleted_documents, 0);
        assert_eq!(first_document.id, second_document.id);
        assert!(second_document
            .normalized_path
            .ends_with("after/renamed-resume.txt"));
        assert_eq!(
            store
                .resume_versions_for_document(&second_document.id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(second_head, first_head);
    }

    #[test]
    fn strong_content_hash_matches_sha256_known_vector() {
        assert_eq!(
            ContentDigest::from_bytes(b"abc").as_str(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn import_root_strong_hash_detects_middle_only_change_hidden_from_quick_fingerprint() {
        let temp = TestDir::new("import-pipeline-strong-content-hash");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        let path = root.join("synthetic-resume.txt");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        let content = synthetic_large_resume_with_middle_skill("Rust");
        fs::write(&path, &content).unwrap();
        let original_mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let first_quick_fingerprint = fs_crawler::crawl_directory(&root)
            .unwrap()
            .files
            .remove(0)
            .fingerprint;

        let store = MetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_194);
        let first_task = import_task(
            "strong-hash-first-import",
            root.to_str().unwrap(),
            first_now,
        );
        store.insert_import_task(&first_task).unwrap();
        import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let first_document = store.visible_documents().unwrap().remove(0);
        let first_content_hash = first_document.content_hash.clone().unwrap();
        let first_head = ready_search_head(&store);
        let first_projection = store
            .active_search_projection_for_document(&first_document.id)
            .unwrap()
            .unwrap();
        let first_selection = SearchSelection {
            document_id: first_document.id.clone(),
            resume_version_id: first_projection.resume_version_id.clone(),
            visible_epoch: first_head.visible_epoch,
        };

        fs::write(&path, synthetic_large_resume_with_middle_skill("Java")).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_mtime))
            .unwrap();
        let second_quick_fingerprint = fs_crawler::crawl_directory(&root)
            .unwrap()
            .files
            .remove(0)
            .fingerprint;
        assert_eq!(
            first_quick_fingerprint.as_str(),
            second_quick_fingerprint.as_str()
        );

        let second_now = UnixTimestamp::from_unix_seconds(1_700_000_195);
        let second_task = import_task(
            "strong-hash-second-import",
            root.to_str().unwrap(),
            second_now,
        );
        store.insert_import_task(&second_task).unwrap();
        let summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();
        let second_document = store.visible_documents().unwrap().remove(0);
        let second_head = ready_search_head(&store);
        let second_projection = store
            .active_search_projection_for_document(&second_document.id)
            .unwrap()
            .unwrap();
        let first_version = store
            .resume_version_by_id(&first_projection.resume_version_id)
            .unwrap()
            .unwrap();
        let second_version = store
            .resume_version_by_id(&second_projection.resume_version_id)
            .unwrap()
            .unwrap();

        assert_eq!(summary.searchable_documents, 1);
        assert_eq!(summary.deleted_documents, 0);
        assert_eq!(first_document.id, second_document.id);
        assert_ne!(first_content_hash, second_document.content_hash.unwrap());
        assert_ne!(
            first_projection.resume_version_id,
            second_projection.resume_version_id
        );
        assert_ne!(first_head.generation, second_head.generation);
        assert!(first_version.clean_text.unwrap().contains("Rust"));
        assert!(second_version.clean_text.unwrap().contains("Java"));
        assert_eq!(
            resolve_selection(&store, &first_selection),
            SearchSelectionResolution::Stale
        );
    }

    fn synthetic_large_resume_with_middle_skill(skill: &str) -> Vec<u8> {
        let mut content = String::from(
            "Synthetic Candidate\nSummary\nEngineer\nExperience\nBuilt reliable systems\n",
        );
        content.push_str(&"a".repeat(5_000));
        content.push_str(skill);
        content.push_str(&"b".repeat(5_000));
        content.push_str("\nEducation\nSynthetic University\nSkills\nDatabases\n");
        content.into_bytes()
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
        let first_head = ready_search_head(&store);
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
        let second_head = ready_search_head(&store);
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
        assert_eq!(second_head.visible_epoch, first_head.visible_epoch);
        assert_eq!(
            second_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            first_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count()
        );
        assert_eq!(second_head.generation, first_head.generation);
        assert!(!format!("{second_head:?}").contains(root.to_str().unwrap()));

        let claimed_at = UnixTimestamp::from_unix_seconds(1_700_000_197);
        let claimed = store.claim_next_ocr_job(claimed_at).unwrap().unwrap();
        assert_eq!(claimed.job.status, IngestJobStatus::Running);
        assert_eq!(claimed.job.attempt_count, 1);
        store
            .finish_ocr_attempt_failure(
                &claimed,
                OcrAttemptFailure::Permanent,
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
        let requeued = store.ingest_job_by_id(&claimed.job.id).unwrap().unwrap();
        assert_eq!(requeued.status, IngestJobStatus::Queued);
        assert_eq!(requeued.attempt_count, 1);
        assert_eq!(requeued.queued_at, third_now);
        let reclaimed = store
            .claim_next_ocr_job(UnixTimestamp::from_unix_seconds(1_700_000_200))
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.job.attempt_count, 2);
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
        let observed_head = ready_search_head(&observed_store);
        let _ready_reader = open_fulltext_generation(&data_dir, &observed_head.generation);

        fs::write(&release_marker, b"release").unwrap();
        let summary = worker.join().unwrap();
        let final_store = MetaStore::open_data_dir(&data_dir).unwrap();
        final_store.run_migrations().unwrap();
        let final_head = ready_search_head(&final_store);

        assert_eq!(scope.files_discovered, 33);
        assert!(
            scope.searchable_documents > 0,
            "expected mid-run searchable progress before full import completion, got scope: {scope:?}"
        );
        assert!(
            status.searchable_documents > 0,
            "expected searchable documents to be visible before the final file completed, got status: {status:?}"
        );
        assert_eq!(observed_head.visible_epoch, 1);
        assert_eq!(
            observed_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            1
        );
        assert_eq!(summary.searchable_documents, 33);
        assert_eq!(final_head.visible_epoch, 2);
        assert_eq!(
            final_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            33
        );
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
        let observed_head = ready_search_head(&observed_store);
        let _ready_reader = open_fulltext_generation(&data_dir, &observed_head.generation);

        fs::write(&release_marker, b"release").unwrap();
        let summary = worker.join().unwrap();
        let final_store = MetaStore::open_data_dir(&data_dir).unwrap();
        final_store.run_migrations().unwrap();
        let final_head = ready_search_head(&final_store);

        assert_eq!(scope.files_discovered, 2);
        assert!(
            scope.searchable_documents > 0,
            "expected first searchable document to publish before batch threshold, got scope: {scope:?}"
        );
        assert!(
            status.searchable_documents > 0,
            "expected searchable status before the slow file completed, got status: {status:?}"
        );
        assert_eq!(observed_head.visible_epoch, 1);
        assert_eq!(
            observed_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            1
        );
        assert_eq!(summary.searchable_documents, 2);
        assert!(summary.milestone_timings.first_searchable.is_some());
        assert!(summary.milestone_timings.full_import_ready.is_some());
        assert!(summary.milestone_timings.full_index_ready.is_some());
        assert_eq!(final_head.visible_epoch, 2);
        assert_eq!(
            final_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            2
        );
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
