use ocr_client::OcrCacheKey;
use parser_common::ParseStatus;
use std::collections::{HashSet, VecDeque};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestVisibility {
    Searchable,
    Partial,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageImage {
    pub page_no: u32,
    pub image_hash: String,
}

impl PageImage {
    #[must_use]
    pub fn new(page_no: u32, image_hash: impl Into<String>) -> Self {
        Self {
            page_no,
            image_hash: image_hash.into(),
        }
    }

    #[must_use]
    pub fn cache_key(&self, doc_id: impl Into<String>) -> OcrCacheKey {
        OcrCacheKey::new(doc_id, self.page_no, self.image_hash.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OcrQueueJob {
    pub cache_key: OcrCacheKey,
    pub timeout: Duration,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IngestScheduler {
    ocr_queue: VecDeque<OcrQueueJob>,
    cancelled: HashSet<OcrCacheKey>,
}

impl IngestScheduler {
    pub fn record_parse_result(
        &mut self,
        doc_id: impl Into<String>,
        status: ParseStatus,
        page_images: Vec<PageImage>,
        page_timeout: Duration,
    ) -> IngestVisibility {
        let doc_id = doc_id.into();
        match status {
            ParseStatus::Parsed => IngestVisibility::Searchable,
            ParseStatus::OcrRequired => {
                for page in page_images {
                    self.ocr_queue.push_back(OcrQueueJob {
                        cache_key: page.cache_key(&doc_id),
                        timeout: page_timeout,
                    });
                }
                IngestVisibility::Partial
            }
        }
    }

    #[must_use]
    pub fn ocr_queue_len(&self) -> usize {
        self.ocr_queue.len()
    }

    pub fn next_ocr_job(&mut self) -> Option<OcrQueueJob> {
        while let Some(job) = self.ocr_queue.pop_front() {
            if !self.cancelled.remove(&job.cache_key) {
                return Some(job);
            }
        }
        None
    }

    pub fn cancel_ocr_job(&mut self, cache_key: OcrCacheKey) {
        self.cancelled.insert(cache_key);
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "ingest-scheduler"
}
