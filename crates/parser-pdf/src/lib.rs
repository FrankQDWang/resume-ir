use lopdf::{content::Content, Document as LoPdfDocument, Encoding, Object, ObjectId};
use parser_common::{
    FileProbe, ParseBudget, ParseInput, ParseOutput, ParseStatus, Parser, ParserError,
    ResourceBudget, Result, SupportLevel,
};
use std::collections::BTreeMap;

const MAX_EXTRACTED_TEXT_CHARS: usize = 1_000_000;
const DEADLINE_CHECK_STRIDE: usize = 4096;

pub fn crate_name() -> &'static str {
    "parser-pdf"
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PdfParser;

impl Parser for PdfParser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel {
        match (probe.extension(), probe.has_pdf_header()) {
            (Some("pdf"), true) => SupportLevel::Supported,
            (Some("pdf"), false) | (_, true) => SupportLevel::Possible,
            _ => SupportLevel::Unsupported,
        }
    }

    fn parse(&self, input: ParseInput<'_>, budget: ResourceBudget) -> Result<ParseOutput> {
        let parse_budget = budget.begin(input.bytes().len())?;

        if self.supports(input.probe()) == SupportLevel::Unsupported {
            return Err(ParserError::unsupported(
                "pdf parser received unsupported probe",
            ));
        }

        let bytes = input.bytes();
        if !input.probe().has_pdf_header() {
            return Err(ParserError::corrupted("pdf header is missing"));
        }
        if !contains_bytes(bytes, b"%%EOF", &parse_budget)? {
            return Err(ParserError::corrupted("pdf EOF marker is missing"));
        }
        if contains_bytes(bytes, b"/Encrypt", &parse_budget)? {
            return Err(ParserError::encrypted("pdf declares an encrypted trailer"));
        }

        parse_budget.check_deadline()?;
        let extraction = extract_text_layer(bytes, &parse_budget)?;
        if has_text_signal(&extraction.text) {
            let mut output = ParseOutput::new(ParseStatus::TextLayer, extraction.text)
                .with_page_count(extraction.page_count);
            for page_text in extraction.pages {
                output = output.with_page_text(page_text.page_no, page_text.text);
            }
            return Ok(output);
        }

        Ok(ParseOutput::new(ParseStatus::OcrRequired, "").with_page_count(extraction.page_count))
    }
}

#[derive(Default)]
struct TextAccumulator {
    output: String,
    chars: usize,
}

impl TextAccumulator {
    fn push(&mut self, run: &str) -> Result<()> {
        let run = run.trim();
        if run.is_empty() {
            return Ok(());
        }

        let separator_chars = usize::from(!self.output.is_empty());
        let run_chars = run.chars().count();
        if self.chars + separator_chars + run_chars > MAX_EXTRACTED_TEXT_CHARS {
            return Err(ParserError::resource_exhausted(
                "pdf extracted text exceeds parser budget",
            ));
        }

        if !self.output.is_empty() {
            self.output.push('\n');
            self.chars += 1;
        }
        self.output.push_str(run);
        self.chars += run_chars;
        Ok(())
    }

    fn into_string(self) -> String {
        self.output
    }
}

struct LopdfTextExtraction {
    text: String,
    page_count: usize,
    pages: Vec<LopdfPageText>,
}

struct LopdfPageText {
    page_no: usize,
    text: String,
}

fn extract_text_layer(bytes: &[u8], budget: &ParseBudget) -> Result<LopdfTextExtraction> {
    budget.check_deadline()?;
    let document = LoPdfDocument::load_mem(bytes)
        .map_err(|_| ParserError::corrupted("pdf structure is invalid"))?;
    budget.check_deadline()?;

    let pages_by_number = document.get_pages();
    let page_count = pages_by_number.len().max(1);
    let mut accumulator = TextAccumulator::default();
    let mut pages = Vec::new();

    for (page_no, page_id) in pages_by_number {
        budget.check_deadline()?;
        let page_text = extract_page_text(&document, page_id, budget)?;
        if page_text.is_empty() {
            continue;
        }

        accumulator.push(&page_text)?;
        pages.push(LopdfPageText {
            page_no: page_no as usize,
            text: page_text,
        });
    }

    budget.check_deadline()?;
    Ok(LopdfTextExtraction {
        text: accumulator.into_string(),
        page_count,
        pages,
    })
}

