use parser_common::{
    FileProbe, ParseBudget, ParseInput, ParseOutput, ParseStatus, Parser, ParserError,
    ResourceBudget, Result, SupportLevel,
};

const MAX_EXTRACTED_TEXT_CHARS: usize = 1_000_000;
const MAX_PDF_TEXT_RUN_BYTES: usize = 128 * 1024;
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
        let text = extract_text_layer(bytes, &parse_budget)?;
        let page_count = count_pdf_pages(bytes, &parse_budget)?.unwrap_or(1);
        if !has_text_signal(&text) {
            return Ok(ParseOutput::new(ParseStatus::OcrRequired, "").with_page_count(page_count));
        }

        Ok(ParseOutput::new(ParseStatus::TextLayer, text.clone())
            .with_page_count(page_count)
            .with_page_text(1, text))
    }
}

fn extract_text_layer(bytes: &[u8], budget: &ParseBudget) -> Result<String> {
    let mut accumulator = TextAccumulator::default();
    let mut search_from = 0;

    while let Some(start) = find_operator(bytes, b"BT", search_from, budget)? {
        budget.check_deadline()?;

        let block_start = start + 2;
        let Some(end) = find_operator(bytes, b"ET", block_start, budget)? else {
            break;
        };

        parse_text_block(&bytes[block_start..end], &mut accumulator, budget)?;
        search_from = end + 2;
    }

    Ok(accumulator.into_string())
}

