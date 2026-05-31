//! Basic `.docx` text extraction from `word/document.xml`.

use parser_common::{ParseInput, ParseOutput, Parser, ParserError, ParserErrorKind, SupportLevel};
use quick_xml::events::Event;
use quick_xml::Reader;
use std::io::{Cursor, Read};
use zip::ZipArchive;

/// Parser for basic `.docx` text extraction.
#[derive(Clone, Copy, Debug, Default)]
pub struct DocxParser;

impl Parser for DocxParser {
    fn parse(&self, input: &ParseInput) -> Result<ParseOutput, ParserError> {
        let document_xml = read_document_xml(input.bytes())?;
        let text = extract_text_nodes(&document_xml)?;
        Ok(ParseOutput::from_text(
            input.source_name().to_owned(),
            text,
            SupportLevel::Text,
        ))
    }
}

fn read_document_xml(bytes: &[u8]) -> Result<String, ParserError> {
    let cursor = Cursor::new(bytes);
    let mut archive = ZipArchive::new(cursor).map_err(|err| {
        ParserError::new(
            ParserErrorKind::CorruptedDocument,
            false,
            "Document could not be parsed.",
            format!("docx zip open failed: {err}"),
            "parser-docx",
        )
    })?;

    let mut document = archive.by_name("word/document.xml").map_err(|err| {
        ParserError::new(
            ParserErrorKind::CorruptedDocument,
            false,
            "Document could not be parsed.",
            format!("docx document.xml missing or unreadable: {err}"),
            "parser-docx",
        )
    })?;

    let mut xml = String::new();
    document.read_to_string(&mut xml).map_err(|err| {
        ParserError::new(
            ParserErrorKind::CorruptedDocument,
            false,
            "Document could not be parsed.",
            format!("docx document.xml utf-8 read failed: {err}"),
            "parser-docx",
        )
    })?;

    Ok(xml)
}

fn extract_text_nodes(xml: &str) -> Result<String, ParserError> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(false);

    let mut output = String::new();
    let mut in_text = false;
    let mut paragraph_has_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let name = element.name();
                let name = name.as_ref();
                if is_word_text_node(name) {
                    in_text = true;
                }
            }
            Ok(Event::Text(text)) if in_text => {
                let decoded = text.unescape().map_err(|err| {
                    ParserError::new(
                        ParserErrorKind::CorruptedDocument,
                        false,
                        "Document could not be parsed.",
                        format!("docx text node decode failed: {err}"),
                        "parser-docx",
                    )
                })?;
                if !decoded.is_empty() {
                    output.push_str(&decoded);
                    paragraph_has_text = true;
                }
            }
            Ok(Event::End(element)) => {
                let name = element.name();
                let name = name.as_ref();
                if is_word_text_node(name) {
                    in_text = false;
                } else if is_word_paragraph_node(name) && paragraph_has_text {
                    trim_trailing_spaces(&mut output);
                    output.push('\n');
                    paragraph_has_text = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(ParserError::new(
                    ParserErrorKind::CorruptedDocument,
                    false,
                    "Document could not be parsed.",
                    format!("docx xml parse failed: {err}"),
                    "parser-docx",
                ));
            }
            _ => {}
        }
    }

    if output.ends_with('\n') {
        output.pop();
    }

    Ok(output)
}

fn is_word_text_node(name: &[u8]) -> bool {
    name == b"w:t" || name == b"t"
}

fn is_word_paragraph_node(name: &[u8]) -> bool {
    name == b"w:p" || name == b"p"
}

fn trim_trailing_spaces(output: &mut String) {
    while output.ends_with(' ') || output.ends_with('\t') {
        output.pop();
    }
}
