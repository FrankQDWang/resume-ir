use std::cell::Cell;
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::sync::{mpsc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use import_pipeline::{
    detect_ocr_page_count, index_claimed_ocr_text_with_policy_and_preparer, ocr_preclaim_decision,
    OcrPreclaimDecision,
};
use meta_store::{
    DocumentId, IngestJobFailureKind, OcrAttemptFailure, OcrPageCacheEntry, OcrPageCacheKey,
    OcrPageCacheStatus, OwnedMetaStore, UnixTimestamp, WorkerTaskKind,
};
use ocr_client::{
    inspect_tesseract_language_availability, CancellationToken, LocalOcrCommandClient,
    LocalOcrCommandSpec, LocalPdfRenderCommandClient, LocalPdfRenderCommandSpec, OcrClient,
    OcrErrorKind, OcrOptions, OcrPage, OcrPageRequest, OcrWorkerBudget,
    TesseractLanguageAvailability, TesseractOcrClient, TesseractOcrSpec,
};

use crate::daemon_error::{DaemonError, Result};
use crate::daemon_policy::STALE_INGEST_JOB_SECONDS;
use crate::run_options::RunOptions;
use crate::source_file_authority::SourceFileError;
use crate::worker_output::OcrWorkerSummary;
use crate::worker_time::{current_timestamp, timestamp_minus_seconds};

static ACTIVE_OCR_DOCUMENTS: OnceLock<(Mutex<HashSet<String>>, Condvar)> = OnceLock::new();

pub(crate) fn run_ocr_worker_once(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    claim_allowed: impl Fn() -> bool,
) -> Result<OcrWorkerSummary> {
    run_ocr_worker_once_with_handoff(data_dir, store, options, claim_allowed, None)
}

pub(crate) fn run_ocr_worker_once_with_handoff(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    claim_allowed: impl Fn() -> bool,
    generation_handoff: Option<&crate::ipc::search_service::GenerationHandoff>,
) -> Result<OcrWorkerSummary> {
    if let Some(handoff) = generation_handoff {
        let prepared = handoff.prepare_runtime().map_err(|_| {
            DaemonError::control_plane("query runtime prepare control became unresponsive")
        })?;
        if !prepared {
            return Ok(OcrWorkerSummary::default());
        }
    }
    let now = current_timestamp()?;
    match ocr_preclaim_decision(store).map_err(DaemonError::import)? {
        OcrPreclaimDecision::Ready => {}
        OcrPreclaimDecision::NotReady(_) => return Ok(OcrWorkerSummary::default()),
    }
    if store
        .worker_task_control(WorkerTaskKind::Ocr)
        .map_err(DaemonError::store)?
        .paused
    {
        return Ok(OcrWorkerSummary {
            paused: true,
            ..OcrWorkerSummary::default()
        });
    }

    if options.ocr_command.is_none() && options.ocr_tesseract_command.is_none() {
        return Err(DaemonError::configuration_invalid(
            "ocr worker blocked: local OCR command not configured",
        ));
    }
    let runtime = match PreparedOcrRuntime::new(options) {
        Ok(runtime) => runtime,
        Err(PreparedOcrRuntimeFailure::Ocr(reason)) => {
            return Ok(OcrWorkerSummary {
                runtime_unavailable: Some(reason),
                ..OcrWorkerSummary::default()
            });
        }
        Err(PreparedOcrRuntimeFailure::Pdfium(reason)) => {
            return Ok(OcrWorkerSummary {
                pdfium_unavailable: Some(reason),
                ..OcrWorkerSummary::default()
            });
        }
    };

    let stale_recovered = recover_stale_ingest_jobs(store, now)?;
    if !claim_allowed() {
        return Ok(OcrWorkerSummary {
            stale_recovered,
            ..OcrWorkerSummary::default()
        });
    }
    let Some(job) = store.claim_next_ocr_job(now).map_err(DaemonError::store)? else {
        return Ok(OcrWorkerSummary {
            stale_recovered,
            ..OcrWorkerSummary::default()
        });
    };
    let _active_claim = ActiveOcrClaim::register(&job.job.document_id)?;
    if !store
        .ocr_claim_is_current(&job)
        .map_err(DaemonError::store)?
    {
        store
            .discard_ocr_claim_for_source_change(&job, now)
            .map_err(DaemonError::store)?;
        return Ok(OcrWorkerSummary {
            stale_recovered,
            ..OcrWorkerSummary::default()
        });
    }

    let mut summary = match run_claimed_ocr_job(
        data_dir,
        store,
        &job,
        options,
        &runtime,
        now,
        generation_handoff,
    ) {
        Ok(summary) => summary,
        Err(error) => {
            mark_ocr_job_failed_retryable(store, &job, now)?;
            return Err(error);
        }
    };
    summary.stale_recovered = stale_recovered;
    Ok(summary)
}

pub(crate) fn wait_for_documents_to_quiesce(documents: &[DocumentId], timeout: Duration) -> bool {
    if documents.is_empty() {
        return true;
    }
    let document_ids = documents
        .iter()
        .map(|document| document.as_str())
        .collect::<HashSet<_>>();
    let (active, changed) =
        ACTIVE_OCR_DOCUMENTS.get_or_init(|| (Mutex::new(HashSet::new()), Condvar::new()));
    let Ok(mut active) = active.lock() else {
        return false;
    };
    let deadline = Instant::now() + timeout;
    while active
        .iter()
        .any(|document| document_ids.contains(document.as_str()))
    {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let Ok((next, result)) = changed.wait_timeout(active, remaining) else {
            return false;
        };
        active = next;
        if result.timed_out()
            && active
                .iter()
                .any(|document| document_ids.contains(document.as_str()))
        {
            return false;
        }
    }
    true
}

struct ActiveOcrClaim {
    document_id: String,
}

impl ActiveOcrClaim {
    fn register(document_id: &DocumentId) -> Result<Self> {
        let (active, _) =
            ACTIVE_OCR_DOCUMENTS.get_or_init(|| (Mutex::new(HashSet::new()), Condvar::new()));
        let mut active = active
            .lock()
            .map_err(|_| DaemonError::control_plane("OCR activity registry is unavailable"))?;
        let document_id = document_id.as_str().to_string();
        if !active.insert(document_id.clone()) {
            return Err(DaemonError::control_plane(
                "OCR document was claimed concurrently",
            ));
        }
        Ok(Self { document_id })
    }
}

impl Drop for ActiveOcrClaim {
    fn drop(&mut self) {
        let (active, changed) =
            ACTIVE_OCR_DOCUMENTS.get_or_init(|| (Mutex::new(HashSet::new()), Condvar::new()));
        if let Ok(mut active) = active.lock() {
            active.remove(&self.document_id);
            changed.notify_all();
        }
    }
}

pub(crate) fn run_ocr_worker_batch_with_handoff(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    jobs_per_tick: usize,
    claim_allowed: impl Fn() -> bool,
    generation_handoff: Option<&crate::ipc::search_service::GenerationHandoff>,
) -> Result<OcrWorkerSummary> {
    let mut aggregate = OcrWorkerSummary::default();
    for _ in 0..jobs_per_tick {
        if !claim_allowed() {
            break;
        }
        let summary = run_ocr_worker_once_with_handoff(
            data_dir,
            store,
            options,
            &claim_allowed,
            generation_handoff,
        )?;
        let stop_after_summary = summary.paused
            || summary.runtime_unavailable.is_some()
            || summary.pdfium_unavailable.is_some()
            || (summary.processed == 0 && summary.failed == 0);
        aggregate.extend(summary);
        if stop_after_summary {
            break;
        }
    }
    Ok(aggregate)
}

fn run_claimed_ocr_job(
    data_dir: &Path,
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    options: &RunOptions,
    runtime: &PreparedOcrRuntime,
    now: UnixTimestamp,
    generation_handoff: Option<&crate::ipc::search_service::GenerationHandoff>,
) -> Result<OcrWorkerSummary> {
    let Some(document) = store
        .document_by_id(&job.job.document_id)
        .map_err(DaemonError::store)?
    else {
        mark_ocr_job_failed_permanent(store, job, now)?;
        return Ok(OcrWorkerSummary {
            failed: 1,
            ..OcrWorkerSummary::default()
        });
    };
    let content_hash = job.source_fingerprint().to_string();

    let cancellation = CancellationToken::new();
    let _claim_monitor = OcrClaimMonitor::start(
        store.open_sibling().map_err(DaemonError::store)?,
        job.clone(),
        cancellation.clone(),
    )?;
    let verified = match crate::source_file_authority::open_verified_revision_with_cancellation(
        store,
        &job.job.document_id,
        job.source_revision_id(),
        || cancellation.is_cancelled(),
    ) {
        Ok(verified) => verified,
        Err(
            SourceFileError::SourceChanged
            | SourceFileError::StaleSelection
            | SourceFileError::NotFound
            | SourceFileError::Cancelled,
        ) => {
            store
                .discard_ocr_claim_for_source_change(job, now)
                .map_err(DaemonError::store)?;
            return Ok(OcrWorkerSummary::default());
        }
        Err(SourceFileError::UnsafePath | SourceFileError::UnsupportedFormat) => {
            mark_ocr_job_failed_permanent(store, job, now)?;
            return Ok(OcrWorkerSummary {
                failed: 1,
                ..OcrWorkerSummary::default()
            });
        }
        Err(
            SourceFileError::SourceMissing
            | SourceFileError::MetadataUnavailable
            | SourceFileError::Io,
        ) => {
            mark_ocr_job_failed_retryable(store, job, now)?;
            return Ok(OcrWorkerSummary {
                failed: 1,
                ..OcrWorkerSummary::default()
            });
        }
    };
    let expected_bytes = verified.byte_size();
    let (mut source, _, _) = verified.into_parts();
    let mut bytes = Vec::with_capacity(
        usize::try_from(expected_bytes)
            .unwrap_or(64 * 1024)
            .min(256 * 1024 * 1024),
    );
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        if cancellation.is_cancelled() {
            store
                .discard_ocr_claim_for_source_change(job, now)
                .map_err(DaemonError::store)?;
            return Ok(OcrWorkerSummary::default());
        }
        let read = match source.read(&mut buffer) {
            Ok(read) => read,
            Err(_) => {
                mark_ocr_job_failed_retryable(store, job, now)?;
                return Ok(OcrWorkerSummary {
                    failed: 1,
                    ..OcrWorkerSummary::default()
                });
            }
        };
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if bytes.len() as u64 > expected_bytes {
            store
                .discard_ocr_claim_for_source_change(job, now)
                .map_err(DaemonError::store)?;
            return Ok(OcrWorkerSummary::default());
        }
    }
    if bytes.len() as u64 != expected_bytes {
        store
            .discard_ocr_claim_for_source_change(job, now)
            .map_err(DaemonError::store)?;
        return Ok(OcrWorkerSummary::default());
    }
    let page_count = match detect_ocr_page_count(&document.extension, &bytes) {
        Ok(page_count) => page_count,
        Err(error) => return Err(DaemonError::import(error)),
    };
    if page_count > options.ocr_max_pages_per_document {
        mark_ocr_job_failed_retryable_with_failure_kind(
            store,
            job,
            IngestJobFailureKind::OcrPageBudgetExceeded,
            now,
        )?;
        return Ok(OcrWorkerSummary {
            failed: 1,
            ..OcrWorkerSummary::default()
        });
    }
    let budget = OcrWorkerBudget::new(options.ocr_page_timeout_ms).map_err(DaemonError::ocr)?;
    let ocr_options = OcrOptions::new(options.ocr_lang.as_str(), options.ocr_profile.as_str())
        .map_err(DaemonError::ocr)?;
    let mut page_texts = Vec::new();
    let mut confidence_sum = 0.0_f32;
    let mut confidence_count = 0_usize;
    let mut cache_writes = 0_usize;
    let mut cache_hits = 0_usize;

    for page_no in 1..=page_count {
        if cancellation.is_cancelled() {
            store
                .discard_ocr_claim_for_source_change(job, now)
                .map_err(DaemonError::store)?;
            return Ok(OcrWorkerSummary {
                cache_writes,
                cache_hits,
                ..OcrWorkerSummary::default()
            });
        }
        let cache_key = OcrPageCacheKey::new(
            content_hash.clone(),
            page_no,
            options.ocr_render_dpi,
            options.ocr_lang.as_str(),
            options.ocr_profile.as_str(),
        )
        .map_err(DaemonError::store)?;

        if let Some(entry) = store
            .ocr_page_cache_entry(&cache_key)
            .map_err(DaemonError::store)?
            .filter(|entry| entry.status() == OcrPageCacheStatus::Succeeded)
        {
            page_texts.push(entry.text().unwrap_or("").to_string());
            if let Some(confidence) = entry.confidence() {
                confidence_sum += confidence;
                confidence_count += 1;
            }
            cache_hits += 1;
            continue;
        }

        if let Some(tesseract_command) = runtime.tesseract_command() {
            match inspect_tesseract_language_availability(
                tesseract_command,
                options.ocr_lang.as_str(),
            ) {
                TesseractLanguageAvailability::Available => {}
                TesseractLanguageAvailability::Missing => {
                    let entry =
                        OcrPageCacheEntry::failed_retryable(cache_key, "LanguageUnavailable", now)
                            .map_err(DaemonError::store)?;
                    store
                        .upsert_ocr_page_cache_entry(&entry)
                        .map_err(DaemonError::store)?;
                    mark_ocr_job_failed_retryable(store, job, now)?;
                    return Ok(OcrWorkerSummary {
                        failed: 1,
                        runtime_unavailable: Some(crate::ipc::OptionalRuntimeReason::Invalid),
                        ..OcrWorkerSummary::default()
                    });
                }
                TesseractLanguageAvailability::Unknown => {
                    let entry =
                        OcrPageCacheEntry::failed_retryable(cache_key, "WorkerUnavailable", now)
                            .map_err(DaemonError::store)?;
                    store
                        .upsert_ocr_page_cache_entry(&entry)
                        .map_err(DaemonError::store)?;
                    mark_ocr_job_failed_retryable(store, job, now)?;
                    return Ok(OcrWorkerSummary {
                        failed: 1,
                        runtime_unavailable: Some(crate::ipc::OptionalRuntimeReason::StartFailed),
                        ..OcrWorkerSummary::default()
                    });
                }
            }
        }

        let rendered_page = match runtime.renderer.render_page(
            &bytes,
            page_no,
            options.ocr_render_dpi,
            budget,
            &cancellation,
        ) {
            Ok(rendered_page) => rendered_page,
            Err(error) => {
                if cancelled_obsolete_claim(store, job, error.kind(), now)? {
                    return Ok(OcrWorkerSummary {
                        cache_writes,
                        cache_hits,
                        ..OcrWorkerSummary::default()
                    });
                }
                let permanent = ocr_failure_is_permanent(error.kind());
                let entry = if permanent {
                    OcrPageCacheEntry::failed_permanent(
                        cache_key,
                        format!("{:?}", error.kind()),
                        now,
                    )
                } else {
                    OcrPageCacheEntry::failed_retryable(
                        cache_key,
                        format!("{:?}", error.kind()),
                        now,
                    )
                }
                .map_err(DaemonError::store)?;
                store
                    .upsert_ocr_page_cache_entry(&entry)
                    .map_err(DaemonError::store)?;
                if permanent {
                    mark_ocr_job_failed_permanent(store, job, now)?;
                } else {
                    mark_ocr_job_failed_retryable(store, job, now)?;
                }
                return Ok(OcrWorkerSummary {
                    failed: 1,
                    pdfium_unavailable: runtime_failure_reason(error.kind()),
                    ..OcrWorkerSummary::default()
                });
            }
        };
        let request =
            OcrPageRequest::new(rendered_page, ocr_options.clone()).map_err(DaemonError::ocr)?;

        let page_result = runtime
            .engine
            .recognize_page(request, budget, &cancellation);
        let page = match page_result {
            Ok(page) => page,
            Err(error) => {
                if cancelled_obsolete_claim(store, job, error.kind(), now)? {
                    return Ok(OcrWorkerSummary {
                        cache_writes,
                        cache_hits,
                        ..OcrWorkerSummary::default()
                    });
                }
                let permanent = ocr_failure_is_permanent(error.kind());
                let entry = if permanent {
                    OcrPageCacheEntry::failed_permanent(
                        cache_key,
                        format!("{:?}", error.kind()),
                        now,
                    )
                } else {
                    OcrPageCacheEntry::failed_retryable(
                        cache_key,
                        format!("{:?}", error.kind()),
                        now,
                    )
                }
                .map_err(DaemonError::store)?;
                store
                    .upsert_ocr_page_cache_entry(&entry)
                    .map_err(DaemonError::store)?;
                if permanent {
                    mark_ocr_job_failed_permanent(store, job, now)?;
                } else {
                    mark_ocr_job_failed_retryable(store, job, now)?;
                }
                return Ok(OcrWorkerSummary {
                    failed: 1,
                    runtime_unavailable: runtime_failure_reason(error.kind()),
                    ..OcrWorkerSummary::default()
                });
            }
        };
        let word_boxes = ocr_word_boxes_for_cache(&page)?;
        let entry = OcrPageCacheEntry::succeeded_with_word_boxes(
            cache_key,
            page.text(),
            page.confidence(),
            page.engine_profile(),
            page.duration_ms(),
            word_boxes,
            now,
        )
        .map_err(DaemonError::store)?;
        store
            .upsert_ocr_page_cache_entry(&entry)
            .map_err(DaemonError::store)?;
        page_texts.push(page.text().to_string());
        confidence_sum += page.confidence();
        confidence_count += 1;
        cache_writes += 1;
    }

    let combined_text = page_texts.join("\n");
    let confidence = (confidence_count > 0).then_some(confidence_sum / confidence_count as f32);
    match crate::source_file_authority::open_verified_revision_with_cancellation(
        store,
        &job.job.document_id,
        job.source_revision_id(),
        || cancellation.is_cancelled(),
    ) {
        Ok(_) => {}
        Err(
            SourceFileError::SourceChanged
            | SourceFileError::StaleSelection
            | SourceFileError::NotFound
            | SourceFileError::Cancelled,
        ) => {
            store
                .discard_ocr_claim_for_source_change(job, now)
                .map_err(DaemonError::store)?;
            return Ok(OcrWorkerSummary {
                cache_writes,
                cache_hits,
                ..OcrWorkerSummary::default()
            });
        }
        Err(SourceFileError::UnsafePath | SourceFileError::UnsupportedFormat) => {
            mark_ocr_job_failed_permanent(store, job, now)?;
            return Ok(OcrWorkerSummary {
                failed: 1,
                cache_writes,
                cache_hits,
                ..OcrWorkerSummary::default()
            });
        }
        Err(
            SourceFileError::SourceMissing
            | SourceFileError::MetadataUnavailable
            | SourceFileError::Io,
        ) => {
            mark_ocr_job_failed_retryable(store, job, now)?;
            return Ok(OcrWorkerSummary {
                failed: 1,
                cache_writes,
                cache_hits,
                ..OcrWorkerSummary::default()
            });
        }
    }
    let generation_control_unresponsive = Cell::new(false);
    let prepare_generation =
        |publication: &meta_store::SearchPublicationRecord,
         projections: &[meta_store::ActiveSearchProjection]| {
            let prepared =
                search_runtime::PreparedQueryGeneration::open(data_dir, publication, projections)
                    .map_err(|_| {
                    import_pipeline::ImportPipelineError::query_generation_preparation()
                })?;
            let staged = generation_handoff.is_some_and(|handoff| match handoff.stage(prepared) {
                Ok(staged) => staged,
                Err(_) => {
                    generation_control_unresponsive.set(true);
                    false
                }
            });
            if staged {
                Ok(())
            } else {
                Err(import_pipeline::ImportPipelineError::query_generation_preparation())
            }
        };
    let outcome = index_claimed_ocr_text_with_policy_and_preparer(
        data_dir,
        store,
        job,
        &combined_text,
        confidence,
        Some(page_count),
        now,
        &options.linear_promotion,
        &options.search_vectorization,
        generation_handoff.map(|_| {
            &prepare_generation
                as &dyn Fn(
                    &meta_store::SearchPublicationRecord,
                    &[meta_store::ActiveSearchProjection],
                ) -> import_pipeline::Result<()>
        }),
    );
    if generation_control_unresponsive.get() {
        return Err(DaemonError::control_plane(
            "query generation install control became unresponsive",
        ));
    }
    if let Some(generation_handoff) = generation_handoff {
        let disposition = if matches!(
            &outcome,
            Ok(import_pipeline::OcrTextIndexOutcome::Committed(_))
        ) {
            crate::ipc::search_service::PublicationDisposition::Committed
        } else {
            crate::ipc::search_service::PublicationDisposition::Aborted
        };
        let finalized = generation_handoff
            .finish_publication(disposition)
            .map_err(|_| {
                DaemonError::control_plane("query generation finalize control became unresponsive")
            })?;
        if disposition == crate::ipc::search_service::PublicationDisposition::Committed
            && !finalized
        {
            return Err(DaemonError::control_plane(
                "prepared query generation could not be activated",
            ));
        }
    }
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => return Err(DaemonError::import(error)),
    };
    Ok(OcrWorkerSummary {
        processed: usize::from(matches!(
            outcome,
            import_pipeline::OcrTextIndexOutcome::Committed(_)
        )),
        cache_writes,
        cache_hits,
        ..OcrWorkerSummary::default()
    })
}

