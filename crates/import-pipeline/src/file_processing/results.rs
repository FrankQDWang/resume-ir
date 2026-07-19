use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use meta_store::{DocumentStatus, OwnedMetaStore, UnixTimestamp};
use resume_classifier::LinearPromotionPolicy;

use super::model::{
    ImportFileResult, ParseWorkItem, ParseWorkOutcome, ParseWorkResult, ParseWorkerClock,
    ProcessedImportFile,
};
use super::persistence::{
    mark_ocr_required_and_enqueue, persist_non_searchable, persist_source_revision_failure,
    prepare_pending_searchable_document,
};
use crate::source_dispositions::ProcessedFile;
use crate::timing::measure_result_stage;
use crate::{
    ImportCancelCheckPhase, ImportPipelineError, ImportPipelineErrorKind, ImportWorkerMetrics,
    Result, PARSE_RESULT_CANCEL_POLL_INTERVAL_MS,
};

pub(crate) fn send_parse_work_with_backpressure(
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

pub(crate) fn drain_available_parse_results(
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

pub(crate) fn recv_parse_result_with_cancel_poll(
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

pub(crate) fn insert_parse_result(
    pending_results: &mut BTreeMap<usize, ImportFileResult>,
    result: ParseWorkResult,
) -> Result<()> {
    insert_import_file_result(
        pending_results,
        result.index,
        ImportFileResult::Parsed(result),
    )
}

pub(crate) fn insert_import_file_result(
    pending_results: &mut BTreeMap<usize, ImportFileResult>,
    index: usize,
    result: ImportFileResult,
) -> Result<()> {
    if pending_results.insert(index, result).is_some() {
        return Err(parallel_parse_error());
    }
    Ok(())
}

pub(crate) fn commit_parse_work_result(
    data_dir: &Path,
    store: &OwnedMetaStore,
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
            let source_revision_id = source_revision.id.clone();
            let resume_version_id = version.id.clone();
            document.status = DocumentStatus::Excluded;
            document.updated_at = now;
            measure_result_stage(db_timing, || {
                persist_non_searchable(store, &document, &source_revision, &version, decision, now)
            })?;
            ProcessedFile::Excluded {
                document: Box::new(document),
                source_revision_id,
                resume_version_id,
            }
        }
        ParseWorkOutcome::OcrRequired => {
            let source_revision_id = source_revision.id.clone();
            ProcessedFile::OcrRequired {
                ocr_job_queued: measure_result_stage(db_timing, || {
                    mark_ocr_required_and_enqueue(
                        store,
                        &mut document,
                        &source_revision,
                        now,
                        linear_promotion,
                    )
                })?,
                source_revision_id,
            }
        }
        ParseWorkOutcome::Failed { status, kind } => {
            let source_revision_id = source_revision.id.clone();
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
            ProcessedFile::Failed {
                kind,
                source_revision_id: Some(source_revision_id),
            }
        }
    };

    Ok(ProcessedImportFile { file, processed })
}

pub(crate) fn parallel_parse_error() -> ImportPipelineError {
    ImportPipelineError {
        kind: ImportPipelineErrorKind::Parser,
        retryable: true,
    }
}
