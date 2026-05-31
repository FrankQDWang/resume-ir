//! OCR client contract tests for the S12 skeleton.

use ocr_client::{
    CancellationToken, DisabledOcrWorkerClient, OcrCacheKey, OcrDeferredReason, OcrOptions,
    OcrPageRequest, OcrPageStatus, OcrWorkerClient, PageOcrTimeout,
};
use std::error::Error;
use std::time::Duration;

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SENSITIVE_PAGE_BYTES: &str = "SYNTHETIC_SCANNED_BYTES_SHOULD_NOT_APPEAR";
const REDACTED_RAW_PATH: &str = "/local/redacted/resume.pdf";

#[test]
fn disabled_client_defers_without_fake_text_and_redacts_sensitive_request_data(
) -> Result<(), Box<dyn Error>> {
    let options = OcrOptions::new(["eng", "chi_sim"], "resume-fast")?;
    let cache_key = OcrCacheKey::new(HASH, 2, 220, &options)?;
    let request = OcrPageRequest::new(
        cache_key.clone(),
        SENSITIVE_PAGE_BYTES.as_bytes().to_vec(),
        options,
        PageOcrTimeout::from_budget(Duration::from_secs(3)),
        CancellationToken::new(),
    );

    let output = DisabledOcrWorkerClient.recognize_page(&request)?;

    assert_eq!(output.cache_key(), &cache_key);
    assert_eq!(
        output.status(),
        &OcrPageStatus::Deferred(OcrDeferredReason::ClientDisabled)
    );
    assert!(!output.ocr_executed());
    assert_eq!(output.text(), None);

    let debug = format!("{request:?} {output:?}");
    assert!(debug.contains("byte_len"));
    assert!(!debug.contains(SENSITIVE_PAGE_BYTES));
    assert!(!debug.contains(HASH));
    assert!(!debug.contains("fake"));

    Ok(())
}

#[test]
fn cache_key_is_deterministic_and_rejects_raw_paths_or_text() -> Result<(), Box<dyn Error>> {
    let options = OcrOptions::new(["CHI_SIM", "eng", "eng"], "Resume-Fast")?;
    let bare = OcrCacheKey::new(HASH.to_ascii_uppercase(), 1, 200, &options)?;
    let prefixed = OcrCacheKey::new(format!("sha256:{HASH}"), 1, 200, &options)?;
    let different_dpi = OcrCacheKey::new(HASH, 1, 300, &options)?;
    let different_profile = OcrCacheKey::new(
        HASH,
        1,
        200,
        &OcrOptions::new(["chi_sim", "eng"], "resume-accurate")?,
    )?;

    assert_eq!(options.languages(), ["chi_sim", "eng"]);
    assert_eq!(bare.as_str(), prefixed.as_str());
    assert_ne!(bare.as_str(), different_dpi.as_str());
    assert_ne!(bare.as_str(), different_profile.as_str());
    assert!(!bare.as_str().contains(REDACTED_RAW_PATH));
    assert!(!bare.as_str().contains("synthetic resume text"));
    assert!(OcrCacheKey::new(REDACTED_RAW_PATH, 1, 200, &options).is_err());
    assert!(OcrCacheKey::new("synthetic resume text", 1, 200, &options).is_err());

    Ok(())
}

#[test]
fn disabled_client_honors_cancellation_and_page_timeout_before_defer() -> Result<(), Box<dyn Error>>
{
    let options = OcrOptions::new(["eng"], "resume-fast")?;
    let cache_key = OcrCacheKey::new(HASH, 1, 200, &options)?;
    let client = DisabledOcrWorkerClient;

    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled = client.recognize_page(&OcrPageRequest::new(
        cache_key.clone(),
        Vec::new(),
        options.clone(),
        PageOcrTimeout::from_budget(Duration::from_secs(1)),
        cancellation,
    ))?;
    assert_eq!(cancelled.status(), &OcrPageStatus::Cancelled);
    assert_eq!(cancelled.text(), None);

    let timed_out = client.recognize_page(&OcrPageRequest::new(
        cache_key,
        Vec::new(),
        options,
        PageOcrTimeout::zero(),
        CancellationToken::new(),
    ))?;
    assert_eq!(timed_out.status(), &OcrPageStatus::TimedOut);
    assert_eq!(timed_out.text(), None);

    Ok(())
}
