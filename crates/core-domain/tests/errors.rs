use core_domain::{AppError, ErrorKind, RedactionLevel, SourceComponent};

#[test]
fn error_model_exposes_retry_and_redaction_fields() {
    let error = AppError::new(
        ErrorKind::ParserTimeout,
        true,
        "The document took too long to parse.",
        "parser exceeded 30s budget for doc_0001",
        RedactionLevel::Safe,
        SourceComponent::Parser,
    );

    assert_eq!(error.kind(), ErrorKind::ParserTimeout);
    assert!(error.retryable());
    assert_eq!(error.redaction_level(), RedactionLevel::Safe);
    assert_eq!(error.source_component(), SourceComponent::Parser);
    assert_eq!(
        error.diagnostic_message_for_logs(),
        "parser exceeded 30s budget for doc_0001"
    );
}

#[test]
fn confidential_diagnostics_are_redacted_for_logs() {
    let error = AppError::new(
        ErrorKind::IoError,
        false,
        "Cannot read the selected file.",
        "/Users/example/resumes/private-name.pdf",
        RedactionLevel::Confidential,
        SourceComponent::FsCrawler,
    );

    assert_eq!(error.user_message(), "Cannot read the selected file.");
    assert_eq!(error.diagnostic_message_for_logs(), "[redacted]");
}
