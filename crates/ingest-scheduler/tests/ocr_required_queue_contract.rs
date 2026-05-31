//! OCR-required queue contract tests for the S12 skeleton.

use ingest_scheduler::{
    InMemoryOcrQueue, OcrClaimPolicy, OcrRoutingState, OcrTaskPriority, OcrTaskState, QueueTick,
};
use ocr_client::{OcrCacheKey, OcrOptions};
use std::error::Error;

const HASH: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn cache_key(page_number: u32, render_dpi: u16) -> Result<OcrCacheKey, Box<dyn Error>> {
    let options = OcrOptions::new(["eng"], "resume-fast")?;
    Ok(OcrCacheKey::new(HASH, page_number, render_dpi, &options)?)
}

#[test]
fn ocr_required_queue_claims_by_priority_and_stays_off_query_path() -> Result<(), Box<dyn Error>> {
    let mut queue = InMemoryOcrQueue::new();
    let low =
        queue.enqueue_ocr_required("doc-low-local-id", cache_key(1, 300)?, OcrTaskPriority::Low);
    let high = queue.enqueue_ocr_required(
        "doc-high-local-id",
        cache_key(2, 300)?,
        OcrTaskPriority::High,
    );

    assert_eq!(queue.routing_state(low), Some(OcrRoutingState::OcrRequired));
    assert_eq!(queue.pending_len(), 2);
    assert!(queue.claim_next(&OcrClaimPolicy::query_path()).is_none());
    assert_eq!(queue.task_state(high), Some(OcrTaskState::Queued));

    let Some(claimed) = queue.claim_next(&OcrClaimPolicy::background(300)) else {
        return Err("expected high priority OCR task to be claimed".into());
    };

    assert_eq!(claimed.task_id(), high);
    assert_eq!(claimed.doc_id(), "doc-high-local-id");
    assert_eq!(claimed.priority(), OcrTaskPriority::High);
    assert_eq!(claimed.attempts(), 1);
    assert_eq!(queue.task_state(high), Some(OcrTaskState::Running));

    let debug = format!("{claimed:?} {queue:?}");
    assert!(!debug.contains("doc-high-local-id"));
    assert!(!debug.contains(HASH));

    Ok(())
}

#[test]
fn queue_defers_retries_and_cancels_without_parsing_or_ocr() -> Result<(), Box<dyn Error>> {
    let mut queue = InMemoryOcrQueue::new();
    let task_id = queue.enqueue_ocr_required(
        "doc-retry-local-id",
        cache_key(1, 200)?,
        OcrTaskPriority::Normal,
    );

    let Some(first_claim) = queue.claim_next(&OcrClaimPolicy::background(300)) else {
        return Err("expected queued OCR task to be claimed".into());
    };
    assert_eq!(first_claim.attempts(), 1);
    assert!(queue.defer(first_claim.task_id(), QueueTick::new(5)));
    assert_eq!(
        queue.task_state(task_id),
        Some(OcrTaskState::Deferred {
            retry_after: QueueTick::new(5)
        })
    );

    assert_eq!(queue.release_ready_deferred(QueueTick::new(4)), 0);
    assert!(queue.claim_next(&OcrClaimPolicy::background(300)).is_none());
    assert_eq!(queue.release_ready_deferred(QueueTick::new(5)), 1);

    let Some(second_claim) = queue.claim_next(&OcrClaimPolicy::background(300)) else {
        return Err("expected deferred OCR task to be retryable".into());
    };
    assert_eq!(second_claim.task_id(), task_id);
    assert_eq!(second_claim.attempts(), 2);

    assert!(queue.cancel(task_id));
    assert_eq!(queue.task_state(task_id), Some(OcrTaskState::Cancelled));
    assert!(queue.claim_next(&OcrClaimPolicy::background(300)).is_none());

    Ok(())
}
