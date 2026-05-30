use ocr_client::{NoopOcrClient, OcrCacheKey, OcrClient, OcrPageRequest, OcrStatus};
use std::time::Duration;

#[test]
fn noop_client_marks_pages_skipped_without_running_ocr() {
    let client = NoopOcrClient;
    let request = OcrPageRequest {
        cache_key: OcrCacheKey::new("doc_scan", 1, "page_hash"),
        image_bytes: vec![1, 2, 3],
        timeout: Duration::from_secs(2),
        cancel_requested: false,
    };

    let output = client.recognize_page(request).expect("noop output");

    assert_eq!(output.status, OcrStatus::Skipped);
    assert!(output.text.is_none());
    assert_eq!(output.cache_key.page_no, 1);
}

#[test]
fn client_request_supports_timeout_and_cancellation_states() {
    let client = NoopOcrClient;

    let timeout = client
        .recognize_page(OcrPageRequest {
            cache_key: OcrCacheKey::new("doc_scan", 2, "timeout_hash"),
            image_bytes: Vec::new(),
            timeout: Duration::ZERO,
            cancel_requested: false,
        })
        .expect("timeout output");
    let cancelled = client
        .recognize_page(OcrPageRequest {
            cache_key: OcrCacheKey::new("doc_scan", 3, "cancel_hash"),
            image_bytes: Vec::new(),
            timeout: Duration::from_secs(2),
            cancel_requested: true,
        })
        .expect("cancel output");

    assert_eq!(timeout.status, OcrStatus::Timeout);
    assert_eq!(cancelled.status, OcrStatus::Cancelled);
}
