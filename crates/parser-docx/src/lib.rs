use std::io::{Cursor, Read};

use parser_common::{
    FileProbe, ParseBudget, ParseInput, ParseOutput, ParseStatus, Parser, ParserError,
    ResourceBudget, Result, SupportLevel,
};
use quick_xml::events::Event;
use quick_xml::Reader;
use zip::result::ZipError;
use zip::ZipArchive;

const DOCUMENT_XML_PATH: &str = "word/document.xml";
const MAX_ZIP_ENTRIES: usize = 256;
const MAX_DOCUMENT_XML_BYTES: u64 = 2 * 1024 * 1024;
const MAX_EXTRACTED_CHARS: usize = 1_000_000;

pub fn crate_name() -> &'static str {
    "parser-docx"
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DocxParser;

impl Parser for DocxParser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel {
        match (probe.extension(), probe.has_zip_header()) {
            (Some("docx"), true) => SupportLevel::Supported,
            (Some("docx"), false) | (_, true) => SupportLevel::Possible,
            _ => SupportLevel::Unsupported,
        }
    }

    fn parse(&self, input: ParseInput<'_>, budget: ResourceBudget) -> Result<ParseOutput> {
        let parse_budget = budget.begin(input.bytes().len())?;

        if self.supports(input.probe()) == SupportLevel::Unsupported {
            return Err(ParserError::unsupported(
                "docx parser received unsupported probe",
            ));
        }

        parse_budget.check_deadline()?;
        let mut archive = ZipArchive::new(Cursor::new(input.bytes())).map_err(map_zip_error)?;
        if archive.len() > MAX_ZIP_ENTRIES {
            return Err(ParserError::resource_exhausted(
                "docx zip entry count exceeds parser budget",
            ));
        }

        let mut document = archive.by_name(DOCUMENT_XML_PATH).map_err(map_zip_error)?;
        if document.encrypted() {
            return Err(ParserError::encrypted(
                "docx word/document.xml is encrypted",
            ));
        }
        if document.size() > MAX_DOCUMENT_XML_BYTES {
            return Err(ParserError::resource_exhausted(
                "docx document xml exceeds parser budget",
            ));
        }

        parse_budget.check_deadline()?;
        let mut xml = String::new();
        document
            .by_ref()
            .take(MAX_DOCUMENT_XML_BYTES + 1)
            .read_to_string(&mut xml)
            .map_err(|_| ParserError::corrupted("docx word/document.xml is not valid utf-8"))?;
        if xml.len() as u64 > MAX_DOCUMENT_XML_BYTES {
            return Err(ParserError::resource_exhausted(
                "docx document xml exceeds parser budget",
            ));
        }

        let text = extract_document_xml_text(&xml, &parse_budget)?;

        Ok(ParseOutput::new(ParseStatus::TextExtracted, text))
    }
}

fn map_zip_error(error: ZipError) -> ParserError {
    match error {
        ZipError::InvalidPassword | ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED) => {
            ParserError::encrypted("docx archive requires a password")
        }
        ZipError::Io(_)
        | ZipError::InvalidArchive(_)
        | ZipError::UnsupportedArchive(_)
        | ZipError::FileNotFound => ParserError::corrupted("docx archive is missing required XML"),
        _ => ParserError::corrupted("docx archive uses an unknown unsupported structure"),
    }
}

fn extract_document_xml_text(xml: &str, budget: &ParseBudget) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.trim_text(false);

    let mut buffer = Vec::new();
    let mut paragraphs = Vec::new();
    let mut paragraph = String::new();
    let mut total_chars = 0_usize;
    let mut in_paragraph = false;
    let mut in_text = false;

    loop {
        budget.check_deadline()?;

        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) => match local_name(element.name().as_ref()) {
                b"p" => {
                    in_paragraph = true;
                    paragraph.clear();
                }
                b"t" if in_paragraph => in_text = true,
                _ => {}
            },
            Ok(Event::Empty(element)) if in_paragraph => {
                match local_name(element.name().as_ref()) {
                    b"tab" => push_bounded(&mut paragraph, "\t", &mut total_chars)?,
                    b"br" => push_bounded(&mut paragraph, "\n", &mut total_chars)?,
                    _ => {}
                }
            }
            Ok(Event::Text(text)) if in_text => {
                let unescaped = text
                    .unescape()
                    .map_err(|_| ParserError::corrupted("docx text node has invalid escaping"))?;
                push_bounded(&mut paragraph, &unescaped, &mut total_chars)?;
            }
            Ok(Event::CData(text)) if in_text => {
                let text = String::from_utf8_lossy(text.as_ref());
                push_bounded(&mut paragraph, &text, &mut total_chars)?;
            }
            Ok(Event::End(element)) => match local_name(element.name().as_ref()) {
                b"t" => in_text = false,
                b"p" => {
                    if !paragraph.trim().is_empty() {
                        paragraphs.push(trim_trailing_line_breaks(&paragraph).to_string());
                    }
                    in_paragraph = false;
                    in_text = false;
                    paragraph.clear();
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => {
                return Err(ParserError::corrupted(
                    "docx word/document.xml is not well-formed XML",
                ));
            }
        }

        buffer.clear();
    }

    Ok(paragraphs.join("\n"))
}

fn push_bounded(target: &mut String, text: &str, total_chars: &mut usize) -> Result<()> {
    let text_chars = text.chars().count();
    if *total_chars + text_chars > MAX_EXTRACTED_CHARS {
        return Err(ParserError::resource_exhausted(
            "docx extracted text exceeds parser budget",
        ));
    }

    *total_chars += text_chars;
    target.push_str(text);
    Ok(())
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

fn trim_trailing_line_breaks(value: &str) -> &str {
    value.trim_end_matches(['\n', '\r'])
}
