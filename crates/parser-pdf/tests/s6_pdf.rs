use core_domain::DocumentStatus;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use parser_common::{
    ParseInput, ParseStatus, Parser, ParserErrorKind, ResourceBudget, SupportLevel,
};
use parser_pdf::PdfParser;
use std::io::Write;

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
fn utf16be_hex_text_layer_pdf_with_odd_utf16_length_returns_corrupted_error() {
    let parser = PdfParser;
    let error = parser
        .parse(
            ParseInput::from_bytes(
                Some("pdf"),
                utf16be_hex_text_layer_pdf_with_odd_utf16_length_bytes(),
            ),
            ResourceBudget::default(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Corrupted);
    assert_eq!(
        error.diagnostic_message(),
        "pdf utf-16 text run has odd byte length"
    );
}

#[test]
fn utf16be_hex_text_layer_pdf_with_invalid_utf16_surrogate_returns_corrupted_error() {
    let parser = PdfParser;
    let error = parser
        .parse(
            ParseInput::from_bytes(
                Some("pdf"),
                utf16be_hex_text_layer_pdf_with_invalid_utf16_surrogate_bytes(),
            ),
            ResourceBudget::default(),
        )
        .unwrap_err();

    assert_eq!(error.kind(), ParserErrorKind::Corrupted);
    assert_eq!(error.diagnostic_message(), "pdf utf-16 text run is invalid");
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
fn compressed_text_stream_pdf_returns_text_layer_status_and_extracted_signal() {
    let parser = PdfParser;
    let bytes = compressed_text_stream_pdf_bytes();
    let input = ParseInput::from_bytes(Some("pdf"), &bytes);

    let output = parser.parse(input, ResourceBudget::default()).unwrap();

    assert_eq!(output.status(), ParseStatus::TextLayer);
    assert_eq!(output.document_status(), DocumentStatus::TextExtracted);
    assert!(output.text().contains("Compressed PDF Text Layer"));
    assert_eq!(output.page_count(), Some(1));
}

#[test]
fn tounicode_cmap_pdf_returns_text_layer_status_and_extracted_signal() {
    let parser = PdfParser;
    let bytes = tounicode_cmap_pdf_bytes();
    let input = ParseInput::from_bytes(Some("pdf"), &bytes);

    let output = parser.parse(input, ResourceBudget::default()).unwrap();

    assert_eq!(output.status(), ParseStatus::TextLayer);
    assert_eq!(output.document_status(), DocumentStatus::TextExtracted);
    assert!(output.text().contains("中文简历"));
    assert_eq!(output.page_count(), Some(1));
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

fn utf16be_hex_text_layer_pdf_with_odd_utf16_length_bytes() -> &'static [u8] {
    b"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >> endobj
4 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
5 0 obj << /Length 38 >> stream
BT /F1 12 Tf 72 720 Td <FEFF4E2D6> Tj ET
endstream endobj
%%EOF"
}

fn utf16be_hex_text_layer_pdf_with_invalid_utf16_surrogate_bytes() -> &'static [u8] {
    b"%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >> endobj
4 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> endobj
5 0 obj << /Length 43 >> stream
BT /F1 12 Tf 72 720 Td <FEFFD8000061> Tj ET
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

fn compressed_text_stream_pdf_bytes() -> Vec<u8> {
    let plain_text = b"BT /F1 12 Tf 72 720 Td (Compressed PDF Text Layer) Tj ET\n";
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(plain_text).unwrap();
    let compressed = encoder.finish().unwrap();

    build_valid_pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        [
            format!(
                "<< /Length {} /Filter /FlateDecode >>\nstream\n",
                compressed.len()
            )
            .into_bytes(),
            compressed,
            b"\nendstream".to_vec(),
        ]
        .concat(),
    ])
}

fn tounicode_cmap_pdf_bytes() -> Vec<u8> {
    let cmap = br"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0001> <0004>
endcodespacerange
4 beginbfchar
<0001> <4E2D>
<0002> <6587>
<0003> <7B80>
<0004> <5386>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end
";
    let content = b"BT /F1 12 Tf 72 720 Td <0001000200030004> Tj ET\n";

    build_valid_pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 7 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type0 /BaseFont /TestFont /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 6 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /TestFont /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 8 0 R /DW 1000 /W [1 [1000 1000]] >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", cmap.len()).into_bytes(),
            cmap.to_vec(),
            b"endstream".to_vec(),
        ]
        .concat(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
            content.to_vec(),
            b"endstream".to_vec(),
        ]
        .concat(),
        b"<< /Type /FontDescriptor /FontName /TestFont /Flags 4 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>".to_vec(),
    ])
}

fn build_valid_pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());

    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        if !object.ends_with(b"\n") {
            pdf.push(b'\n');
        }
        pdf.extend_from_slice(b"endobj\n");
    }

    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{}\n%%EOF",
            objects.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );

    pdf
}
