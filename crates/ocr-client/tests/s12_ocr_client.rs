use ocr_client::{
    CancellationToken, DisabledOcrWorkerClient, OcrCacheKey, OcrClient, OcrErrorKind, OcrOptions,
    OcrPage, OcrPageRequest, OcrWorkerBudget, RenderedPage,
};

#[test]
fn exposes_ocr_client_crate_identity() {
    assert_eq!(ocr_client::crate_name(), "ocr-client");
}

#[test]
fn cache_key_and_page_result_keep_ocr_payloads_out_of_debug() {
    let cache_key =
        OcrCacheKey::new("0123456789abcdef", 3, 300, "eng+chi_sim", "balanced").unwrap();
    assert_eq!(cache_key.page_no(), 3);
    assert!(!format!("{cache_key:?}").contains("0123456789abcdef"));

    let page = OcrPage::new(3, "Synthetic OCR text", 0.88, "balanced", 42).unwrap();
    assert_eq!(page.page_no(), 3);
    assert_eq!(page.text(), "Synthetic OCR text");
    assert!(!format!("{page:?}").contains("Synthetic OCR text"));
}

#[test]
fn rejects_invalid_page_and_timeout_inputs() {
    assert!(OcrCacheKey::new("content", 0, 300, "eng", "balanced").is_err());
    assert!(RenderedPage::new(0, 300, b"bytes".to_vec()).is_err());
    assert!(OcrWorkerBudget::new(0).is_err());
}

#[test]
fn disabled_worker_never_runs_heavy_ocr_and_honors_cancellation() {
    let client = DisabledOcrWorkerClient;
    let request = OcrPageRequest::new(
        RenderedPage::new(1, 300, b"SYNTHETIC IMAGE BYTES".to_vec()).unwrap(),
        OcrOptions::new("eng", "economy").unwrap(),
    )
    .unwrap();

    let cancelled = client
        .recognize_page(
            request.clone(),
            OcrWorkerBudget::new(500).unwrap(),
            &CancellationToken::new_cancelled(),
        )
        .unwrap_err();
    assert_eq!(cancelled.kind(), OcrErrorKind::Cancelled);
    assert!(!format!("{cancelled:?}").contains("SYNTHETIC"));

    let disabled = client
        .recognize_page(
            request,
            OcrWorkerBudget::new(500).unwrap(),
            &CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(disabled.kind(), OcrErrorKind::Disabled);
}