fn parse_text_block(
    block: &[u8],
    accumulator: &mut TextAccumulator,
    budget: &ParseBudget,
) -> Result<()> {
    let mut index = 0;

    while index < block.len() {
        budget.check_deadline()?;

        match block[index] {
            b'(' => {
                if let Some((text, next_index)) = parse_literal_string(block, index, budget)? {
                    if has_visible_text(&text) {
                        accumulator.push(&text)?;
                    }
                    index = next_index;
                } else {
                    index += 1;
                }
            }
            b'<' if block.get(index + 1) != Some(&b'<') => {
                if let Some((text, next_index)) = parse_hex_string(block, index, budget)? {
                    if has_visible_text(&text) {
                        accumulator.push(&text)?;
                    }
                    index = next_index;
                } else {
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }

    Ok(())
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

fn parse_literal_string(
    block: &[u8],
    start: usize,
    budget: &ParseBudget,
) -> Result<Option<(String, usize)>> {
    let mut output = Vec::new();
    let mut index = start + 1;
    let mut depth = 1_usize;

    while index < block.len() {
        if (index - start).is_multiple_of(DEADLINE_CHECK_STRIDE) {
            budget.check_deadline()?;
        }

        match block[index] {
            b'\\' => {
                index += 1;
                if index >= block.len() {
                    return Err(ParserError::corrupted("pdf text literal is unterminated"));
                }

                match block[index] {
                    b'n' => push_text_run_byte(&mut output, b'\n')?,
                    b'r' => push_text_run_byte(&mut output, b'\r')?,
                    b't' => push_text_run_byte(&mut output, b'\t')?,
                    b'b' => push_text_run_byte(&mut output, 0x08)?,
                    b'f' => push_text_run_byte(&mut output, 0x0c)?,
                    b'(' | b')' | b'\\' => push_text_run_byte(&mut output, block[index])?,
                    b'\n' => {}
                    b'\r' => {
                        if block.get(index + 1) == Some(&b'\n') {
                            index += 1;
                        }
                    }
                    b'0'..=b'7' => {
                        let (value, next_index) = parse_octal_escape(block, index);
                        push_text_run_byte(&mut output, value)?;
                        index = next_index;
                        continue;
                    }
                    other => push_text_run_byte(&mut output, other)?,
                }
            }
            b'(' => {
                depth += 1;
                push_text_run_byte(&mut output, b'(')?;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return decode_text_run_bytes(
                        &output,
                        TextRunEncodingFallback::StrictUtf8,
                        "pdf text literal is not utf-8",
                    )
                    .map(|text| Some((text, index + 1)));
                }
                push_text_run_byte(&mut output, b')')?;
            }
            byte => push_text_run_byte(&mut output, byte)?,
        }

        index += 1;
    }

    Err(ParserError::corrupted("pdf text literal is unterminated"))
}

fn push_text_run_byte(output: &mut Vec<u8>, byte: u8) -> Result<()> {
    if output.len() >= MAX_PDF_TEXT_RUN_BYTES {
        return Err(ParserError::resource_exhausted(
            "pdf text run exceeds parser budget",
        ));
    }

    output.push(byte);
    Ok(())
}

fn parse_octal_escape(block: &[u8], start: usize) -> (u8, usize) {
    let mut value = 0_u16;
    let mut index = start;
    let mut digits = 0;

    while index < block.len() && digits < 3 {
        match block[index] {
            digit @ b'0'..=b'7' => {
                value = (value * 8) + u16::from(digit - b'0');
                index += 1;
                digits += 1;
            }
            _ => break,
        }
    }

    (value.min(u16::from(u8::MAX)) as u8, index)
}

fn parse_hex_string(
    block: &[u8],
    start: usize,
    budget: &ParseBudget,
) -> Result<Option<(String, usize)>> {
    let mut nibbles = Vec::new();
    let mut index = start + 1;

    while index < block.len() {
        if (index - start).is_multiple_of(DEADLINE_CHECK_STRIDE) {
            budget.check_deadline()?;
        }

        match block[index] {
            b'>' => {
                if nibbles.len() % 2 == 1 {
                    if nibbles.len() >= MAX_PDF_TEXT_RUN_BYTES * 2 {
                        return Err(ParserError::resource_exhausted(
                            "pdf text run exceeds parser budget",
                        ));
                    }
                    nibbles.push(0);
                }

                let bytes = nibbles
                    .chunks(2)
                    .map(|pair| (pair[0] << 4) | pair[1])
                    .collect::<Vec<_>>();
                return decode_text_run_bytes(
                    &bytes,
                    TextRunEncodingFallback::LossyUtf8,
                    "pdf text run is not utf-8",
                )
                .map(|text| Some((text, index + 1)));
            }
            byte if byte.is_ascii_whitespace() => {}
            byte => {
                let Some(value) = hex_nibble(byte) else {
                    return Ok(None);
                };
                if nibbles.len() >= MAX_PDF_TEXT_RUN_BYTES * 2 {
                    return Err(ParserError::resource_exhausted(
                        "pdf text run exceeds parser budget",
                    ));
                }
                nibbles.push(value);
            }
        }

        index += 1;
    }

    Ok(None)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum TextRunEncodingFallback {
    StrictUtf8,
    LossyUtf8,
}

fn decode_text_run_bytes(
    bytes: &[u8],
    fallback: TextRunEncodingFallback,
    invalid_utf8_message: &'static str,
) -> Result<String> {
    if let Some(text) = decode_utf16_with_bom(bytes)? {
        return Ok(text);
    }

    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_owned()),
        Err(_) => match fallback {
            TextRunEncodingFallback::StrictUtf8 => {
                Err(ParserError::corrupted(invalid_utf8_message))
            }
            TextRunEncodingFallback::LossyUtf8 => Ok(String::from_utf8_lossy(bytes).into_owned()),
        },
    }
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

fn has_visible_text(text: &str) -> bool {
    text.chars().any(|character| {
        !character.is_whitespace()
            && (character.is_alphanumeric() || character.is_ascii_punctuation())
    })
}

fn count_pdf_pages(bytes: &[u8], budget: &ParseBudget) -> Result<Option<usize>> {
    let marker = b"/Type /Page";
    let mut count = 0_usize;
    let mut index = 0;

    while let Some(marker_start) = find_bytes(bytes, marker, index, budget)? {
        let after_marker = marker_start + marker.len();
        if bytes.get(after_marker) != Some(&b's') {
            count += 1;
        }
        index = after_marker;
    }

    Ok((count > 0).then_some(count))
}

fn contains_bytes(bytes: &[u8], needle: &[u8], budget: &ParseBudget) -> Result<bool> {
    Ok(find_bytes(bytes, needle, 0, budget)?.is_some())
}

fn find_operator(
    bytes: &[u8],
    operator: &[u8],
    start: usize,
    budget: &ParseBudget,
) -> Result<Option<usize>> {
    budget.check_deadline()?;
    if operator.is_empty() || start >= bytes.len() {
        return Ok(None);
    }

    let Some(max_start) = bytes.len().checked_sub(operator.len()) else {
        return Ok(None);
    };
    if start > max_start {
        return Ok(None);
    }

    for index in start..=max_start {
        if (index - start).is_multiple_of(DEADLINE_CHECK_STRIDE) {
            budget.check_deadline()?;
        }

        if bytes[index..].starts_with(operator)
            && is_operator_start_boundary(bytes, index)
            && is_operator_end_boundary(bytes, index + operator.len())
        {
            return Ok(Some(index));
        }
    }

    budget.check_deadline()?;
    Ok(None)
}

fn is_operator_start_boundary(bytes: &[u8], index: usize) -> bool {
    if index == 0 {
        return true;
    }

    is_delimiter(bytes[index - 1])
}

fn is_operator_end_boundary(bytes: &[u8], index: usize) -> bool {
    match bytes.get(index) {
        None => true,
        Some(byte) => is_delimiter(*byte),
    }
}

fn is_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace() || b"[]()<>{}/%".contains(&byte)
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
