pub fn crate_name() -> &'static str {
    "ingest-scheduler"
}

use std::fmt;

use core_domain::{DocumentId, DocumentStatus};
use ocr_client::{OcrCacheKey, OcrError, OcrOptions};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrScheduler {
    policy: OcrSchedulingPolicy,
}

impl OcrScheduler {
    pub fn new(policy: OcrSchedulingPolicy) -> Self {
        Self { policy }
    }

    pub fn plan_ocr(
        &self,
        input: &OcrSchedulingInput,
        options: &OcrOptions,
    ) -> Result<OcrSchedulePlan, OcrError> {
        if input.status != DocumentStatus::OcrRequired {
            return Ok(OcrSchedulePlan {
                decision: OcrScheduleDecision::NotRequired,
                queue_items: Vec::new(),
            });
        }

        if !self.policy.enabled {
            return Ok(OcrSchedulePlan {
                decision: OcrScheduleDecision::OcrDisabled,
                queue_items: Vec::new(),
            });
        }

        let queue_items = (1..=input.page_count)
            .take(self.policy.max_queued_pages)
            .map(|page_no| {
                Ok(OcrQueueItem::new(
                    input.document_id.clone(),
                    OcrCacheKey::new(
                        input.content_hash.clone(),
                        page_no,
                        input.render_dpi,
                        options.lang(),
                        options.profile(),
                    )?,
                    page_no,
                    self.policy.page_timeout_ms,
                ))
            })
            .collect::<Result<Vec<_>, OcrError>>()?;

        Ok(OcrSchedulePlan {
            decision: OcrScheduleDecision::Scheduled,
            queue_items,
        })
    }
}

impl Default for OcrScheduler {
    fn default() -> Self {
        Self::new(OcrSchedulingPolicy::disabled())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrSchedulingPolicy {
    enabled: bool,
    max_queued_pages: usize,
    page_timeout_ms: u64,
}

impl OcrSchedulingPolicy {
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            max_queued_pages: 0,
            page_timeout_ms: 1,
        }
    }

    pub fn enabled(max_queued_pages: usize, page_timeout_ms: u64) -> Result<Self, OcrError> {
        if max_queued_pages == 0 || page_timeout_ms == 0 {
            return Err(OcrError::new(ocr_client::OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            enabled: true,
            max_queued_pages,
            page_timeout_ms,
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OcrSchedulingInput {
    document_id: DocumentId,
    status: DocumentStatus,
    content_hash: String,
    page_count: u32,
    render_dpi: u32,
}

impl OcrSchedulingInput {
    pub fn new(
        document_id: DocumentId,
        status: DocumentStatus,
        content_hash: impl Into<String>,
        page_count: u32,
        render_dpi: u32,
    ) -> Result<Self, OcrError> {
        let content_hash = content_hash.into();
        if content_hash.trim().is_empty() || page_count == 0 || render_dpi == 0 {
            return Err(OcrError::new(ocr_client::OcrErrorKind::InvalidRequest));
        }

        Ok(Self {
            document_id,
            status,
            content_hash,
            page_count,
            render_dpi,
        })
    }

    pub fn status(&self) -> DocumentStatus {
        self.status
    }
}

impl fmt::Debug for OcrSchedulingInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrSchedulingInput")
            .field("document_id", &self.document_id)
            .field("status", &self.status)
            .field("content_hash", &"<redacted>")
            .field("page_count", &self.page_count)
            .field("render_dpi", &self.render_dpi)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrScheduleDecision {
    OcrDisabled,
    NotRequired,
    Scheduled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcrSchedulePlan {
    decision: OcrScheduleDecision,
    queue_items: Vec<OcrQueueItem>,
}

impl OcrSchedulePlan {
    pub fn decision(&self) -> OcrScheduleDecision {
        self.decision
    }

    pub fn queue_items(&self) -> &[OcrQueueItem] {
        &self.queue_items
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OcrQueueItem {
    document_id: DocumentId,
    cache_key: OcrCacheKey,
    page_no: u32,
    page_timeout_ms: u64,
}

impl OcrQueueItem {
    fn new(
        document_id: DocumentId,
        cache_key: OcrCacheKey,
        page_no: u32,
        page_timeout_ms: u64,
    ) -> Self {
        Self {
            document_id,
            cache_key,
            page_no,
            page_timeout_ms,
        }
    }

    pub fn page_no(&self) -> u32 {
        self.page_no
    }

    pub fn document_id(&self) -> &DocumentId {
        &self.document_id
    }

    pub fn cache_key(&self) -> &OcrCacheKey {
        &self.cache_key
    }

    pub fn page_timeout_ms(&self) -> u64 {
        self.page_timeout_ms
    }
}

impl fmt::Debug for OcrQueueItem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrQueueItem")
            .field("document_id", &self.document_id)
            .field("cache_key", &self.cache_key)
            .field("page_no", &self.page_no)
            .field("page_timeout_ms", &self.page_timeout_ms)
            .finish()
    }
}
