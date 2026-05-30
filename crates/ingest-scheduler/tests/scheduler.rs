use ingest_scheduler::{IngestScheduler, IngestVisibility, PageImage};
use parser_common::ParseStatus;
use std::time::Duration;

#[test]
fn ocr_required_documents_are_queued_without_running_ocr() {
    let mut scheduler = IngestScheduler::default();

    let visibility = scheduler.record_parse_result(
        "doc_scan",
        ParseStatus::OcrRequired,
        vec![PageImage::new(1, "hash_page_1")],
        Duration::from_secs(5),
    );

    assert_eq!(visibility, IngestVisibility::Partial);
    assert_eq!(scheduler.ocr_queue_len(), 1);
    let job = scheduler.next_ocr_job().expect("queued OCR job");
    assert_eq!(job.cache_key.doc_id, "doc_scan");
    assert_eq!(job.cache_key.page_no, 1);
    assert_eq!(job.timeout, Duration::from_secs(5));
}

#[test]
fn parsed_documents_do_not_enter_ocr_queue() {
    let mut scheduler = IngestScheduler::default();

    let visibility = scheduler.record_parse_result(
        "doc_text",
        ParseStatus::Parsed,
        vec![PageImage::new(1, "hash_page_1")],
        Duration::from_secs(5),
    );

    assert_eq!(visibility, IngestVisibility::Searchable);
    assert_eq!(scheduler.ocr_queue_len(), 0);
}

#[test]
fn cancelled_ocr_jobs_are_skipped_by_scheduler() {
    let mut scheduler = IngestScheduler::default();
    scheduler.record_parse_result(
        "doc_scan",
        ParseStatus::OcrRequired,
        vec![
            PageImage::new(1, "hash_page_1"),
            PageImage::new(2, "hash_page_2"),
        ],
        Duration::from_secs(5),
    );

    scheduler.cancel_ocr_job(PageImage::new(1, "hash_page_1").cache_key("doc_scan"));
    let next = scheduler.next_ocr_job().expect("second job");

    assert_eq!(next.cache_key.page_no, 2);
    assert!(scheduler.next_ocr_job().is_none());
}
