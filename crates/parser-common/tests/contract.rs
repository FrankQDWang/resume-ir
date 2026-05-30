use core_domain::{ErrorKind, SourceComponent};
use parser_common::{
    FileProbe, ParseInput, ParseOutput, ParseStatus, Parser, ParserError, ParserErrorKind,
    ResourceBudget, SupportLevel,
};
use std::path::PathBuf;
use std::time::Duration;

struct FakeParser;

impl Parser for FakeParser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel {
        if probe.extension == "txt" {
            SupportLevel::Supported
        } else {
            SupportLevel::Unsupported
        }
    }

    fn parse(
        &self,
        input: ParseInput,
        _budget: ResourceBudget,
    ) -> parser_common::ParseResult<ParseOutput> {
        Ok(ParseOutput {
            text: String::from_utf8(input.bytes).expect("utf8 fixture"),
            status: ParseStatus::Parsed,
            page_count: None,
            warnings: Vec::new(),
        })
    }
}

#[test]
fn parser_trait_accepts_probe_and_parse_input() {
    let parser = FakeParser;
    let probe = FileProbe {
        path: PathBuf::from("resume.txt"),
        extension: "txt".to_owned(),
    };
    let input = ParseInput {
        path: probe.path.clone(),
        bytes: b"hello".to_vec(),
    };

    assert_eq!(parser.supports(&probe), SupportLevel::Supported);
    assert_eq!(
        parser
            .parse(input, ResourceBudget::new(Duration::from_secs(1)))
            .expect("parse")
            .text,
        "hello"
    );
}

#[test]
fn timeout_error_maps_to_core_error_model() {
    let error = ParserError::timeout(SourceComponent::Parser, "parser exceeded budget");

    assert_eq!(error.kind(), ParserErrorKind::Timeout);
    assert!(error.retryable());
    assert_eq!(error.to_app_error().kind(), ErrorKind::ParserTimeout);
    assert_eq!(
        error.to_app_error().user_message(),
        "Document parsing timed out."
    );
}
