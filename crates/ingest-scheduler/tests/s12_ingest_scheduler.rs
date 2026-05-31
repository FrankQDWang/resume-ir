use core_domain::{DocumentId, DocumentStatus};
use ingest_scheduler::{
    OcrScheduleDecision, OcrScheduler, OcrSchedulingInput, OcrSchedulingPolicy,
};
use ocr_client::OcrOptions;

#[test]
fn exposes_ingest_scheduler_crate_identity() {
    assert_eq!(ingest_scheduler::crate_name(), "ingest-scheduler");
}

#[test]
fn default_scheduler_keeps_ocr_required_documents_queued_without_running_ocr() {
    let input = ocr_input(DocumentStatus::OcrRequired, 4);
    let scheduler = OcrScheduler::default();

    let plan = scheduler
        .plan_ocr(&input, &OcrOptions::new("eng", "economy").unwrap())
        .unwrap();

    assert_eq!(plan.decision(), OcrScheduleDecision::OcrDisabled);
    assert!(plan.queue_items().is_empty());
    assert_eq!(input.status(), DocumentStatus::OcrRequired);
}

#[test]
fn enabled_scheduler_limits_pages_and_builds_redacted_cache_keys() {
    let input = ocr_input(DocumentStatus::OcrRequired, 5);
    let scheduler = OcrScheduler::new(OcrSchedulingPolicy::enabled(2, 750).unwrap());

    let plan = scheduler
        .plan_ocr(&input, &OcrOptions::new("eng+chi_sim", "balanced").unwrap())
        .unwrap();

    assert_eq!(plan.decision(), OcrScheduleDecision::Scheduled);
    assert_eq!(plan.queue_items().len(), 2);
    assert_eq!(plan.queue_items()[0].page_no(), 1);
    assert_eq!(plan.queue_items()[1].page_no(), 2);
    assert_eq!(plan.queue_items()[0].page_timeout_ms(), 750);
    assert_eq!(plan.queue_items()[0].cache_key().page_no(), 1);
    assert!(!format!("{:?}", plan.queue_items()[0]).contains("synthetic-content-hash"));
}

#[test]
fn searchable_documents_do_not_enter_ocr_queue() {
    let input = ocr_input(DocumentStatus::Searchable, 2);
    let scheduler = OcrScheduler::new(OcrSchedulingPolicy::enabled(10, 750).unwrap());

    let plan = scheduler
        .plan_ocr(&input, &OcrOptions::new("eng", "balanced").unwrap())
        .unwrap();

    assert_eq!(plan.decision(), OcrScheduleDecision::NotRequired);
    assert!(plan.queue_items().is_empty());
}

fn ocr_input(status: DocumentStatus, page_count: u32) -> OcrSchedulingInput {
    OcrSchedulingInput::new(
        DocumentId::from_non_secret_parts(&["s12", "synthetic-scanned"]),
        status,
        "synthetic-content-hash",
        page_count,
        300,
    )
    .unwrap()
}
