use std::fmt;
use std::time::{Duration, Instant};

use core_domain::{DocumentStatus, ErrorKind, RedactionLevel, ResumeIrError, SourceComponent};

pub fn crate_name() -> &'static str {
    "parser-common"
}

pub type Result<T> = std::result::Result<T, ParserError>;

pub trait Parser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel;
    fn parse(&self, input: ParseInput<'_>, budget: ResourceBudget) -> Result<ParseOutput>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SupportLevel {
    Unsupported,
    Possible,
    Supported,
}

#[derive(Clone, PartialEq, Eq)]
pub struct FileProbe {
    extension: Option<String>,
    header: Vec<u8>,
    byte_len: usize,
}

impl FileProbe {
    const MAX_HEADER_BYTES: usize = 32;

    pub fn from_bytes(extension: Option<&str>, bytes: &[u8]) -> Self {
        Self {
            extension: normalize_extension(extension),
            header: bytes.iter().take(Self::MAX_HEADER_BYTES).copied().collect(),
            byte_len: bytes.len(),
        }
    }

    pub fn extension(&self) -> Option<&str> {
        self.extension.as_deref()
    }

    pub fn byte_len(&self) -> usize {
        self.byte_len
    }

    pub fn has_zip_header(&self) -> bool {
        self.header.starts_with(b"PK\x03\x04")
            || self.header.starts_with(b"PK\x05\x06")
            || self.header.starts_with(b"PK\x07\x08")
    }

    pub fn has_pdf_header(&self) -> bool {
        self.header.starts_with(b"%PDF-")
    }
}

impl fmt::Debug for FileProbe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FileProbe")
            .field("extension", &self.extension)
            .field("byte_len", &self.byte_len)
            .field("header_len", &self.header.len())
            .field("zip_header", &self.has_zip_header())
            .field("pdf_header", &self.has_pdf_header())
            .finish()
    }
}

fn normalize_extension(extension: Option<&str>) -> Option<String> {
    let extension = extension?.trim().trim_start_matches('.');
    if extension.is_empty() {
        return None;
    }

    Some(extension.to_ascii_lowercase())
}

pub struct ParseInput<'a> {
    bytes: &'a [u8],
    probe: FileProbe,
}

impl<'a> ParseInput<'a> {
    pub fn from_bytes(extension: Option<&str>, bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            probe: FileProbe::from_bytes(extension, bytes),
        }
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn probe(&self) -> &FileProbe {
        &self.probe
    }
}

impl fmt::Debug for ParseInput<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParseInput")
            .field("byte_len", &self.bytes.len())
            .field("probe", &self.probe)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceBudget {
    max_bytes: Option<usize>,
    timeout: Option<Duration>,
}

impl ResourceBudget {
    pub fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn begin(&self, byte_len: usize) -> Result<ParseBudget> {
        if matches!(self.timeout, Some(timeout) if timeout.is_zero()) {
            return Err(ParserError::timeout(
                "parser budget expired before parse start",
            ));
        }

        if let Some(max_bytes) = self.max_bytes {
            if byte_len > max_bytes {
                return Err(ParserError::resource_exhausted(
                    "document exceeds parser byte budget",
                ));
            }
        }

        let deadline = self
            .timeout
            .and_then(|timeout| Instant::now().checked_add(timeout));

        Ok(ParseBudget { deadline })
    }

    pub fn check_parse_allowed(&self, byte_len: usize) -> Result<()> {
        self.begin(byte_len).map(|_| ())
    }
}

impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            max_bytes: None,
            timeout: Some(Duration::from_secs(30)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ParseBudget {
    deadline: Option<Instant>,
}

impl ParseBudget {
    pub fn check_deadline(&self) -> Result<()> {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(ParserError::timeout("parser budget expired during parse"));
        }

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParserErrorKind {
    Unsupported,
    Corrupted,
    Encrypted,
    Timeout,
    OcrRequired,
    ResourceExhausted,
    Io,
    Cancelled,
    Internal,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ParserError {
    kind: ParserErrorKind,
    retryable: bool,
    user_message: String,
    diagnostic_message: String,
}

impl ParserError {
    pub fn new(
        kind: ParserErrorKind,
        retryable: bool,
        user_message: impl Into<String>,
        diagnostic_message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            retryable,
            user_message: user_message.into(),
            diagnostic_message: diagnostic_message.into(),
        }
    }

    pub fn unsupported(diagnostic_message: impl Into<String>) -> Self {
        Self::new(
            ParserErrorKind::Unsupported,
            false,
            "This document format is not supported.",
            diagnostic_message,
        )
    }

    pub fn corrupted(diagnostic_message: impl Into<String>) -> Self {
        Self::new(
            ParserErrorKind::Corrupted,
            false,
            "This document could not be parsed because it appears to be damaged.",
            diagnostic_message,
        )
    }

    pub fn encrypted(diagnostic_message: impl Into<String>) -> Self {
        Self::new(
            ParserErrorKind::Encrypted,
            false,
            "This document is encrypted and needs a password before parsing.",
            diagnostic_message,
        )
    }

    pub fn timeout(diagnostic_message: impl Into<String>) -> Self {
        Self::new(
            ParserErrorKind::Timeout,
            true,
            "Document parsing timed out.",
            diagnostic_message,
        )
    }

    pub fn ocr_required(diagnostic_message: impl Into<String>) -> Self {
        Self::new(
            ParserErrorKind::OcrRequired,
            false,
            "This PDF needs OCR before text extraction.",
            diagnostic_message,
        )
    }

    pub fn resource_exhausted(diagnostic_message: impl Into<String>) -> Self {
        Self::new(
            ParserErrorKind::ResourceExhausted,
            true,
            "Document parsing exceeded its resource budget.",
            diagnostic_message,
        )
    }

    pub fn io(diagnostic_message: impl Into<String>) -> Self {
        Self::new(
            ParserErrorKind::Io,
            true,
            "Document parsing failed while reading parser input.",
            diagnostic_message,
        )
    }

    pub fn internal(diagnostic_message: impl Into<String>) -> Self {
        Self::new(
            ParserErrorKind::Internal,
            false,
            "Document parsing failed because of an internal parser error.",
            diagnostic_message,
        )
    }

    pub fn kind(&self) -> ParserErrorKind {
        self.kind
    }

    pub fn retryable(&self) -> bool {
        self.retryable
    }

    pub fn code(&self) -> &'static str {
        match self.kind {
            ParserErrorKind::Unsupported => "UNSUPPORTED_FORMAT",
            ParserErrorKind::Corrupted => "CORRUPTED_DOCUMENT",
            ParserErrorKind::Encrypted => "ENCRYPTED_DOCUMENT",
            ParserErrorKind::Timeout => "PARSER_TIMEOUT",
            ParserErrorKind::OcrRequired => "OCR_REQUIRED",
            ParserErrorKind::ResourceExhausted => "RESOURCE_EXHAUSTED",
            ParserErrorKind::Io => "IO_ERROR",
            ParserErrorKind::Cancelled => "CANCELLED",
            ParserErrorKind::Internal => "INTERNAL_BUG",
        }
    }

    pub fn diagnostic_message(&self) -> &str {
        &self.diagnostic_message
    }

    pub fn to_resume_ir_error(&self) -> ResumeIrError {
        ResumeIrError::new(
            self.domain_error_kind(),
            self.retryable,
            self.user_message.clone(),
            self.diagnostic_message.clone(),
            RedactionLevel::Sensitive,
            SourceComponent::Parser,
        )
    }

    fn domain_error_kind(&self) -> ErrorKind {
        match self.kind {
            ParserErrorKind::Unsupported | ParserErrorKind::OcrRequired => {
                ErrorKind::UnsupportedFormat
            }
            ParserErrorKind::Corrupted => ErrorKind::CorruptedDocument,
            ParserErrorKind::Encrypted => ErrorKind::EncryptedDocument,
            ParserErrorKind::Timeout => ErrorKind::ParserTimeout,
            ParserErrorKind::ResourceExhausted => ErrorKind::ResourceExhausted,
            ParserErrorKind::Io => ErrorKind::IoError,
            ParserErrorKind::Cancelled => ErrorKind::Cancelled,
            ParserErrorKind::Internal => ErrorKind::InternalBug,
        }
    }
}

impl fmt::Debug for ParserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParserError")
            .field("kind", &self.kind)
            .field("code", &self.code())
            .field("retryable", &self.retryable)
            .field("user_message", &self.user_message)
            .field("diagnostic_message", &"<redacted>")
            .finish()
    }
}

impl fmt::Display for ParserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} [code={}, retryable={}]",
            self.user_message,
            self.code(),
            self.retryable
        )
    }
}

impl std::error::Error for ParserError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseStatus {
    TextExtracted,
    TextLayer,
    OcrRequired,
}

impl ParseStatus {
    pub fn document_status(self) -> DocumentStatus {
        match self {
            ParseStatus::TextExtracted | ParseStatus::TextLayer => DocumentStatus::TextExtracted,
            ParseStatus::OcrRequired => DocumentStatus::OcrRequired,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ParseOutput {
    status: ParseStatus,
    text: String,
    pages: Vec<PageText>,
    page_count: Option<usize>,
}

impl ParseOutput {
    pub fn new(status: ParseStatus, text: impl Into<String>) -> Self {
        Self {
            status,
            text: text.into(),
            pages: Vec::new(),
            page_count: None,
        }
    }

    pub fn with_page_text(mut self, page_no: usize, text: impl Into<String>) -> Self {
        self.pages.push(PageText::new(page_no, text));
        self
    }

    pub fn with_page_count(mut self, page_count: usize) -> Self {
        self.page_count = Some(page_count);
        self
    }

    pub fn status(&self) -> ParseStatus {
        self.status
    }

    pub fn document_status(&self) -> DocumentStatus {
        self.status.document_status()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn pages(&self) -> &[PageText] {
        &self.pages
    }

    pub fn page_count(&self) -> Option<usize> {
        self.page_count
    }
}

impl fmt::Debug for ParseOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParseOutput")
            .field("status", &self.status)
            .field("text_len_chars", &self.text.chars().count())
            .field("pages", &self.pages)
            .field("page_count", &self.page_count)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PageText {
    page_no: usize,
    text: String,
}

impl PageText {
    pub fn new(page_no: usize, text: impl Into<String>) -> Self {
        Self {
            page_no,
            text: text.into(),
        }
    }

    pub fn page_no(&self) -> usize {
        self.page_no
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for PageText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PageText")
            .field("page_no", &self.page_no)
            .field("text_len_chars", &self.text.chars().count())
            .finish()
    }
}
