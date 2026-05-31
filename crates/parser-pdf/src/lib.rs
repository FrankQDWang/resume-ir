//! Lightweight PDF text-layer detection.

use parser_common::{ParseInput, ParseOutput, Parser, ParserError, ParserErrorKind, SupportLevel};

/// Parser that classifies PDFs by text-layer presence.
#[derive(Clone, Copy, Debug, Default)]
pub struct PdfParser;

impl Parser for PdfParser {
    fn parse(&self, input: &ParseInput) -> Result<ParseOutput, ParserError> {
        if !input.bytes().starts_with(b"%PDF-") {
            return Err(ParserError::new(
                ParserErrorKind::CorruptedDocument,
                false,
                "PDF document could not be parsed.",
                "pdf header missing",
                "parser-pdf",
            ));
        }

        let support_level = if has_text_layer(input.bytes()) {
            SupportLevel::TextLayer
        } else if has_encoded_stream(input.bytes()) {
            SupportLevel::Unknown
        } else if has_image_xobject(input.bytes()) {
            SupportLevel::OcrRequired
        } else {
            SupportLevel::Unknown
        };

        Ok(ParseOutput::classification(
            input.source_name().to_owned(),
            support_level,
        ))
    }
}

fn has_encoded_stream(bytes: &[u8]) -> bool {
    find_token(bytes, b"/Filter").is_some()
}

fn has_image_xobject(bytes: &[u8]) -> bool {
    find_token(bytes, b"/Subtype").is_some() && find_token(bytes, b"/Image").is_some()
}

fn has_text_layer(bytes: &[u8]) -> bool {
    let mut index = 0;
    while let Some(text_start) = find_token(&bytes[index..], b"BT") {
        let object_start = index + text_start + b"BT".len();
        if let Some(text_end) = find_token(&bytes[object_start..], b"ET") {
            let object = &bytes[object_start..object_start + text_end];
            if contains_text_show_operator(object) {
                return true;
            }
            index = object_start + text_end + b"ET".len();
        } else {
            break;
        }
    }

    false
}

fn contains_text_show_operator(bytes: &[u8]) -> bool {
    find_token(bytes, b"Tj").is_some()
        || find_token(bytes, b"TJ").is_some()
        || find_token(bytes, b"'").is_some()
        || find_token(bytes, b"\"").is_some()
}

fn find_token(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
        .filter(|position| {
            let start = *position;
            let end = start + needle.len();
            is_pdf_boundary(haystack.get(start.wrapping_sub(1)).copied())
                && is_pdf_boundary(haystack.get(end).copied())
        })
}

fn is_pdf_boundary(byte: Option<u8>) -> bool {
    match byte {
        None => true,
        Some(
            b'\0' | b'\t' | b'\n' | b'\r' | b'\x0c' | b' ' | b'(' | b')' | b'<' | b'>' | b'['
            | b']' | b'/' | b'%',
        ) => true,
        Some(_) => false,
    }
}
