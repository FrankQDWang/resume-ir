use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use meta_store::SearchProjectionFilter;

pub(crate) struct DaemonSearchArgs {
    pub(crate) query: String,
    pub(crate) mode: DaemonSearchMode,
    pub(crate) top_k: usize,
    pub(crate) filter: SearchProjectionFilter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DaemonSearchMode {
    FullText,
    Semantic,
    Hybrid,
}

impl DaemonSearchMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "fulltext" | "keyword" => Some(Self::FullText),
            "semantic" => Some(Self::Semantic),
            "hybrid" => Some(Self::Hybrid),
            _ => None,
        }
    }

    pub(crate) fn response_label(self) -> &'static str {
        match self {
            Self::FullText => "keyword",
            Self::Semantic => "semantic",
            Self::Hybrid => "hybrid",
        }
    }
}

pub(crate) struct SearchCancellation {
    requested: AtomicBool,
}

impl SearchCancellation {
    pub(crate) fn new() -> Self {
        Self {
            requested: AtomicBool::new(false),
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.requested.load(Ordering::Acquire)
    }

    pub(crate) fn request(&self) {
        self.requested.store(true, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
pub(crate) struct SearchDeadline {
    started_at: Instant,
    expires_at: Instant,
}

impl SearchDeadline {
    pub(crate) fn new(started_at: Instant, deadline_ms: u64) -> Self {
        Self {
            started_at,
            expires_at: started_at + Duration::from_millis(deadline_ms),
        }
    }

    pub(crate) fn expired(&self) -> bool {
        Instant::now() >= self.expires_at
    }

    pub(crate) fn expires_at(&self) -> Instant {
        self.expires_at
    }

    pub(crate) fn remaining_ms(&self) -> Option<u64> {
        let remaining = self.expires_at.checked_duration_since(Instant::now())?;
        u64::try_from(remaining.as_millis().max(1)).ok()
    }

    pub(crate) fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

pub(crate) fn redact_search_file_name(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = privacy::redact_contact_values(&compact);
    truncate_utf8_bytes(&redacted, crate::SEARCH_RESULT_FILE_NAME_MAX_BYTES)
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    const ELLIPSIS: &str = "...";
    let mut end = max_bytes.saturating_sub(ELLIPSIS.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], ELLIPSIS)
}
