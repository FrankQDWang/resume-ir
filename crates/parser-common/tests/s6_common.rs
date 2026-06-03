use std::time::Duration;

use core_domain::{ErrorKind, RedactionLevel, SourceComponent};
use parser_common::{
    FileProbe, ParseOutput, ParseStatus, ParserError, ParserErrorKind, ResourceBudget, SupportLevel,
};

#[test]
fn exposes_parser_common_crate_identity() {
    assert_eq!(parser_common::crate_name(), "parser-common");
}

#[test]
fn file_probe_normalizes_extension_and_classifies_support_without_path_leakage() {
    let probe = FileProbe::from_bytes(Some(".DOCX"), b"PK\x03\x04synthetic zip body");

    assert_eq!(probe.extension(), Some("docx"));
    assert_eq!(probe.byte_len(), 22);
    assert!(probe.has_zip_header());
    assert!(!probe.has_ole_header());
    assert_eq!(
        SupportLevel::Supported,
        SupportLevel::Supported.max(SupportLevel::Possible)
    );

    let debug = format!("{probe:?}");
    assert!(debug.contains("header_len"));
    assert!(!debug.contains("synthetic zip body"));
}

#[test]
fn resource_budget_timeout_maps_to_core_domain_parser_timeout() {
    let error = ResourceBudget::default()
        .with_timeout(Duration::ZERO)
        .check_parse_allowed(16)
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Timeout);
    assert!(error.retryable());

    let domain_error = error.to_resume_ir_error();

    assert_eq!(domain_error.kind, ErrorKind::ParserTimeout);
    assert_eq!(domain_error.source_component, SourceComponent::Parser);
    assert_eq!(domain_error.redaction_level, RedactionLevel::Sensitive);
    assert!(domain_error.retryable);
}

#[test]
fn resource_budget_nonzero_timeout_expires_after_deadline() {
    let parse_budget = ResourceBudget::default()
        .with_timeout(Duration::from_nanos(1))
        .begin(16)
        .unwrap();

    std::thread::sleep(Duration::from_millis(1));
    let error = parse_budget.check_deadline().unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Timeout);
}

#[test]
fn parser_error_mapping_redacts_diagnostics_but_keeps_kind_alignment() {
    let diagnostic = "SYNTHETIC_RAW_TEXT_SHOULD_NOT_APPEAR";
    let error = ParserError::corrupted(diagnostic);

    assert_eq!(error.kind(), ParserErrorKind::Corrupted);
    assert!(!error.retryable());
    assert!(!format!("{error:?}").contains(diagnostic));
    assert!(!error.to_string().contains(diagnostic));

    let domain_error = error.to_resume_ir_error();

    assert_eq!(domain_error.kind, ErrorKind::CorruptedDocument);
    assert_eq!(domain_error.diagnostic_message(), diagnostic);
    assert!(!format!("{domain_error:?}").contains(diagnostic));
}

#[test]
fn ocr_required_keeps_parser_code_when_mapped_to_core_domain_error() {
    let error = ParserError::ocr_required("synthetic scanned pdf has no text layer");

    assert_eq!(error.kind(), ParserErrorKind::OcrRequired);
    assert_eq!(error.code(), "OCR_REQUIRED");

    let domain_error = error.to_resume_ir_error();

    assert_eq!(domain_error.kind, ErrorKind::UnsupportedFormat);
    assert_eq!(domain_error.source_component, SourceComponent::Parser);
    assert!(!domain_error.retryable);
    assert!(domain_error.user_message.contains("OCR"));
}

#[test]
fn parse_output_debug_does_not_print_raw_extracted_text() {
    let output = ParseOutput::new(ParseStatus::TextExtracted, "SYNTHETIC PRIVATE RAW TEXT")
        .with_page_text(1, "SYNTHETIC PRIVATE RAW TEXT");

    assert_eq!(output.text(), "SYNTHETIC PRIVATE RAW TEXT");
    assert_eq!(output.pages().len(), 1);

    let debug = format!("{output:?}");

    assert!(debug.contains("text_len_chars"));
    assert!(!debug.contains("SYNTHETIC PRIVATE RAW TEXT"));
}
