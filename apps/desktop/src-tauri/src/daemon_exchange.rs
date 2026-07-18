use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::daemon_client::DesktopError;
use crate::daemon_request::Operation;

pub(crate) const MAX_REQUEST_BYTES: usize = 64 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Exact immutable search identity carried from a search hit into subsequent
/// detail requests. The bridge never reconstructs this identity from a
/// document id.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchSelection {
    doc_id: String,
    version_id: String,
    visible_epoch: u64,
}

impl SearchSelection {
    pub(crate) fn is_valid(&self) -> bool {
        valid_stable_id(&self.doc_id, "doc_")
            && valid_stable_id(&self.version_id, "ver_")
            && (1..=MAX_SAFE_INTEGER).contains(&self.visible_epoch)
    }

    pub(crate) fn visible_epoch(&self) -> u64 {
        self.visible_epoch
    }
}

/// Response context captured before opening a daemon connection. Contextual
/// responses must echo these values exactly; a shape-valid response from a
/// different request is rejected.
#[derive(Clone)]
pub(crate) enum ExpectedResponse {
    Status,
    Diagnostics,
    Import,
    RootControl,
    Search {
        request_id: String,
        max_results: usize,
    },
    Detail {
        request_id: String,
        selection: SearchSelection,
    },
    Hydrate {
        request_id: String,
        selection: SearchSelection,
        body_offset_bytes: u64,
        body_limit_bytes: u32,
    },
    Cancel {
        request_id: String,
    },
}

impl ExpectedResponse {
    pub(crate) fn operation(&self) -> Operation {
        match self {
            Self::Status => Operation::Status,
            Self::Diagnostics => Operation::Diagnostics,
            Self::Import => Operation::Import,
            Self::RootControl => Operation::RootControl,
            Self::Search { .. } => Operation::Search,
            Self::Detail { .. } => Operation::Detail,
            Self::Hydrate { .. } => Operation::Hydrate,
            Self::Cancel { .. } => Operation::Cancel,
        }
    }
}

/// A validated request body and its exact response contract. Construction is
/// the only place a daemon request can cross the desktop IPC size boundary.
pub(crate) struct PreparedDaemonRequest {
    body: Vec<u8>,
    expected: ExpectedResponse,
    response_timeout: Duration,
}

impl PreparedDaemonRequest {
    pub(crate) fn new(
        body: Vec<u8>,
        expected: ExpectedResponse,
        response_timeout: Duration,
    ) -> Result<Self, DesktopError> {
        if body.len() > MAX_REQUEST_BYTES {
            return Err(DesktopError::new(
                "request_too_large",
                "请求超过本地 IPC 上限",
            ));
        }
        Ok(Self {
            body,
            expected,
            response_timeout,
        })
    }

    pub(crate) fn empty(expected: ExpectedResponse, response_timeout: Duration) -> Self {
        Self {
            body: Vec::new(),
            expected,
            response_timeout,
        }
    }

    pub(crate) fn body(&self) -> &[u8] {
        &self.body
    }

    pub(crate) fn expected(&self) -> &ExpectedResponse {
        &self.expected
    }

    pub(crate) fn response_timeout(&self) -> Duration {
        self.response_timeout
    }
}

pub(crate) fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub(crate) fn valid_stable_id(value: &str, prefix: &str) -> bool {
    value.len() == prefix.len() + 32
        && value.starts_with(prefix)
        && value[prefix.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}
