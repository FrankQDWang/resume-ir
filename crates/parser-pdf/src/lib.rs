use lopdf::{
    content::{Content, Operation},
    Document as LoPdfDocument, Object, ObjectId,
};
use parser_common::{
    FileProbe, ParseBudget, ParseInput, ParseOutput, ParseStatus, Parser, ParserError,
    ResourceBudget, Result, SupportLevel,
};

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
        let page_text = extract_page_text(&document, page_no, page_id, budget)?;
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

fn normalize_lopdf_page_text(page_text: String) -> String {
    page_text.replace('\u{0c}', "\n").trim().to_owned()
}

fn extract_page_text(
    document: &LoPdfDocument,
    page_no: u32,
    page_id: ObjectId,
    budget: &ParseBudget,
) -> Result<String> {
    let direct_text = extract_direct_page_text(document, page_id, budget)?;
    budget.check_deadline()?;
    let lopdf_text = match document.extract_text(&[page_no]) {
        Ok(page_text) => normalize_lopdf_page_text(page_text),
        Err(_) => String::new(),
    };

    if direct_text.is_empty() {
        return Ok(lopdf_text);
    }
    if lopdf_text.is_empty() {
        return Ok(direct_text);
    }
    if lopdf_text.contains(&direct_text) {
        return Ok(lopdf_text);
    }
    if direct_text.is_ascii() {
        return Ok(lopdf_text);
    }

    Ok(direct_text)
}

fn extract_direct_page_text(
    document: &LoPdfDocument,
    page_id: ObjectId,
    budget: &ParseBudget,
) -> Result<String> {
    budget.check_deadline()?;
    let content_data = document
        .get_page_content(page_id)
        .map_err(|_| ParserError::corrupted("pdf structure is invalid"))?;
    budget.check_deadline()?;
    let content = Content::decode(&content_data)
        .map_err(|_| ParserError::corrupted("pdf structure is invalid"))?;
    let mut accumulator = TextAccumulator::default();

    for operation in content.operations {
        budget.check_deadline()?;
        if let Some(text_run) = extract_direct_text_run(&operation)? {
            accumulator.push(&text_run)?;
        }
    }

    Ok(accumulator.into_string())
}

fn extract_direct_text_run(operation: &Operation) -> Result<Option<String>> {
    match operation.operator.as_str() {
        "Tj" | "'" => decode_direct_text_operand(operation.operands.first()),
        "\"" => decode_direct_text_operand(operation.operands.get(2)),
        "TJ" => decode_direct_text_array(operation.operands.first()),
        _ => Ok(None),
    }
}

fn decode_direct_text_array(operand: Option<&Object>) -> Result<Option<String>> {
    let Some(Object::Array(items)) = operand else {
        return Ok(None);
    };

    let mut output = String::new();
    for item in items {
        if let Some(text) = decode_direct_text_object(item)? {
            output.push_str(&text);
        }
    }

    Ok((!output.trim().is_empty()).then_some(output))
}

fn decode_direct_text_operand(operand: Option<&Object>) -> Result<Option<String>> {
    operand.map_or(Ok(None), decode_direct_text_object)
}

fn decode_direct_text_object(object: &Object) -> Result<Option<String>> {
    let Object::String(bytes, _) = object else {
        return Ok(None);
    };
    decode_direct_text_bytes(bytes)
}

fn decode_direct_text_bytes(bytes: &[u8]) -> Result<Option<String>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    if let Some(text) = decode_utf16_with_bom(bytes)? {
        return Ok(Some(text));
    }

    Ok(std::str::from_utf8(bytes).ok().map(str::to_owned))
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