fn cancelled_obsolete_claim(
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    failure: OcrErrorKind,
    now: UnixTimestamp,
) -> Result<bool> {
    if failure != OcrErrorKind::Cancelled
        || store
            .ocr_claim_is_current(job)
            .map_err(DaemonError::store)?
    {
        return Ok(false);
    }
    store
        .discard_ocr_claim_for_source_change(job, now)
        .map_err(DaemonError::store)?;
    Ok(true)
}

struct OcrClaimMonitor {
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<()>>,
}

impl OcrClaimMonitor {
    fn start(
        store: OwnedMetaStore,
        job: meta_store::ClaimedOcrJob,
        cancellation: CancellationToken,
    ) -> Result<Self> {
        let (stop, stopped) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("resume-ocr-claim-monitor".to_string())
            .spawn(move || loop {
                match stopped.recv_timeout(Duration::from_millis(50)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if !store.ocr_claim_is_current(&job).unwrap_or(false) {
                    cancellation.cancel();
                    return;
                }
            })
            .map_err(|_| {
                DaemonError::recoverable_dependency("OCR claim monitor could not start")
            })?;
        Ok(Self {
            stop: Some(stop),
            worker: Some(worker),
        })
    }
}

impl Drop for OcrClaimMonitor {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn ocr_failure_is_permanent(kind: OcrErrorKind) -> bool {
    matches!(kind, OcrErrorKind::InvalidRequest | OcrErrorKind::Disabled)
}

enum PreparedOcrEngine {
    Local(LocalOcrCommandClient),
    Tesseract {
        command: std::path::PathBuf,
        client: TesseractOcrClient,
    },
}

impl PreparedOcrEngine {
    fn recognize_page(
        &self,
        request: OcrPageRequest,
        budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> std::result::Result<OcrPage, ocr_client::OcrError> {
        match self {
            Self::Local(client) => client.recognize_page(request, budget, cancellation),
            Self::Tesseract { client, .. } => client.recognize_page(request, budget, cancellation),
        }
    }

    fn tesseract_command(&self) -> Option<&Path> {
        match self {
            Self::Local(_) => None,
            Self::Tesseract { command, .. } => Some(command),
        }
    }
}

struct PreparedOcrRuntime {
    engine: PreparedOcrEngine,
    renderer: LocalPdfRenderCommandClient,
}

enum PreparedOcrRuntimeFailure {
    Ocr(crate::ipc::OptionalRuntimeReason),
    Pdfium(crate::ipc::OptionalRuntimeReason),
}

impl PreparedOcrRuntime {
    fn new(options: &RunOptions) -> std::result::Result<Self, PreparedOcrRuntimeFailure> {
        let engine = options
            .ocr_command
            .as_deref()
            .or(options.ocr_tesseract_command.as_deref())
            .ok_or(PreparedOcrRuntimeFailure::Ocr(
                crate::ipc::OptionalRuntimeReason::NotConfigured,
            ))?;
        let renderer =
            options
                .pdf_render_command
                .as_deref()
                .ok_or(PreparedOcrRuntimeFailure::Pdfium(
                    crate::ipc::OptionalRuntimeReason::NotConfigured,
                ))?;
        let tessdata = std::env::var_os("TESSDATA_PREFIX").map(std::path::PathBuf::from);
        let engine = crate::runtime_pack::validated_ocr_engine_with_cancel(
            engine,
            &options.ocr_lang,
            tessdata.as_deref(),
            &|| false,
        )
        .map_err(PreparedOcrRuntimeFailure::Ocr)?;
        let renderer = crate::runtime_pack::validated_pdf_renderer(renderer)
            .map_err(PreparedOcrRuntimeFailure::Pdfium)?
            .into_path();
        let engine = if options.ocr_command.is_some() {
            PreparedOcrEngine::Local(LocalOcrCommandClient::new(
                LocalOcrCommandSpec::new(
                    engine,
                    Vec::<String>::new(),
                    options.ocr_engine_profile.as_str(),
                )
                .map_err(|_| {
                    PreparedOcrRuntimeFailure::Ocr(crate::ipc::OptionalRuntimeReason::StartFailed)
                })?,
            ))
        } else {
            let client = TesseractOcrClient::new(
                TesseractOcrSpec::new(&engine, options.ocr_engine_profile.as_str()).map_err(
                    |_| {
                        PreparedOcrRuntimeFailure::Ocr(
                            crate::ipc::OptionalRuntimeReason::StartFailed,
                        )
                    },
                )?,
            );
            PreparedOcrEngine::Tesseract {
                command: engine,
                client,
            }
        };
        let renderer = LocalPdfRenderCommandClient::new(
            LocalPdfRenderCommandSpec::new(renderer, Vec::<String>::new()).map_err(|_| {
                PreparedOcrRuntimeFailure::Pdfium(crate::ipc::OptionalRuntimeReason::StartFailed)
            })?,
        );
        Ok(Self { engine, renderer })
    }

