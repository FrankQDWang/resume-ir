use parser_common::{FileProbe, ParseInput, ParseStatus, Parser, ResourceBudget, SupportLevel};
use parser_pdf::PdfParser;
use std::path::PathBuf;
use std::time::Duration;

#[test]
fn text_layer_pdf_is_marked_parsed() {
    let parser = PdfParser;
    let bytes =
        b"%PDF-1.4\n1 0 obj <<>> stream\nBT (Java engineer) Tj ET\nendstream\n%%EOF".to_vec();

    let output = parser
        .parse(
            ParseInput {
                path: PathBuf::from("text.pdf"),
                bytes,
            },
            ResourceBudget::new(Duration::from_secs(1)),
        )
        .expect("parse pdf");

    assert_eq!(output.status, ParseStatus::Parsed);
    assert!(output.text.contains("Java engineer"));
}

#[test]
fn scanned_pdf_is_marked_ocr_required() {
    let parser = PdfParser;
    let bytes = b"%PDF-1.4\n/Image XObject only\n%%EOF".to_vec();

    let output = parser
        .parse(
            ParseInput {
                path: PathBuf::from("scan.pdf"),
                bytes,
            },
            ResourceBudget::new(Duration::from_secs(1)),
        )
        .expect("parse pdf");

    assert_eq!(output.status, ParseStatus::OcrRequired);
    assert!(output.text.is_empty());
}

#[test]
fn pdf_parser_supports_pdf_extension() {
    let parser = PdfParser;
    let probe = FileProbe {
        path: PathBuf::from("resume.pdf"),
        extension: "pdf".to_owned(),
    };

    assert_eq!(parser.supports(&probe), SupportLevel::Supported);
}
