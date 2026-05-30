use core_domain::{AppError, ErrorKind, RedactionLevel, SourceComponent};
use std::path::PathBuf;
use std::time::Duration;

pub type ParseResult<T> = Result<T, ParserError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportLevel {
    Unsupported,
    Maybe,
    Supported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileProbe {
    pub path: PathBuf,
    pub extension: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseInput {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseStatus {
    Parsed,
    OcrRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseOutput {
    pub text: String,
    pub status: ParseStatus,
    pub page_count: Option<u32>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceBudget {
    pub timeout: Duration,
}

impl ResourceBudget {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }
}

pub trait Parser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel;
    fn parse(&self, input: ParseInput, budget: ResourceBudget) -> ParseResult<ParseOutput>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParserErrorKind {
    Timeout,
    Unsupported,
    Corrupted,
    Io,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParserError {
    kind: ParserErrorKind,
    retryable: bool,
    user_message: String,
    diagnostic_message: String,
    source_component: SourceComponent,
}

impl ParserError {
    #[must_use]
    pub fn timeout(
        source_component: SourceComponent,
        diagnostic_message: impl Into<String>,
    ) -> Self {
        Self {
            kind: ParserErrorKind::Timeout,
            retryable: true,
            user_message: "Document parsing timed out.".to_owned(),
            diagnostic_message: diagnostic_message.into(),
            source_component,
        }
    }

    #[must_use]
    pub fn corrupted(
        user_message: impl Into<String>,
        diagnostic_message: impl Into<String>,
    ) -> Self {
        Self {
            kind: ParserErrorKind::Corrupted,
            retryable: false,
            user_message: user_message.into(),
            diagnostic_message: diagnostic_message.into(),
            source_component: SourceComponent::Parser,
        }
    }

    #[must_use]
    pub fn io(user_message: impl Into<String>, diagnostic_message: impl Into<String>) -> Self {
        Self {
            kind: ParserErrorKind::Io,
            retryable: true,
            user_message: user_message.into(),
            diagnostic_message: diagnostic_message.into(),
            source_component: SourceComponent::Parser,
        }
    }

    #[must_use]
    pub fn internal(
        user_message: impl Into<String>,
        diagnostic_message: impl Into<String>,
    ) -> Self {
        Self {
            kind: ParserErrorKind::Internal,
            retryable: false,
            user_message: user_message.into(),
            diagnostic_message: diagnostic_message.into(),
            source_component: SourceComponent::Parser,
        }
    }

    #[must_use]
    pub fn kind(&self) -> ParserErrorKind {
        self.kind
    }

    #[must_use]
    pub fn retryable(&self) -> bool {
        self.retryable
    }

    #[must_use]
    pub fn user_message(&self) -> &str {
        &self.user_message
    }

    #[must_use]
    pub fn to_app_error(&self) -> AppError {
        let kind = match self.kind {
            ParserErrorKind::Timeout => ErrorKind::ParserTimeout,
            ParserErrorKind::Unsupported => ErrorKind::UnsupportedFormat,
            ParserErrorKind::Corrupted => ErrorKind::CorruptedDocument,
            ParserErrorKind::Io => ErrorKind::IoError,
            ParserErrorKind::Internal => ErrorKind::InternalBug,
        };
        AppError::new(
            kind,
            self.retryable,
            self.user_message.clone(),
            self.diagnostic_message.clone(),
            RedactionLevel::Safe,
            self.source_component,
        )
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "parser-common"
}
