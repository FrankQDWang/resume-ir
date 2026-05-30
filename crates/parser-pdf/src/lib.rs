use parser_common::{
    FileProbe, ParseInput, ParseOutput, ParseResult, ParseStatus, Parser, ResourceBudget,
    SupportLevel,
};

pub struct PdfParser;

impl Parser for PdfParser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel {
        if probe.extension.eq_ignore_ascii_case("pdf") {
            SupportLevel::Supported
        } else {
            SupportLevel::Unsupported
        }
    }

    fn parse(&self, input: ParseInput, _budget: ResourceBudget) -> ParseResult<ParseOutput> {
        if has_text_layer(&input.bytes) {
            Ok(ParseOutput {
                text: extract_literal_text(&input.bytes),
                status: ParseStatus::Parsed,
                page_count: None,
                warnings: Vec::new(),
            })
        } else {
            Ok(ParseOutput {
                text: String::new(),
                status: ParseStatus::OcrRequired,
                page_count: None,
                warnings: vec!["OCR_REQUIRED".to_owned()],
            })
        }
    }
}

pub fn has_text_layer(bytes: &[u8]) -> bool {
    let content = String::from_utf8_lossy(bytes);
    content.contains("BT")
        && content.contains("ET")
        && (content.contains("Tj") || content.contains("TJ"))
}

pub fn extract_literal_text(bytes: &[u8]) -> String {
    let content = String::from_utf8_lossy(bytes);
    let mut text = Vec::new();

    for segment in content.split("Tj") {
        if let (Some(start), Some(end)) = (segment.rfind('('), segment.rfind(')')) {
            if start < end {
                text.push(segment[start + 1..end].to_owned());
            }
        }
    }

    text.join("\n")
}

#[must_use]
pub fn crate_name() -> &'static str {
    "parser-pdf"
}