    fn tesseract_command(&self) -> Option<&Path> {
        self.engine.tesseract_command()
    }
}

fn runtime_failure_reason(kind: OcrErrorKind) -> Option<crate::ipc::OptionalRuntimeReason> {
    matches!(
        kind,
        OcrErrorKind::WorkerUnavailable | OcrErrorKind::EngineFailed
    )
    .then_some(crate::ipc::OptionalRuntimeReason::StartFailed)
}

fn ocr_word_boxes_for_cache(page: &ocr_client::OcrPage) -> Result<Vec<meta_store::OcrWordBox>> {
    page.word_boxes()
        .iter()
        .map(|word_box| {
            meta_store::OcrWordBox::new(
                word_box.text(),
                word_box.left(),
                word_box.top(),
                word_box.width(),
                word_box.height(),
                word_box.confidence(),
            )
            .map_err(DaemonError::store)
        })
        .collect()
}

fn mark_ocr_job_failed_retryable(
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .finish_ocr_attempt_failure(job, OcrAttemptFailure::Retryable, now)
        .map(|_| ())
        .map_err(DaemonError::store)
}

fn mark_ocr_job_failed_retryable_with_failure_kind(
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    failure_kind: IngestJobFailureKind,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .finish_ocr_attempt_failure(job, OcrAttemptFailure::RetryableWithKind(failure_kind), now)
        .map(|_| ())
        .map_err(DaemonError::store)
}

fn mark_ocr_job_failed_permanent(
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    now: UnixTimestamp,
) -> Result<()> {
    store
        .finish_ocr_attempt_failure(job, OcrAttemptFailure::Permanent, now)
        .map(|_| ())
        .map_err(DaemonError::store)
}

fn recover_stale_ingest_jobs(store: &OwnedMetaStore, now: UnixTimestamp) -> Result<usize> {
    store
        .recover_stale_running_ingest_jobs(
            now,
            timestamp_minus_seconds(now, STALE_INGEST_JOB_SECONDS),
        )
        .map_err(DaemonError::store)
}
