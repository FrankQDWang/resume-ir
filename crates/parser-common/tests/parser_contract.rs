//! Parser contract tests.

use parser_common::{
    parse_with_elapsed_budget, parse_with_timeout, ParseInput, ParseOutput, ParseTimeout, Parser,
    ParserError, ParserErrorKind, SupportLevel,
};
use std::time::Duration;

struct StaticParser;

impl Parser for StaticParser {
    fn parse(&self, input: &ParseInput) -> Result<ParseOutput, ParserError> {
        Ok(ParseOutput::from_text(
            input.source_name().to_owned(),
            "synthetic resume text".to_owned(),
            SupportLevel::Text,
        ))
    }
}

#[test]
fn zero_timeout_budget_returns_retryable_timeout_without_calling_parser(
) -> Result<(), Box<dyn std::error::Error>> {
    let input = ParseInput::new("synthetic.docx", b"unused".to_vec());

    let err = match parse_with_elapsed_budget(&StaticParser, &input, ParseTimeout::zero()) {
        Ok(_) => return Err("expected timeout error".into()),
        Err(err) => err,
    };

    assert_eq!(err.kind(), ParserErrorKind::Timeout);
    assert!(err.retryable());
    assert_eq!(err.source_component(), "parser-common");
    Ok(())
}

#[test]
fn pre_charged_elapsed_budget_returns_retryable_timeout() -> Result<(), Box<dyn std::error::Error>>
{
    let input = ParseInput::new("synthetic.docx", b"unused".to_vec());

    let err = match parse_with_timeout(
        &StaticParser,
        &input,
        ParseTimeout::from_elapsed(Duration::from_secs(1), Duration::from_secs(2)),
    ) {
        Ok(_) => return Err("expected timeout error".into()),
        Err(err) => err,
    };

    assert_eq!(err.kind(), ParserErrorKind::Timeout);
    assert!(err.retryable());
    Ok(())
}

#[test]
fn elapsed_budget_helper_returns_parser_output_when_budget_remains() -> Result<(), ParserError> {
    let input = ParseInput::new("synthetic.docx", b"unused".to_vec());

    let output = parse_with_elapsed_budget(
        &StaticParser,
        &input,
        ParseTimeout::from_budget(Duration::from_secs(1)),
    )?;

    assert_eq!(output.text(), Some("synthetic resume text"));
    assert_eq!(output.support_level(), SupportLevel::Text);
    assert!(!output.ocr_required());
    Ok(())
}

#[test]
fn parser_error_debug_redacts_local_diagnostics() {
    let err = ParserError::new(
        ParserErrorKind::CorruptedDocument,
        false,
        "Document could not be parsed.",
        "/local/private/path/synthetic.docx had raw bytes",
        "parser-docx",
    );

    let debug = format!("{err:?}");

    assert!(debug.contains("[redacted local diagnostic]"));
    assert!(!debug.contains("/local/private/path"));
    assert!(!debug.contains("raw bytes"));
}

#[test]
fn parse_output_debug_redacts_source_name_and_text() {
    let output = ParseOutput::from_text(
        "synthetic-private-source.docx",
        "synthetic extracted private content".to_owned(),
        SupportLevel::Text,
    );

    let debug = format!("{output:?}");

    assert!(debug.contains("text_present: true"));
    assert!(!debug.contains("synthetic-private-source"));
    assert!(!debug.contains("synthetic extracted private content"));
}
