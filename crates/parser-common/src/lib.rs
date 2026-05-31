//! Shared parser contracts and error types.

use std::fmt;
use std::time::{Duration, Instant};
use thiserror::Error;

/// Parser support classification for a document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportLevel {
    /// Parser can extract text.
    Text,
    /// PDF appears to contain a text layer, but extraction is not implemented here.
    TextLayer,
    /// Parser detected a scanned or image-only document that needs OCR.
    OcrRequired,
    /// Lightweight detection could not determine whether text or OCR is needed.
    Unknown,
    /// Parser does not support this document.
    Unsupported,
}

/// Bytes and source metadata passed to a parser.
#[derive(Clone, Eq, PartialEq)]
pub struct ParseInput {
    source_name: String,
    bytes: Vec<u8>,
}

impl ParseInput {
    /// Creates parser input from a display-safe source name and raw bytes.
    #[must_use]
    pub fn new(source_name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            source_name: source_name.into(),
            bytes,
        }
    }

    /// Returns the source name supplied by the caller.
    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Returns the document bytes.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for ParseInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParseInput")
            .field("source_name", &"[redacted source name]")
            .field("byte_len", &self.bytes.len())
            .finish()
    }
}

/// Output produced by a parser.
#[derive(Clone, Eq, PartialEq)]
pub struct ParseOutput {
    source_name: String,
    text: Option<String>,
    support_level: SupportLevel,
    ocr_required: bool,
}

impl fmt::Debug for ParseOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParseOutput")
            .field("source_name", &"[redacted source name]")
            .field("text_present", &self.text.is_some())
            .field("support_level", &self.support_level)
            .field("ocr_required", &self.ocr_required)
            .finish()
    }
}

impl ParseOutput {
    /// Creates text output.
    #[must_use]
    pub fn from_text(
        source_name: impl Into<String>,
        text: String,
        support_level: SupportLevel,
    ) -> Self {
        Self {
            source_name: source_name.into(),
            text: Some(text),
            support_level,
            ocr_required: matches!(support_level, SupportLevel::OcrRequired),
        }
    }

    /// Creates classification output without extracted text.
    #[must_use]
    pub fn classification(source_name: impl Into<String>, support_level: SupportLevel) -> Self {
        Self {
            source_name: source_name.into(),
            text: None,
            support_level,
            ocr_required: matches!(support_level, SupportLevel::OcrRequired),
        }
    }

    /// Returns the caller-supplied source name.
    #[must_use]
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Returns extracted text when available.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// Returns support classification.
    #[must_use]
    pub fn support_level(&self) -> SupportLevel {
        self.support_level
    }

    /// Returns whether the document requires OCR.
    #[must_use]
    pub fn ocr_required(&self) -> bool {
        self.ocr_required
    }
}

/// Parser behavior contract.
pub trait Parser {
    /// Parses or classifies a document.
    fn parse(&self, input: &ParseInput) -> Result<ParseOutput, ParserError>;
}

/// Elapsed-budget accounting used by parser callers.
///
/// This type does not interrupt a parser after it starts. Hard cancellation
/// belongs at a worker or process boundary; this budget records elapsed work
/// and lets callers fail fast when no budget remains.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParseTimeout {
    budget: Duration,
    elapsed: Duration,
}

impl ParseTimeout {
    /// Creates a timeout with no remaining budget.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            budget: Duration::ZERO,
            elapsed: Duration::ZERO,
        }
    }

    /// Creates a timeout from remaining budget.
    #[must_use]
    pub fn from_budget(budget: Duration) -> Self {
        Self {
            budget,
            elapsed: Duration::ZERO,
        }
    }

    /// Creates a timeout from total budget and elapsed work.
    #[must_use]
    pub fn from_elapsed(budget: Duration, elapsed: Duration) -> Self {
        Self { budget, elapsed }
    }

    /// Returns the configured budget.
    #[must_use]
    pub fn budget(&self) -> Duration {
        self.budget
    }

    /// Returns elapsed time already charged against the budget.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    fn remaining_budget(&self) -> Option<Duration> {
        self.budget.checked_sub(self.elapsed)
    }
}

