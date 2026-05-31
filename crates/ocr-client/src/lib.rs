//! OCR worker client interfaces for the S12 skeleton.

use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use thiserror::Error;

/// Options that affect OCR output and page-cache identity.
#[derive(Clone, Eq, PartialEq)]
pub struct OcrOptions {
    languages: Vec<String>,
    profile: String,
}

impl OcrOptions {
    /// Creates normalized OCR options from language tags and a profile label.
    pub fn new<I, S>(languages: I, profile: impl Into<String>) -> Result<Self, OcrClientError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut normalized_languages = Vec::new();
        for language in languages {
            normalized_languages.push(normalize_language(language.into())?);
        }
        if normalized_languages.is_empty() {
            return Err(OcrClientError::EmptyLanguageList);
        }
        normalized_languages.sort();
        normalized_languages.dedup();

        Ok(Self {
            languages: normalized_languages,
            profile: normalize_profile(profile.into())?,
        })
    }

    /// Returns normalized language tags in deterministic order.
    #[must_use]
    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    /// Returns the normalized OCR profile label.
    #[must_use]
    pub fn profile(&self) -> &str {
        &self.profile
    }
}

impl fmt::Debug for OcrOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrOptions")
            .field("languages", &self.languages)
            .field("profile", &self.profile)
            .finish()
    }
}

/// Deterministic key for one rendered document page and OCR profile.
#[derive(Clone, Eq, PartialEq)]
pub struct OcrCacheKey {
    content_hash: String,
    page_number: u32,
    render_dpi: u16,
    languages: Vec<String>,
    profile: String,
    serialized: String,
}

impl OcrCacheKey {
    /// Creates a page-level OCR cache key from a SHA-256 content hash.
    pub fn new(
        content_hash: impl Into<String>,
        page_number: u32,
        render_dpi: u16,
        options: &OcrOptions,
    ) -> Result<Self, OcrClientError> {
        if page_number == 0 {
            return Err(OcrClientError::InvalidPageNumber);
        }
        if render_dpi == 0 {
            return Err(OcrClientError::InvalidRenderDpi);
        }

        let content_hash = normalize_content_hash(content_hash.into())?;
        let language_key = options.languages.join("+");
        let profile = options.profile.clone();
        let serialized = format!(
            "v1|hash={content_hash}|page={page_number}|dpi={render_dpi}|lang={language_key}|profile={profile}"
        );

        Ok(Self {
            content_hash,
            page_number,
            render_dpi,
            languages: options.languages.clone(),
            profile,
            serialized,
        })
    }

    /// Returns the deterministic cache key string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.serialized
    }

    /// Returns the canonical content hash used by the key.
    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    /// Returns the one-based page number.
    #[must_use]
    pub fn page_number(&self) -> u32 {
        self.page_number
    }

    /// Returns the render DPI used for the OCR page image.
    #[must_use]
    pub fn render_dpi(&self) -> u16 {
        self.render_dpi
    }

    /// Returns normalized language tags in cache-key order.
    #[must_use]
    pub fn languages(&self) -> &[String] {
        &self.languages
    }

    /// Returns the OCR profile label.
    #[must_use]
    pub fn profile(&self) -> &str {
        &self.profile
    }
}

impl fmt::Debug for OcrCacheKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrCacheKey")
            .field("content_hash", &"[redacted content hash]")
            .field("page_number", &self.page_number)
            .field("render_dpi", &self.render_dpi)
            .field("languages", &self.languages)
            .field("profile", &self.profile)
            .finish()
    }
}

/// Page-level OCR timeout budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageOcrTimeout {
    budget: Duration,
}

impl PageOcrTimeout {
    /// Creates a timeout with no remaining budget.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            budget: Duration::ZERO,
        }
    }

    /// Creates a timeout from a remaining page budget.
    #[must_use]
    pub fn from_budget(budget: Duration) -> Self {
        Self { budget }
    }

    /// Returns the configured page budget.
    #[must_use]
    pub fn budget(&self) -> Duration {
        self.budget
    }

    /// Returns whether the page has any remaining OCR budget.
    #[must_use]
    pub fn has_remaining(&self) -> bool {
        !self.budget.is_zero()
    }
}

