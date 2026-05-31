//! PDF parser behavior tests.

use parser_common::{ParseInput, Parser, SupportLevel};
use parser_pdf::PdfParser;

#[test]
fn extracts_plain_text_from_simple_text_layer_pdf() -> Result<(), Box<dyn std::error::Error>> {
    let input = ParseInput::new("synthetic.pdf", text_layer_pdf_bytes());

    let output = PdfParser.parse(&input)?;

    assert_eq!(output.support_level(), SupportLevel::TextLayer);
    assert_eq!(output.text(), Some("Synthetic text layer"));
    assert!(!output.ocr_required());
    Ok(())
}

#[test]
fn classifies_image_only_pdf_as_ocr_required() -> Result<(), Box<dyn std::error::Error>> {
    let input = ParseInput::new("synthetic.pdf", image_only_pdf_bytes());

    let output = PdfParser.parse(&input)?;

    assert_eq!(output.support_level(), SupportLevel::OcrRequired);
    assert_eq!(output.text(), None);
    assert!(output.ocr_required());
    Ok(())
}

#[test]
fn keeps_encoded_stream_without_raw_text_layer_unknown() -> Result<(), Box<dyn std::error::Error>> {
    let input = ParseInput::new("synthetic.pdf", encoded_stream_pdf_bytes());

    let output = PdfParser.parse(&input)?;

    assert_eq!(output.support_level(), SupportLevel::Unknown);
    assert_eq!(output.text(), None);
    assert!(!output.ocr_required());
    Ok(())
}

fn text_layer_pdf_bytes() -> Vec<u8> {
    b"%PDF-1.4
1 0 obj
<< /Type /Page /Contents 2 0 R /Resources << /Font << /F1 3 0 R >> >> >>
endobj
2 0 obj
<< /Length 48 >>
stream
BT
/F1 12 Tf
72 720 Td
(Synthetic text layer) Tj
ET
endstream
endobj
3 0 obj
<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>
endobj
%%EOF"
        .to_vec()
}

fn encoded_stream_pdf_bytes() -> Vec<u8> {
    b"%PDF-1.4
1 0 obj
<< /Type /Page /Contents 2 0 R >>
endobj
2 0 obj
<< /Length 24 /Filter /FlateDecode >>
stream
synthetic encoded stream bytes
endstream
endobj
%%EOF"
        .to_vec()
}

fn image_only_pdf_bytes() -> Vec<u8> {
    b"%PDF-1.4
1 0 obj
<< /Type /Page /Resources << /XObject << /Im1 2 0 R >> >> /Contents 3 0 R >>
endobj
2 0 obj
<< /Type /XObject /Subtype /Image /Width 10 /Height 10 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 4 >>
stream
0000
endstream
endobj
3 0 obj
<< /Length 24 >>
stream
q 10 0 0 10 0 0 cm /Im1 Do Q
endstream
endobj
%%EOF"
        .to_vec()
}