/// Runs a parser with elapsed-budget accounting.
///
/// The budget is checked before parsing and again after `Parser::parse`
/// returns. This is not a hard timeout and will not preempt a parser that hangs.
pub fn parse_with_elapsed_budget<P: Parser>(
    parser: &P,
    input: &ParseInput,
    timeout: ParseTimeout,
) -> Result<ParseOutput, ParserError> {
    let Some(remaining_budget) = timeout.remaining_budget() else {
        return Err(ParserError::timeout("parser-common"));
    };

    if remaining_budget.is_zero() {
        return Err(ParserError::timeout("parser-common"));
    }

    let started = Instant::now();
    let output = parser.parse(input)?;
    if started.elapsed() > remaining_budget {
        return Err(ParserError::timeout("parser-common"));
    }

    Ok(output)
}

/// Runs a parser with timeout-shaped elapsed-budget accounting.
///
/// This compatibility name maps exhausted elapsed budget to
/// `ParserErrorKind::Timeout`, but it is not a hard interrupt.
pub fn parse_with_timeout<P: Parser>(
    parser: &P,
    input: &ParseInput,
    timeout: ParseTimeout,
) -> Result<ParseOutput, ParserError> {
    parse_with_elapsed_budget(parser, input, timeout)
}

/// Stable parser error categories.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum ParserErrorKind {
    /// Source document is corrupt or malformed.
    #[error("corrupted document")]
    CorruptedDocument,
    /// Source document is encrypted.
    #[error("encrypted document")]
    EncryptedDocument,
    /// Document type is unsupported.
    #[error("unsupported document")]
    UnsupportedDocument,
    /// Parse exceeded caller budget.
    #[error("timeout")]
    Timeout,
    /// Internal parser failure.
    #[error("internal error")]
    Internal,
}

/// Structured parser error with redacted debug output.
#[derive(Clone, Eq, Error, PartialEq)]
#[error("{kind}: {user_message}")]
pub struct ParserError {
    kind: ParserErrorKind,
    retryable: bool,
    user_message: String,
    diagnostic_message: String,
    source_component: String,
}

impl ParserError {
    /// Creates a parser error.
    #[must_use]
    pub fn new(
        kind: ParserErrorKind,
        retryable: bool,
        user_message: impl Into<String>,
        diagnostic_message: impl Into<String>,
        source_component: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            retryable,
            user_message: user_message.into(),
            diagnostic_message: diagnostic_message.into(),
            source_component: source_component.into(),
        }
    }

    /// Creates a timeout parser error.
    #[must_use]
    pub fn timeout(source_component: impl Into<String>) -> Self {
        Self::new(
            ParserErrorKind::Timeout,
            true,
            "Parsing timed out.",
            "parse timeout budget was exhausted",
            source_component,
        )
    }

    /// Returns the stable error kind.
    #[must_use]
    pub fn kind(&self) -> ParserErrorKind {
        self.kind
    }

    /// Returns whether retrying may succeed.
    #[must_use]
    pub fn retryable(&self) -> bool {
        self.retryable
    }

    /// Returns a safe user-facing message.
    #[must_use]
    pub fn user_message(&self) -> &str {
        &self.user_message
    }

    /// Returns the producing component.
    #[must_use]
    pub fn source_component(&self) -> &str {
        &self.source_component
    }

    /// Returns the local diagnostic message.
    ///
    /// This may contain local implementation detail and must not be included in
    /// `Debug` output.
    #[must_use]
    pub fn local_diagnostic_message(&self) -> &str {
        &self.diagnostic_message
    }
}

impl fmt::Debug for ParserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParserError")
            .field("kind", &self.kind)
            .field("retryable", &self.retryable)
            .field("user_message", &self.user_message)
            .field("diagnostic_message", &"[redacted local diagnostic]")
            .field("source_component", &self.source_component)
            .finish()
    }
}
