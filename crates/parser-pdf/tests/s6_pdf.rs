use core_domain::DocumentStatus;
use parser_common::{
    ParseInput, ParseStatus, Parser, ParserErrorKind, ResourceBudget, SupportLevel,
};
use parser_pdf::PdfParser;

#[test]
fn exposes_parser_pdf_crate_identity() {
    assert_eq!(parser_pdf::crate_name(), "parser-pdf");
}

#[test]
fn text_layer_pdf_returns_text_layer_status_and_extracted_signal() {
    let parser = PdfParser;
    let bytes = text_layer_pdf_bytes();
    let input = ParseInput::from_bytes(Some("pdf"), bytes);

    assert_eq!(parser.supports(input.probe()), SupportLevel::Supported);

    let output = parser.parse(input, ResourceBudget::default()).unwrap();

    assert_eq!(output.status(), ParseStatus::TextLayer);
    assert_eq!(output.document_status(), DocumentStatus::TextExtracted);
    assert!(output.text().contains("Synthetic PDF Text Layer"));
    assert_eq!(output.page_count(), Some(1));
    assert!(!format!("{output:?}").contains("Synthetic PDF Text Layer"));
}

#[test]
fn utf16be_hex_text_layer_pdf_returns_text_layer_status_and_extracted_signal() {
    let parser = PdfParser;
    let input = ParseInput::from_bytes(Some("pdf"), utf16be_hex_text_layer_pdf_bytes());

    let output = parser.parse(input, ResourceBudget::default()).unwrap();

    let expected = "\u{4E2D}\u{6587}\u{7B80}\u{5386}";
    assert_eq!(output.status(), ParseStatus::TextLayer);
    assert_eq!(output.document_status(), DocumentStatus::TextExtracted);
    assert!(output.text().contains(expected));
    assert_eq!(output.page_count(), Some(1));
    assert!(!format!("{output:?}").contains(expected));
}

#[test]
fn scanned_image_pdf_returns_ocr_required_without_running_ocr() {
    let parser = PdfParser;
    let output = parser
        .parse(
            ParseInput::from_bytes(Some("pdf"), scanned_pdf_bytes()),
            ResourceBudget::default(),
        )
        .unwrap();

    assert_eq!(output.status(), ParseStatus::OcrRequired);
    assert_eq!(output.document_status(), DocumentStatus::OcrRequired);
    assert_eq!(output.text(), "");
    assert_eq!(output.page_count(), Some(1));
    assert!(!format!("{output:?}").contains("image bytes"));
}

#[test]
fn corrupted_pdf_returns_corrupted_error() {
    let parser = PdfParser;
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("pdf"), b"%PDF-1.4\nmissing eof"),
            ResourceBudget::default(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Corrupted);
}

#[test]
fn pdf_parser_enforces_input_byte_budget() {
    let parser = PdfParser;
    let bytes = text_layer_pdf_bytes();
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("pdf"), bytes),
            ResourceBudget::default().with_max_bytes(bytes.len() - 1),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::ResourceExhausted);
}

#[test]
fn pdf_parser_enforces_runtime_timeout() {
    let parser = PdfParser;
    let bytes = many_text_runs_pdf_bytes(10_000);
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("pdf"), &bytes),
            ResourceBudget::default().with_timeout(std::time::Duration::from_nanos(1)),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Timeout);
}

#[test]
fn pdf_parser_enforces_runtime_timeout_without_text_layer_operator() {
    let parser = PdfParser;
    let bytes = large_scanned_pdf_without_text_layer_bytes(1_000_000);
    let error = parser
        .parse(
            ParseInput::from_bytes(Some("pdf"), &bytes),
            ResourceBudget::default().with_timeout(std::time::Duration::from_nanos(1)),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Timeout);
}

fn text_layer_pdf_bytes() -> &'static [u8] {
    b"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >> endobj
4 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
5 0 obj << /Length 58 >> stream
BT /F1 12 Tf 72 720 Td (Synthetic PDF Text Layer) Tj ET
endstream endobj
%%EOF"
}

fn utf16be_hex_text_layer_pdf_bytes() -> &'static [u8] {
    b"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >> endobj
4 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
5 0 obj << /Length 47 >> stream
BT /F1 12 Tf 72 720 Td <FEFF4E2D65877B805386> Tj ET
endstream endobj
%%EOF"
}

fn scanned_pdf_bytes() -> &'static [u8] {
    b"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >> endobj
4 0 obj << /Type /XObject /Subtype /Image /Width 100 /Height 100 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 11 >> stream
image bytes
endstream endobj
5 0 obj << /Length 24 >> stream
q 100 0 0 100 0 0 cm /Im1 Do Q
endstream endobj
%%EOF"
}

fn many_text_runs_pdf_bytes(run_count: usize) -> Vec<u8> {
    let mut stream = String::new();
    for index in 0..run_count {
        stream.push_str("BT (Synthetic run ");
        stream.push_str(&index.to_string());
        stream.push_str(") Tj ET\n");
    }

    format!(
        "%PDF-1.4\n\
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
3 0 obj << /Type /Page /Parent 2 0 R /Contents 4 0 R >> endobj\n\
4 0 obj << /Length {} >> stream\n{}endstream endobj\n\
%%EOF",
        stream.len(),
        stream
    )
    .into_bytes()
}

fn large_scanned_pdf_without_text_layer_bytes(payload_len: usize) -> Vec<u8> {
    let payload = "0".repeat(payload_len);
    format!(
        "%PDF-1.4\n\
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
3 0 obj << /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >> endobj\n\
4 0 obj << /Type /XObject /Subtype /Image /Width 100 /Height 100 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length {} >> stream\n\
{}\
endstream endobj\n\
5 0 obj << /Length 24 >> stream\n\
q 100 0 0 100 0 0 cm /Im1 Do Q\n\
endstream endobj\n\
%%EOF",
        payload.len(),
        payload
    )
    .into_bytes()
}