/// Shareable cancellation flag for page OCR requests.
#[derive(Clone)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a token in the active state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Marks the token as cancelled.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// One rendered page request sent to an OCR worker boundary.
#[derive(Clone)]
pub struct OcrPageRequest {
    cache_key: OcrCacheKey,
    page_bytes: Vec<u8>,
    options: OcrOptions,
    timeout: PageOcrTimeout,
    cancellation: CancellationToken,
}

impl OcrPageRequest {
    /// Creates a page OCR request from already-rendered page bytes.
    #[must_use]
    pub fn new(
        cache_key: OcrCacheKey,
        page_bytes: Vec<u8>,
        options: OcrOptions,
        timeout: PageOcrTimeout,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            cache_key,
            page_bytes,
            options,
            timeout,
            cancellation,
        }
    }

    /// Returns the OCR cache key.
    #[must_use]
    pub fn cache_key(&self) -> &OcrCacheKey {
        &self.cache_key
    }

    /// Returns rendered page bytes for worker implementations.
    #[must_use]
    pub fn page_bytes(&self) -> &[u8] {
        &self.page_bytes
    }

    /// Returns OCR options.
    #[must_use]
    pub fn options(&self) -> &OcrOptions {
        &self.options
    }

    /// Returns the page timeout budget.
    #[must_use]
    pub fn timeout(&self) -> PageOcrTimeout {
        self.timeout
    }

    /// Returns the cancellation token.
    #[must_use]
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

impl fmt::Debug for OcrPageRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrPageRequest")
            .field("cache_key", &"[redacted OCR cache key]")
            .field("page_number", &self.cache_key.page_number)
            .field("render_dpi", &self.cache_key.render_dpi)
            .field("byte_len", &self.page_bytes.len())
            .field("options", &self.options)
            .field("timeout", &self.timeout)
            .field("cancellation", &self.cancellation)
            .finish()
    }
}

/// Reason a page OCR request was deferred instead of executed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrDeferredReason {
    /// OCR execution is disabled in this skeleton client.
    ClientDisabled,
}

/// Page OCR execution status.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrPageStatus {
    /// OCR completed and text is present.
    Completed,
    /// OCR was intentionally deferred.
    Deferred(OcrDeferredReason),
    /// OCR was skipped because cancellation was requested.
    Cancelled,
    /// OCR was skipped because the page budget was exhausted.
    TimedOut,
}

/// OCR output for one page.
#[derive(Clone, Eq, PartialEq)]
pub struct OcrPageOutput {
    cache_key: OcrCacheKey,
    status: OcrPageStatus,
    text: Option<String>,
}

impl OcrPageOutput {
    /// Creates completed OCR output with extracted text.
    #[must_use]
    pub fn completed(cache_key: OcrCacheKey, text: impl Into<String>) -> Self {
        Self {
            cache_key,
            status: OcrPageStatus::Completed,
            text: Some(text.into()),
        }
    }

    /// Creates deferred OCR output without text.
    #[must_use]
    pub fn deferred(cache_key: OcrCacheKey, reason: OcrDeferredReason) -> Self {
        Self::without_text(cache_key, OcrPageStatus::Deferred(reason))
    }

    /// Creates cancelled OCR output without text.
    #[must_use]
    pub fn cancelled(cache_key: OcrCacheKey) -> Self {
        Self::without_text(cache_key, OcrPageStatus::Cancelled)
    }

    /// Creates timed-out OCR output without text.
    #[must_use]
    pub fn timed_out(cache_key: OcrCacheKey) -> Self {
        Self::without_text(cache_key, OcrPageStatus::TimedOut)
    }

    /// Returns the cache key for this page.
    #[must_use]
    pub fn cache_key(&self) -> &OcrCacheKey {
        &self.cache_key
    }

