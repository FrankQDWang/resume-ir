use parser_common::{FileProbe, ParseInput, ParseStatus, Parser, ResourceBudget, SupportLevel};
use parser_docx::DocxParser;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

fn fixture_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("resume_ir_docx_{name}_{}.docx", std::process::id()))
}

fn write_docx(path: &Path, document_xml: &str) {
    let file = fs::File::create(path).expect("create docx");
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default();
    zip.start_file("[Content_Types].xml", options)
        .expect("content types");
    zip.write_all(br#"<?xml version="1.0"?><Types/>"#)
        .expect("write content types");
    zip.start_file("word/document.xml", options)
        .expect("document xml");
    zip.write_all(document_xml.as_bytes())
        .expect("write document xml");
    zip.finish().expect("finish zip");
}

#[test]
fn docx_parser_extracts_basic_text() {
    let path = fixture_path("happy");
    write_docx(
        &path,
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Java</w:t></w:r><w:r><w:t> engineer</w:t></w:r></w:p></w:body></w:document>"#,
    );
    let parser = DocxParser;
    let bytes = fs::read(&path).expect("read fixture");

    let output = parser
        .parse(
            ParseInput { path, bytes },
            ResourceBudget::new(Duration::from_secs(1)),
        )
        .expect("parse docx");

    assert_eq!(output.status, ParseStatus::Parsed);
    assert_eq!(output.text, "Java engineer");
}

#[test]
fn docx_parser_reports_corrupt_zip() {
    let parser = DocxParser;

    let error = parser
        .parse(
            ParseInput {
                path: PathBuf::from("broken.docx"),
                bytes: b"not a zip".to_vec(),
            },
            ResourceBudget::new(Duration::from_secs(1)),
        )
        .expect_err("corrupt docx should fail");

    assert!(error.user_message().contains("DOCX"));
}

#[test]
fn docx_parser_supports_docx_extension() {
    let parser = DocxParser;
    let probe = FileProbe {
        path: PathBuf::from("resume.docx"),
        extension: "docx".to_owned(),
    };

    assert_eq!(parser.supports(&probe), SupportLevel::Supported);
}
