use std::time::Duration;

pub type OcrResult<T> = Result<T, OcrError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OcrError {
    message: String,
}

impl OcrError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct OcrCacheKey {
    pub doc_id: String,
    pub page_no: u32,
    pub image_hash: String,
}

impl OcrCacheKey {
    #[must_use]
    pub fn new(doc_id: impl Into<String>, page_no: u32, image_hash: impl Into<String>) -> Self {
        Self {
            doc_id: doc_id.into(),
            page_no,
            image_hash: image_hash.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OcrStatus {
    Completed,
    Timeout,
    Cancelled,
    Skipped,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OcrPageRequest {
    pub cache_key: OcrCacheKey,
    pub image_bytes: Vec<u8>,
    pub timeout: Duration,
    pub cancel_requested: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OcrPageOutput {
    pub cache_key: OcrCacheKey,
    pub status: OcrStatus,
    pub text: Option<String>,
}

pub trait OcrClient {
    fn recognize_page(&self, request: OcrPageRequest) -> OcrResult<OcrPageOutput>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoopOcrClient;

impl OcrClient for NoopOcrClient {
    fn recognize_page(&self, request: OcrPageRequest) -> OcrResult<OcrPageOutput> {
        let status = if request.cancel_requested {
            OcrStatus::Cancelled
        } else if request.timeout == Duration::ZERO {
            OcrStatus::Timeout
        } else {
            OcrStatus::Skipped
        };
        Ok(OcrPageOutput {
            cache_key: request.cache_key,
            status,
            text: None,
        })
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "ocr-client"
}
