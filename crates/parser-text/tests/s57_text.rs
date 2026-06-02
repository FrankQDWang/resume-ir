use parser_common::{
    ParseInput, ParseStatus, Parser, ParserErrorKind, ResourceBudget, SupportLevel,
};
use parser_text::TxtParser;

#[test]
fn exposes_parser_text_crate_identity() {
    assert_eq!(parser_text::crate_name(), "parser-text");
}

#[test]
fn extracts_utf8_text_without_leaking_contents_in_debug() {
    let parser = TxtParser;
    let input = ParseInput::from_bytes(
        Some("txt"),
        b"Synthetic Candidate\r\nRust search infrastructure\r\n",
    );

    assert_eq!(parser.supports(input.probe()), SupportLevel::Supported);

    let output = parser.parse(input, ResourceBudget::default()).unwrap();

    assert_eq!(output.status(), ParseStatus::TextExtracted);
    assert_eq!(
        output.text(),
        "Synthetic Candidate\nRust search infrastructure\n"
    );
    assert!(!format!("{output:?}").contains("Synthetic Candidate"));
}

#[test]
fn extracts_utf16_little_endian_text_with_bom() {
    let parser = TxtParser;
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(
        "Synthetic UTF16 Candidate"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );

    let output = parser
        .parse(
            ParseInput::from_bytes(Some("txt"), &bytes),
            ResourceBudget::default(),
        )
        .unwrap();

    assert_eq!(output.text(), "Synthetic UTF16 Candidate");
}

#[test]
fn extracts_utf16_big_endian_text_with_bom() {
    let parser = TxtParser;
    let mut bytes = vec![0xfe, 0xff];
    bytes.extend(
        "Synthetic UTF16BE Candidate"
            .encode_utf16()
            .flat_map(u16::to_be_bytes),
    );

    let output = parser
        .parse(
            ParseInput::from_bytes(Some("txt"), &bytes),
            ResourceBudget::default(),
        )
        .unwrap();

    assert_eq!(output.text(), "Synthetic UTF16BE Candidate");
}

#[test]
fn unsupported_extension_is_rejected_without_text_leakage() {
    let parser = TxtParser;
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("pdf"), b"Synthetic txt payload"),
            ResourceBudget::default(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Unsupported);
    assert!(!format!("{error:?}").contains("Synthetic"));
}

#[test]
fn invalid_utf8_is_corrupted_without_byte_leakage() {
    let parser = TxtParser;
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("txt"), b"Synthetic \xff Candidate"),
            ResourceBudget::default(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Corrupted);
    assert!(!format!("{error:?}").contains("Synthetic"));
    assert!(!error.to_string().contains("Synthetic"));
}

#[test]
fn invalid_utf16_surrogate_is_corrupted_without_text_leakage() {
    let parser = TxtParser;
    let bytes = [0xff, 0xfe, 0x00, 0xd8];
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("txt"), &bytes),
            ResourceBudget::default(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Corrupted);
    assert!(!format!("{error:?}").contains("d800"));
}

#[test]
fn txt_parser_enforces_input_byte_budget() {
    let parser = TxtParser;
    let bytes = b"Synthetic budget text";
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("txt"), bytes),
            ResourceBudget::default().with_max_bytes(bytes.len() - 1),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::ResourceExhausted);
}
