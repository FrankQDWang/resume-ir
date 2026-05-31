pub fn crate_name() -> &'static str {
    "ocr-client"
}

use std::fmt;

pub trait OcrClient {
    fn recognize_page(
        &self,
        request: OcrPageRequest,
        budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> Result<OcrPage, OcrError>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisabledOcrWorkerClient;

impl OcrClient for DisabledOcrWorkerClient {
    fn recognize_page(
        &self,
        _request: OcrPageRequest,
        _budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> Result<OcrPage, OcrError> {
        if cancellation.is_cancelled() {
            return Err(OcrError::new(OcrErrorKind::Cancelled));
        }

        Err(OcrError::new(OcrErrorKind::Disabled))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OcrCacheKey {
    file_content_hash: String,
    page_no: u32,
    render_dpi: u32,
    ocr_lang: String,
    ocr_profile: String,
}

impl OcrCacheKey {
    pub fn new(
        file_content_hash: impl Into<String>,
        page_no: u32,
        render_dpi: u32,
        ocr_lang: impl Into<String>,
        ocr_profile: impl Into<String>,
    ) -> Result<Self, OcrError> {
        let file_content_hash = file_content_hash.into();
        let ocr_lang = ocr_lang.into();
        let ocr_profile = ocr_profile.into();
        if file_content_hash.trim().is_empty()
            || page_no == 0
            || render_dpi == 0
            || ocr_lang.trim().is_empty()
            || ocr_profile.trim().is_empty()
        {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            file_content_hash,
            page_no,
            render_dpi,
            ocr_lang,
            ocr_profile,
        })
    }

    pub fn page_no(&self) -> u32 {
        self.page_no
    }

    pub fn render_dpi(&self) -> u32 {
        self.render_dpi
    }

    pub fn ocr_lang(&self) -> &str {
        &self.ocr_lang
    }

    pub fn ocr_profile(&self) -> &str {
        &self.ocr_profile
    }
}

impl fmt::Debug for OcrCacheKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrCacheKey")
            .field("file_content_hash", &"<redacted>")
            .field("page_no", &self.page_no)
            .field("render_dpi", &self.render_dpi)
            .field("ocr_lang", &self.ocr_lang)
            .field("ocr_profile", &self.ocr_profile)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct OcrPage {
    page_no: u32,
    text: String,
    confidence: f32,
    engine_profile: String,
    duration_ms: u64,
}

impl OcrPage {
    pub fn new(
        page_no: u32,
        text: impl Into<String>,
        confidence: f32,
        engine_profile: impl Into<String>,
        duration_ms: u64,
    ) -> Result<Self, OcrError> {
        if page_no == 0 || !confidence.is_finite() {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            page_no,
            text: text.into(),
            confidence: confidence.clamp(0.0, 1.0),
            engine_profile: engine_profile.into(),
            duration_ms,
        })
    }

    pub fn page_no(&self) -> u32 {
        self.page_no
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn confidence(&self) -> f32 {
        self.confidence
    }

    pub fn duration_ms(&self) -> u64 {
        self.duration_ms
    }
}

impl fmt::Debug for OcrPage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrPage")
            .field("page_no", &self.page_no)
            .field("text", &"<redacted>")
            .field("text_bytes", &self.text.len())
            .field("confidence", &self.confidence)
            .field("engine_profile", &self.engine_profile)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RenderedPage {
    page_no: u32,
    render_dpi: u32,
    bytes: Vec<u8>,
}

impl RenderedPage {
    pub fn new(page_no: u32, render_dpi: u32, bytes: Vec<u8>) -> Result<Self, OcrError> {
        if page_no == 0 || render_dpi == 0 || bytes.is_empty() {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            page_no,
            render_dpi,
            bytes,
        })
    }

    pub fn page_no(&self) -> u32 {
        self.page_no
    }

    pub fn render_dpi(&self) -> u32 {
        self.render_dpi
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for RenderedPage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RenderedPage")
            .field("page_no", &self.page_no)
            .field("render_dpi", &self.render_dpi)
            .field("bytes", &"<redacted>")
            .field("byte_len", &self.bytes.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrOptions {
    lang: String,
    profile: String,
}

impl OcrOptions {
    pub fn new(lang: impl Into<String>, profile: impl Into<String>) -> Result<Self, OcrError> {
        let lang = lang.into();
        let profile = profile.into();
        if lang.trim().is_empty() || profile.trim().is_empty() {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self { lang, profile })
    }

    pub fn lang(&self) -> &str {
        &self.lang
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrPageRequest {
    page: RenderedPage,
    options: OcrOptions,
}

impl OcrPageRequest {
    pub fn new(page: RenderedPage, options: OcrOptions) -> Result<Self, OcrError> {
        Ok(Self { page, options })
    }

    pub fn page(&self) -> &RenderedPage {
        &self.page
    }

    pub fn options(&self) -> &OcrOptions {
        &self.options
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OcrWorkerBudget {
    page_timeout_ms: u64,
}

impl OcrWorkerBudget {
    pub fn new(page_timeout_ms: u64) -> Result<Self, OcrError> {
        if page_timeout_ms == 0 {
            return Err(OcrError::new(OcrErrorKind::InvalidRequest));
        }

        Ok(Self { page_timeout_ms })
    }

    pub fn page_timeout_ms(self) -> u64 {
        self.page_timeout_ms
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CancellationToken {
    cancelled: bool,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self { cancelled: false }
    }

    pub fn new_cancelled() -> Self {
        Self { cancelled: true }
    }

    pub fn is_cancelled(self) -> bool {
        self.cancelled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrErrorKind {
    Disabled,
    Cancelled,
    Timeout,
    InvalidRequest,
    WorkerUnavailable,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OcrError {
    kind: OcrErrorKind,
}

impl OcrError {
    pub fn new(kind: OcrErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(&self) -> OcrErrorKind {
        self.kind
    }
}

impl fmt::Debug for OcrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrError")
            .field("kind", &self.kind)
            .finish()
    }
}

impl fmt::Display for OcrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            OcrErrorKind::Disabled => formatter.write_str("OCR worker is disabled"),
            OcrErrorKind::Cancelled => formatter.write_str("OCR request was cancelled"),
            OcrErrorKind::Timeout => formatter.write_str("OCR request timed out"),
            OcrErrorKind::InvalidRequest => formatter.write_str("OCR request is invalid"),
            OcrErrorKind::WorkerUnavailable => formatter.write_str("OCR worker is unavailable"),
        }
    }
}

impl std::error::Error for OcrError {}
