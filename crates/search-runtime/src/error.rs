use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchRuntimeErrorCode {
    Unavailable,
    Integrity,
    SemanticDisabled,
    SelectionTooLarge,
    InvalidRequest,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SearchRuntimeError {
    code: SearchRuntimeErrorCode,
}

impl SearchRuntimeError {
    pub fn code(self) -> SearchRuntimeErrorCode {
        self.code
    }

    pub(crate) fn unavailable() -> Self {
        Self {
            code: SearchRuntimeErrorCode::Unavailable,
        }
    }

    pub(crate) fn integrity() -> Self {
        Self {
            code: SearchRuntimeErrorCode::Integrity,
        }
    }

    /// Reports a violated facade invariant detected while consuming one
    /// generation-pinned result set.
    pub fn integrity_violation() -> Self {
        Self::integrity()
    }

    pub(crate) fn semantic_disabled() -> Self {
        Self {
            code: SearchRuntimeErrorCode::SemanticDisabled,
        }
    }

    pub(crate) fn selection_too_large() -> Self {
        Self {
            code: SearchRuntimeErrorCode::SelectionTooLarge,
        }
    }

    pub(crate) fn invalid_request() -> Self {
        Self {
            code: SearchRuntimeErrorCode::InvalidRequest,
        }
    }
}

impl fmt::Debug for SearchRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchRuntimeError")
            .field("code", &self.code)
            .finish()
    }
}

impl fmt::Display for SearchRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.code {
            SearchRuntimeErrorCode::Unavailable => "SEARCH_UNAVAILABLE",
            SearchRuntimeErrorCode::Integrity => "SEARCH_RUNTIME_INTEGRITY",
            SearchRuntimeErrorCode::SemanticDisabled => "SEMANTIC_DISABLED",
            SearchRuntimeErrorCode::SelectionTooLarge => "SEARCH_SELECTION_TOO_LARGE",
            SearchRuntimeErrorCode::InvalidRequest => "INVALID_SEARCH_REQUEST",
        })
    }
}

impl std::error::Error for SearchRuntimeError {}