fn extract_page_text(
    document: &LoPdfDocument,
    page_id: ObjectId,
    budget: &ParseBudget,
) -> Result<String> {
    let encodings = page_font_encodings(document, page_id)?;
    budget.check_deadline()?;
    let content_data = document
        .get_page_content(page_id)
        .map_err(|_| ParserError::corrupted("pdf structure is invalid"))?;
    budget.check_deadline()?;
    let content = Content::decode(&content_data)
        .map_err(|_| ParserError::corrupted("pdf structure is invalid"))?;
    let mut accumulator = TextAccumulator::default();
    let mut current_text = String::new();
    let mut current_encoding = None;

    for operation in &content.operations {
        budget.check_deadline()?;
        match operation.operator.as_str() {
            "Tf" => {
                flush_page_text(&mut current_text, &mut accumulator)?;
                current_encoding = operation
                    .operands
                    .first()
                    .and_then(|operand| operand.as_name().ok())
                    .and_then(|font_name| encodings.get(font_name));
            }
            "Tj" | "TJ" => {
                if let Some(encoding) = current_encoding {
                    collect_page_text(&mut current_text, encoding, &operation.operands)?;
                }
            }
            "'" => {
                if let Some(encoding) = current_encoding {
                    if !current_text.ends_with('\n') {
                        current_text.push('\n');
                    }
                    collect_page_text(&mut current_text, encoding, &operation.operands)?;
                }
            }
            "\"" => {
                if let Some(encoding) = current_encoding {
                    if !current_text.ends_with('\n') {
                        current_text.push('\n');
                    }
                    if let Some(string_operand) = operation.operands.get(2) {
                        collect_page_text(
                            &mut current_text,
                            encoding,
                            std::slice::from_ref(string_operand),
                        )?;
                    }
                }
            }
            "T*" if !current_text.ends_with('\n') => current_text.push('\n'),
            "T*" => {}
            "ET" if !current_text.ends_with('\n') => current_text.push('\n'),
            "ET" => {}
            _ => {}
        }
    }

    flush_page_text(&mut current_text, &mut accumulator)?;
    Ok(accumulator.into_string())
}

fn page_font_encodings<'a>(
    document: &'a LoPdfDocument,
    page_id: ObjectId,
) -> Result<BTreeMap<Vec<u8>, Encoding<'a>>> {
    let fonts = document
        .get_page_fonts(page_id)
        .map_err(|_| ParserError::corrupted("pdf structure is invalid"))?;
    let mut encodings = BTreeMap::new();
    for (name, font) in fonts {
        let encoding = font
            .get_font_encoding(document)
            .map_err(|_| ParserError::corrupted("pdf structure is invalid"))?;
        encodings.insert(name, encoding);
    }
    Ok(encodings)
}

fn flush_page_text(buffer: &mut String, accumulator: &mut TextAccumulator) -> Result<()> {
    let page_text = buffer.trim();
    if !page_text.is_empty() {
        accumulator.push(page_text)?;
        buffer.clear();
    } else if !buffer.is_empty() {
        buffer.clear();
    }
    Ok(())
}

fn collect_page_text(
    text: &mut String,
    encoding: &Encoding<'_>,
    operands: &[Object],
) -> Result<()> {
    for operand in operands {
        match operand {
            Object::String(bytes, _) => text.push_str(&decode_text_bytes(encoding, bytes)?),
            Object::Array(items) => {
                collect_page_text(text, encoding, items)?;
                text.push(' ');
            }
            Object::Integer(value) if *value < -100 => text.push(' '),
            _ => {}
        }
    }
    Ok(())
}

fn decode_text_bytes(encoding: &Encoding<'_>, bytes: &[u8]) -> Result<String> {
    if bytes.is_empty() {
        return Ok(String::new());
    }
    if let Some(text) = decode_utf16_with_bom(bytes)? {
        return Ok(text);
    }

    LoPdfDocument::decode_text(encoding, bytes)
        .map_err(|_| ParserError::corrupted("pdf structure is invalid"))
}

fn decode_utf16_with_bom(bytes: &[u8]) -> Result<Option<String>> {
    let Some((&first, rest)) = bytes.split_first() else {
        return Ok(None);
    };
    let Some((&second, payload)) = rest.split_first() else {
        return Ok(None);
    };
    let endianness = match (first, second) {
        (0xFE, 0xFF) => Utf16Endianness::Big,
        (0xFF, 0xFE) => Utf16Endianness::Little,
        _ => return Ok(None),
    };

    if payload.len() % 2 != 0 {
        return Err(ParserError::corrupted(
            "pdf utf-16 text run has odd byte length",
        ));
    }

    let code_units = payload
        .chunks_exact(2)
        .map(|chunk| match endianness {
            Utf16Endianness::Big => u16::from_be_bytes([chunk[0], chunk[1]]),
            Utf16Endianness::Little => u16::from_le_bytes([chunk[0], chunk[1]]),
        })
        .collect::<Vec<_>>();
    String::from_utf16(&code_units)
        .map(Some)
        .map_err(|_| ParserError::corrupted("pdf utf-16 text run is invalid"))
}

#[derive(Clone, Copy)]
enum Utf16Endianness {
    Big,
    Little,
}

fn has_text_signal(text: &str) -> bool {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .count()
        >= 3
}

fn contains_bytes(bytes: &[u8], needle: &[u8], budget: &ParseBudget) -> Result<bool> {
    Ok(find_bytes(bytes, needle, 0, budget)?.is_some())
}

fn find_bytes(
    bytes: &[u8],
    needle: &[u8],
    start: usize,
    budget: &ParseBudget,
) -> Result<Option<usize>> {
    budget.check_deadline()?;

    if needle.is_empty() || start >= bytes.len() {
        return Ok(None);
    }

    let Some(max_start) = bytes.len().checked_sub(needle.len()) else {
        return Ok(None);
    };
    if start > max_start {
        return Ok(None);
    }

    for index in start..=max_start {
        if (index - start).is_multiple_of(DEADLINE_CHECK_STRIDE) {
            budget.check_deadline()?;
        }

        if bytes[index..].starts_with(needle) {
            return Ok(Some(index));
        }
    }

    budget.check_deadline()?;
    Ok(None)
}
