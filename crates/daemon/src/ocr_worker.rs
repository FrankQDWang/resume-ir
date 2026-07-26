use std::fs;
use std::path::Path;

use import_pipeline::{
    detect_ocr_page_count, index_claimed_ocr_text_with_policy, ocr_preclaim_decision,
    OcrPreclaimDecision,
};
use meta_store::{
    IngestJobFailureKind, OcrAttemptFailure, OcrPageCacheEntry, OcrPageCacheKey,
    OcrPageCacheStatus, OwnedMetaStore, UnixTimestamp, WorkerTaskKind,
};
use ocr_client::{
    inspect_tesseract_language_availability, CancellationToken, LocalOcrCommandClient,
    LocalOcrCommandSpec, LocalPdfRenderCommandClient, LocalPdfRenderCommandSpec, OcrClient,
    OcrOptions, OcrPage, OcrPageRequest, OcrWorkerBudget, PdftoppmPdfRenderer, PdftoppmRenderSpec,
    RenderedPage, TesseractLanguageAvailability, TesseractOcrClient, TesseractOcrSpec,
};

use crate::daemon_error::{DaemonError, Result};
use crate::daemon_policy::STALE_INGEST_JOB_SECONDS;
use crate::run_options::RunOptions;
use crate::worker_output::OcrWorkerSummary;
use crate::worker_time::{current_timestamp, timestamp_minus_seconds};

pub(crate) fn run_ocr_worker_once(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    claim_allowed: impl Fn() -> bool,
) -> Result<OcrWorkerSummary> {
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
        Err(reason) => {
            return Ok(OcrWorkerSummary {
                runtime_unavailable: Some(reason),
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

    let mut summary = match run_claimed_ocr_job(data_dir, store, &job, options, &runtime, now) {
        Ok(summary) => summary,
        Err(error) => {
            mark_ocr_job_failed_retryable(store, &job, now)?;
            return Err(error);
        }
    };
    summary.stale_recovered = stale_recovered;
    Ok(summary)
}

pub(crate) fn run_ocr_worker_batch(
    data_dir: &Path,
    store: &OwnedMetaStore,
    options: &RunOptions,
    jobs_per_tick: usize,
    claim_allowed: impl Fn() -> bool,
) -> Result<OcrWorkerSummary> {
    let mut aggregate = OcrWorkerSummary::default();
    for _ in 0..jobs_per_tick {
        if !claim_allowed() {
            break;
        }
        let summary = run_ocr_worker_once(data_dir, store, options, &claim_allowed)?;
        let stop_after_summary = summary.paused
            || summary.runtime_unavailable.is_some()
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

    let bytes = match fs::read(&document.normalized_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            mark_ocr_job_failed_retryable(store, job, now)?;
            return Ok(OcrWorkerSummary {
                failed: 1,
                ..OcrWorkerSummary::default()
            });
        }
    };
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
    let cancellation = CancellationToken::new();
    let ocr_options = OcrOptions::new(options.ocr_lang.as_str(), options.ocr_profile.as_str())
        .map_err(DaemonError::ocr)?;
    let mut page_texts = Vec::new();
    let mut confidence_sum = 0.0_f32;
    let mut confidence_count = 0_usize;
    let mut cache_writes = 0_usize;
    let mut cache_hits = 0_usize;

    for page_no in 1..=page_count {
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
                let entry = OcrPageCacheEntry::failed_retryable(
                    cache_key,
                    format!("{:?}", error.kind()),
                    now,
                )
                .map_err(DaemonError::store)?;
                store
                    .upsert_ocr_page_cache_entry(&entry)
                    .map_err(DaemonError::store)?;
                mark_ocr_job_failed_retryable(store, job, now)?;
                return Ok(OcrWorkerSummary {
                    failed: 1,
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
                let entry = OcrPageCacheEntry::failed_retryable(
                    cache_key,
                    format!("{:?}", error.kind()),
                    now,
                )
                .map_err(DaemonError::store)?;
                store
                    .upsert_ocr_page_cache_entry(&entry)
                    .map_err(DaemonError::store)?;
                mark_ocr_job_failed_retryable(store, job, now)?;
                return Ok(OcrWorkerSummary {
                    failed: 1,
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
    let outcome = match index_claimed_ocr_text_with_policy(
        data_dir,
        store,
        job,
        &combined_text,
        confidence,
        Some(page_count),
        now,
        &options.linear_promotion,
        &options.search_vectorization,
    ) {
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

enum PreparedPdfRenderer {
    Local(LocalPdfRenderCommandClient),
    Pdftoppm(PdftoppmPdfRenderer),
}

impl PreparedPdfRenderer {
    fn render_page(
        &self,
        document: &[u8],
        page_no: u32,
        dpi: u32,
        budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> std::result::Result<RenderedPage, ocr_client::OcrError> {
        match self {
            Self::Local(renderer) => {
                renderer.render_page(document, page_no, dpi, budget, cancellation)
            }
            Self::Pdftoppm(renderer) => {
                renderer.render_page(document, page_no, dpi, budget, cancellation)
            }
        }
    }
}

struct PreparedOcrRuntime {
    engine: PreparedOcrEngine,
    renderer: PreparedPdfRenderer,
}

impl PreparedOcrRuntime {
    fn new(options: &RunOptions) -> std::result::Result<Self, crate::ipc::OptionalRuntimeReason> {
        let engine = options
            .ocr_command
            .as_deref()
            .or(options.ocr_tesseract_command.as_deref())
            .ok_or(crate::ipc::OptionalRuntimeReason::NotConfigured)?;
        let renderer = options
            .ocr_render_command
            .as_deref()
            .or(options.ocr_pdftoppm_command.as_deref());
        let tessdata = std::env::var_os("TESSDATA_PREFIX").map(std::path::PathBuf::from);
        let validated = crate::runtime_pack::validated_ocr_runtime(
            engine,
            renderer,
            &options.ocr_lang,
            tessdata.as_deref(),
        )?;
        let (engine, renderer) = validated.into_paths();
        let engine = if options.ocr_command.is_some() {
            PreparedOcrEngine::Local(LocalOcrCommandClient::new(
                LocalOcrCommandSpec::new(
                    engine,
                    Vec::<String>::new(),
                    options.ocr_engine_profile.as_str(),
                )
                .map_err(|_| crate::ipc::OptionalRuntimeReason::StartFailed)?,
            ))
        } else {
            let client = TesseractOcrClient::new(
                TesseractOcrSpec::new(&engine, options.ocr_engine_profile.as_str())
                    .map_err(|_| crate::ipc::OptionalRuntimeReason::StartFailed)?,
            );
            PreparedOcrEngine::Tesseract {
                command: engine,
                client,
            }
        };
        let renderer = if options.ocr_render_command.is_some() {
            PreparedPdfRenderer::Local(LocalPdfRenderCommandClient::new(
                LocalPdfRenderCommandSpec::new(renderer, Vec::<String>::new())
                    .map_err(|_| crate::ipc::OptionalRuntimeReason::StartFailed)?,
            ))
        } else {
            PreparedPdfRenderer::Pdftoppm(PdftoppmPdfRenderer::new(
                PdftoppmRenderSpec::new(renderer)
                    .map_err(|_| crate::ipc::OptionalRuntimeReason::StartFailed)?,
            ))
        };
        Ok(Self { engine, renderer })
    }

    fn tesseract_command(&self) -> Option<&Path> {
        self.engine.tesseract_command()
    }
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
