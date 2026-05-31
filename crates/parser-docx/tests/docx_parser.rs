//! DOCX parser behavior tests.

use parser_common::{ParseInput, Parser, ParserErrorKind, SupportLevel};
use parser_docx::DocxParser;
use std::io::Write;
use zip::write::FileOptions;

#[test]
fn extracts_text_from_word_document_xml_text_nodes() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = synthetic_docx(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:t>Synthetic profile</w:t></w:r></w:p>
    <w:p><w:r><w:t>Rust systems testing</w:t></w:r></w:p>
  </w:body>
</w:document>"#,
    )?;
    let input = ParseInput::new("synthetic.docx", bytes);

    let output = DocxParser.parse(&input)?;

    assert_eq!(
        output.text(),
        Some("Synthetic profile\nRust systems testing")
    );
    assert_eq!(output.support_level(), SupportLevel::Text);
    assert!(!output.ocr_required());
    Ok(())
}

#[test]
fn corrupt_docx_maps_to_corrupted_document_error() -> Result<(), Box<dyn std::error::Error>> {
    let input = ParseInput::new("synthetic.docx", b"this is not a zip archive".to_vec());

    let err = match DocxParser.parse(&input) {
        Ok(_) => return Err("expected corrupt document error".into()),
        Err(err) => err,
    };

    assert_eq!(err.kind(), ParserErrorKind::CorruptedDocument);
    assert!(!err.retryable());
    assert_eq!(err.source_component(), "parser-docx");
    assert!(!format!("{err:?}").contains("this is not a zip archive"));
    Ok(())
}

fn synthetic_docx(document_xml: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let cursor = std::io::Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(cursor);
    let options = FileOptions::default();

    writer.start_file("[Content_Types].xml", options)?;
    writer.write_all(br#"<?xml version="1.0" encoding="UTF-8"?><Types/>"#)?;
    writer.start_file("word/document.xml", options)?;
    writer.write_all(document_xml.as_bytes())?;

    Ok(writer.finish()?.into_inner())
}