    /// Returns the page status.
    #[must_use]
    pub fn status(&self) -> &OcrPageStatus {
        &self.status
    }

    /// Returns OCR text when a real worker completed.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Returns whether OCR execution actually completed.
    #[must_use]
    pub fn ocr_executed(&self) -> bool {
        matches!(self.status, OcrPageStatus::Completed)
    }

    fn without_text(cache_key: OcrCacheKey, status: OcrPageStatus) -> Self {
        Self {
            cache_key,
            status,
            text: None,
        }
    }
}

impl fmt::Debug for OcrPageOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrPageOutput")
            .field("cache_key", &"[redacted OCR cache key]")
            .field("page_number", &self.cache_key.page_number)
            .field("status", &self.status)
            .field("text_present", &self.text.is_some())
            .finish()
    }
}

/// Replaceable OCR worker client boundary.
pub trait OcrWorkerClient {
    /// Recognizes text from one rendered page or returns a typed non-execution status.
    fn recognize_page(&self, request: &OcrPageRequest) -> Result<OcrPageOutput, OcrClientError>;
}

/// OCR client used when heavy OCR execution is disabled.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DisabledOcrWorkerClient;

impl OcrWorkerClient for DisabledOcrWorkerClient {
    fn recognize_page(&self, request: &OcrPageRequest) -> Result<OcrPageOutput, OcrClientError> {
        if request.cancellation().is_cancelled() {
            return Ok(OcrPageOutput::cancelled(request.cache_key().clone()));
        }
        if !request.timeout().has_remaining() {
            return Ok(OcrPageOutput::timed_out(request.cache_key().clone()));
        }

        Ok(OcrPageOutput::deferred(
            request.cache_key().clone(),
            OcrDeferredReason::ClientDisabled,
        ))
    }
}

/// Errors returned while constructing OCR request and cache-key types.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum OcrClientError {
    /// At least one OCR language must be supplied.
    #[error("OCR language list must contain at least one language")]
    EmptyLanguageList,
    /// OCR language tags must not be empty.
    #[error("OCR language tag must not be empty")]
    EmptyLanguage,
    /// OCR language tags must be stable cache-key tokens.
    #[error("OCR language tag contains unsupported characters")]
    InvalidLanguage,
    /// OCR profile labels must not be empty.
    #[error("OCR profile must not be empty")]
    EmptyProfile,
    /// OCR profile labels must be stable cache-key tokens.
    #[error("OCR profile contains unsupported characters")]
    InvalidProfile,
    /// Content hashes must be canonical SHA-256 values, not paths or text.
    #[error("OCR content hash must be a SHA-256 hex value")]
    InvalidContentHash,
    /// Page numbers are one-based.
    #[error("OCR page number must be greater than zero")]
    InvalidPageNumber,
    /// Render DPI must be greater than zero.
    #[error("OCR render dpi must be greater than zero")]
    InvalidRenderDpi,
}

fn normalize_language(language: String) -> Result<String, OcrClientError> {
    let normalized = language.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(OcrClientError::EmptyLanguage);
    }
    if !is_stable_token(&normalized) {
        return Err(OcrClientError::InvalidLanguage);
    }
    Ok(normalized)
}

fn normalize_profile(profile: String) -> Result<String, OcrClientError> {
    let normalized = profile.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(OcrClientError::EmptyProfile);
    }
    if !is_stable_token(&normalized) {
        return Err(OcrClientError::InvalidProfile);
    }
    Ok(normalized)
}

fn normalize_content_hash(content_hash: String) -> Result<String, OcrClientError> {
    let normalized = content_hash.trim().to_ascii_lowercase();
    let hex = if let Some(stripped) = normalized.strip_prefix("sha256:") {
        stripped
    } else {
        normalized.as_str()
    };

    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(OcrClientError::InvalidContentHash);
    }

    Ok(format!("sha256:{hex}"))
}

fn is_stable_token(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
