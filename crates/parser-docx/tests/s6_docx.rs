use std::io::{Cursor, Write};

use parser_common::{
    ParseInput, ParseStatus, Parser, ParserErrorKind, ResourceBudget, SupportLevel,
};
use parser_docx::DocxParser;
use zip::{write::SimpleFileOptions, CompressionMethod, ZipWriter};

#[test]
fn exposes_parser_docx_crate_identity() {
    assert_eq!(parser_docx::crate_name(), "parser-docx");
}

#[test]
fn extracts_basic_docx_paragraph_text_from_synthetic_zip_xml() {
    let bytes = synthetic_docx(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Synthetic Candidate</w:t></w:r></w:p>
    <w:p><w:r><w:t>Rust &amp; Search</w:t></w:r></w:p>
  </w:body>
</w:document>"#,
    );
    let parser = DocxParser;
    let input = ParseInput::from_bytes(Some("docx"), &bytes);

    assert_eq!(parser.supports(input.probe()), SupportLevel::Supported);

    let output = parser.parse(input, ResourceBudget::default()).unwrap();

    assert_eq!(output.status(), ParseStatus::TextExtracted);
    assert_eq!(output.text(), "Synthetic Candidate\nRust & Search");
    assert!(!format!("{output:?}").contains("Synthetic Candidate"));
}

#[test]
fn corrupted_docx_returns_corrupted_error_without_panic_or_byte_leakage() {
    let parser = DocxParser;
    let bytes = b"not a zip archive containing word/document.xml";
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("docx"), bytes),
            ResourceBudget::default(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Corrupted);
    assert!(!format!("{error:?}").contains("not a zip"));
    assert!(!error.to_string().contains("word/document.xml"));
}

#[test]
fn docx_missing_document_xml_is_corrupted() {
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut buffer);
    writer.start_file("word/styles.xml", zip_options()).unwrap();
    writer.write_all(b"<w:styles/>").unwrap();
    writer.finish().unwrap();

    let parser = DocxParser;
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("docx"), buffer.get_ref()),
            ResourceBudget::default(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Corrupted);
}

#[test]
fn docx_parser_enforces_input_byte_budget() {
    let bytes = synthetic_docx(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body><w:p><w:r><w:t>Synthetic budget text</w:t></w:r></w:p></w:body>
</w:document>"#,
    );
    let parser = DocxParser;
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("docx"), &bytes),
            ResourceBudget::default().with_max_bytes(bytes.len() - 1),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::ResourceExhausted);
}

#[test]
fn docx_parser_rejects_excessive_entry_count() {
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut buffer);
    for index in 0..300 {
        writer
            .start_file(format!("word/filler-{index}.xml"), zip_options())
            .unwrap();
        writer.write_all(b"<filler/>").unwrap();
    }
    writer
        .start_file("word/document.xml", zip_options())
        .unwrap();
    writer
        .write_all(br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#)
        .unwrap();
    writer.finish().unwrap();

    let parser = DocxParser;
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("docx"), buffer.get_ref()),
            ResourceBudget::default(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::ResourceExhausted);
}

fn synthetic_docx(document_xml: &str) -> Vec<u8> {
    let mut buffer = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(&mut buffer);

    writer
        .start_file("[Content_Types].xml", zip_options())
        .unwrap();
    writer
        .write_all(
            br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>"#,
        )
        .unwrap();
    writer
        .start_file("word/document.xml", zip_options())
        .unwrap();
    writer.write_all(document_xml.as_bytes()).unwrap();
    writer.finish().unwrap();

    buffer.into_inner()
}

fn zip_options() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Stored)
}
