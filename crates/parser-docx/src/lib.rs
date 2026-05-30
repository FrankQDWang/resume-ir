use parser_common::{
    FileProbe, ParseInput, ParseOutput, ParseResult, ParseStatus, Parser, ParserError,
    ResourceBudget, SupportLevel,
};
use quick_xml::Reader;
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use std::io::{Cursor, Read};
use zip::ZipArchive;

pub struct DocxParser;

impl Parser for DocxParser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel {
        if probe.extension.eq_ignore_ascii_case("docx") {
            SupportLevel::Supported
        } else {
            SupportLevel::Unsupported
        }
    }

    fn parse(&self, input: ParseInput, _budget: ResourceBudget) -> ParseResult<ParseOutput> {
        let text = extract_docx_text(&input.bytes)?;
        Ok(ParseOutput {
            text,
            status: ParseStatus::Parsed,
            page_count: None,
            warnings: Vec::new(),
        })
    }
}

pub fn extract_docx_text(bytes: &[u8]) -> ParseResult<String> {
    let reader = Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader).map_err(|error| {
        ParserError::corrupted(
            "DOCX file is corrupted or not a valid archive.",
            error.to_string(),
        )
    })?;
    let mut document = archive.by_name("word/document.xml").map_err(|error| {
        ParserError::corrupted("DOCX document body is missing.", error.to_string())
    })?;
    let mut xml = String::new();
    document
        .read_to_string(&mut xml)
        .map_err(|error| ParserError::io("DOCX text could not be read.", error.to_string()))?;

    extract_text_from_document_xml(&xml)
}

fn extract_text_from_document_xml(xml: &str) -> ParseResult<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut text = String::new();
    let mut in_text_node = false;

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) if element.name().as_ref() == b"w:t" => {
                in_text_node = true;
            }
            Ok(Event::End(element)) if element.name().as_ref() == b"w:t" => {
                in_text_node = false;
            }
            Ok(Event::Text(value)) if in_text_node => {
                let decoded = value.decode().map_err(|error| {
                    ParserError::corrupted("DOCX XML text is invalid.", error.to_string())
                })?;
                let unescaped = unescape(&decoded).map_err(|error| {
                    ParserError::corrupted("DOCX XML text is invalid.", error.to_string())
                })?;
                text.push_str(&unescaped);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                return Err(ParserError::corrupted(
                    "DOCX XML could not be parsed.",
                    error.to_string(),
                ));
            }
        }
        buffer.clear();
    }

    Ok(text)
}

#[must_use]
pub fn crate_name() -> &'static str {
    "parser-docx"
}
