use lopdf::{Document as LoPdfDocument, Encoding, LoadOptions, Object, ObjectId};
use parser_common::{
    FileProbe, ParseBudget, ParseInput, ParseOutput, ParseStatus, Parser, ParserError,
    ResourceBudget, Result, SupportLevel,
};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

const MAX_EXTRACTED_TEXT_CHARS: usize = 1_000_000;
const DEADLINE_CHECK_STRIDE: usize = 4096;
const PDF_EOF_MARKER_TAIL_BYTES: usize = 16 * 1024;

pub fn crate_name() -> &'static str {
    "parser-pdf"
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PdfParser;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PdfTextExtractionTimings {
    pub document_load: Duration,
    pub page_content_fetch: Duration,
    pub text_operator_prefilter: Duration,
    pub font_encoding: Duration,
    pub content_decode: Duration,
    pub content_string_parse: Duration,
    pub text_collection: Duration,
    pub text_byte_decode: Duration,
    pub text_accumulation: Duration,
    pub content_string_operands: u64,
    pub content_string_bytes: u64,
    pub text_decode_runs: u64,
    pub text_decode_input_bytes: u64,
}

impl PdfTextExtractionTimings {
    pub fn add_assign(&mut self, next: &Self) {
        self.document_load += next.document_load;
        self.page_content_fetch += next.page_content_fetch;
        self.text_operator_prefilter += next.text_operator_prefilter;
        self.font_encoding += next.font_encoding;
        self.content_decode += next.content_decode;
        self.content_string_parse += next.content_string_parse;
        self.text_collection += next.text_collection;
        self.text_byte_decode += next.text_byte_decode;
        self.text_accumulation += next.text_accumulation;
        self.content_string_operands += next.content_string_operands;
        self.content_string_bytes += next.content_string_bytes;
        self.text_decode_runs += next.text_decode_runs;
        self.text_decode_input_bytes += next.text_decode_input_bytes;
    }
}

impl PdfParser {
    pub fn parse_with_timings(
        &self,
        input: ParseInput<'_>,
        budget: ResourceBudget,
    ) -> Result<(ParseOutput, PdfTextExtractionTimings)> {
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
        if !has_pdf_eof_marker(bytes, &parse_budget)? {
            return Err(ParserError::corrupted("pdf EOF marker is missing"));
        }

        parse_budget.check_deadline()?;
        let mut timings = PdfTextExtractionTimings::default();
        let extraction = extract_text_layer_with_timings(bytes, &parse_budget, &mut timings)?;
        if has_text_signal(&extraction.text) {
            return Ok((
                ParseOutput::new(ParseStatus::TextLayer, extraction.text)
                    .with_page_count(extraction.page_count),
                timings,
            ));
        }

        Ok((
            ParseOutput::new(ParseStatus::OcrRequired, "").with_page_count(extraction.page_count),
            timings,
        ))
    }
}

impl Parser for PdfParser {
    fn supports(&self, probe: &FileProbe) -> SupportLevel {
        match (probe.extension(), probe.has_pdf_header()) {
            (Some("pdf"), true) => SupportLevel::Supported,
            (Some("pdf"), false) | (_, true) => SupportLevel::Possible,
            _ => SupportLevel::Unsupported,
        }
    }

    fn parse(&self, input: ParseInput<'_>, budget: ResourceBudget) -> Result<ParseOutput> {
        self.parse_with_timings(input, budget)
            .map(|(output, _timings)| output)
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
}

#[cfg(test)]
fn extract_text_layer(bytes: &[u8], budget: &ParseBudget) -> Result<LopdfTextExtraction> {
    let mut timings = PdfTextExtractionTimings::default();
    extract_text_layer_with_timings(bytes, budget, &mut timings)
}

fn extract_text_layer_with_timings(
    bytes: &[u8],
    budget: &ParseBudget,
    timings: &mut PdfTextExtractionTimings,
) -> Result<LopdfTextExtraction> {
    budget.check_deadline()?;
    let document = measure_pdf_timing(&mut timings.document_load, || {
        load_document_for_text_extraction(bytes)
    })?;
    if document.trailer.get(b"Encrypt").is_ok() {
        return Err(ParserError::encrypted("pdf declares an encrypted trailer"));
    }
    budget.check_deadline()?;

    let pages_by_number = document.get_pages();
    let page_count = pages_by_number.len().max(1);
    let mut accumulator = TextAccumulator::default();
    let mut font_encoding_cache = FontEncodingCache::default();

    for (_, page_id) in pages_by_number {
        budget.check_deadline()?;
        let page_text = extract_page_text(
            &document,
            page_id,
            budget,
            &mut font_encoding_cache,
            timings,
        )?;
        if page_text.is_empty() {
            continue;
        }

        accumulator.push(&page_text)?;
    }

    budget.check_deadline()?;
    Ok(LopdfTextExtraction {
        text: accumulator.into_string(),
        page_count,
    })
}

fn measure_pdf_timing<T>(stage: &mut Duration, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    *stage += started.elapsed();
    result
}

fn extract_page_text<'a>(
    document: &'a LoPdfDocument,
    page_id: ObjectId,
    budget: &ParseBudget,
    font_encoding_cache: &mut FontEncodingCache<'a>,
    timings: &mut PdfTextExtractionTimings,
) -> Result<String> {
    let content_data = measure_pdf_timing(&mut timings.page_content_fetch, || {
        document
            .get_page_content(page_id)
            .map_err(|_| ParserError::corrupted("pdf structure is invalid"))
    })?;
    budget.check_deadline()?;
    if !measure_pdf_timing(&mut timings.text_operator_prefilter, || {
        content_may_show_text(&content_data)
    }) {
        return Ok(String::new());
    }

    let encodings = measure_pdf_timing(&mut timings.font_encoding, || {
        page_font_encodings_with_cache(document, page_id, font_encoding_cache)
    })?;
    budget.check_deadline()?;
    let mut content_decode_details = PdfTextExtractionTimings::default();
    let content = measure_pdf_timing(&mut timings.content_decode, || {
        decode_text_only_content_with_timings(&content_data, budget, &mut content_decode_details)
    })?;
    timings.add_assign(&content_decode_details);
    let mut text_collection_details = PdfTextExtractionTimings::default();
    let page_text = measure_pdf_timing(&mut timings.text_collection, || {
        collect_text_only_page_text_with_timings(
            &content,
            &encodings,
            budget,
            &mut text_collection_details,
        )
    })?;
    timings.add_assign(&text_collection_details);
    Ok(page_text)
}

#[cfg(test)]
fn collect_decoded_page_text(
    content: &lopdf::content::Content,
    encodings: &BTreeMap<Vec<u8>, Rc<Encoding<'_>>>,
    budget: &ParseBudget,
) -> Result<String> {
    let mut accumulator = TextAccumulator::default();
    let mut current_text = String::new();
    let mut current_encoding: Option<Rc<Encoding<'_>>> = None;

    for operation in &content.operations {
        budget.check_deadline()?;
        match operation.operator.as_str() {
            "Tf" => {
                flush_page_text(&mut current_text, &mut accumulator)?;
                current_encoding = operation
                    .operands
                    .first()
                    .and_then(|operand| operand.as_name().ok())
                    .and_then(|font_name| encodings.get(font_name).cloned());
            }
            "Tj" | "TJ" => {
                if let Some(encoding) = current_encoding.as_deref() {
                    collect_page_text(&mut current_text, encoding, &operation.operands)?;
                }
            }
            "'" => {
                if let Some(encoding) = current_encoding.as_deref() {
                    if !current_text.ends_with('\n') {
                        current_text.push('\n');
                    }
                    collect_page_text(&mut current_text, encoding, &operation.operands)?;
                }
            }
            "\"" => {
                if let Some(encoding) = current_encoding.as_deref() {
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

#[derive(Debug, PartialEq, Eq)]
struct TextOnlyContent {
    operations: Vec<TextOnlyOperation>,
}

#[derive(Debug, PartialEq, Eq)]
enum TextOnlyOperation {
    SetFont(Vec<u8>),
    Show(Vec<TextOnlyOperand>),
    ShowLine(Vec<TextOnlyOperand>),
    NewLine,
    EndText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TextOnlyOperand {
    Name(Vec<u8>),
    String(Vec<u8>),
    Array(Vec<TextOnlyOperand>),
    Integer(i64),
    Other,
}

#[cfg(test)]
fn decode_text_only_content(content_data: &[u8], budget: &ParseBudget) -> Result<TextOnlyContent> {
    let mut timings = PdfTextExtractionTimings::default();
    decode_text_only_content_with_timings(content_data, budget, &mut timings)
}

fn decode_text_only_content_with_timings(
    content_data: &[u8],
    budget: &ParseBudget,
    timings: &mut PdfTextExtractionTimings,
) -> Result<TextOnlyContent> {
    TextOnlyContentParser::new(content_data, budget, timings).parse()
}

struct TextOnlyContentParser<'a> {
    bytes: &'a [u8],
    budget: &'a ParseBudget,
    timings: &'a mut PdfTextExtractionTimings,
    index: usize,
    operands: Vec<TextOnlyOperand>,
    operations: Vec<TextOnlyOperation>,
}

impl<'a> TextOnlyContentParser<'a> {
    fn new(
        bytes: &'a [u8],
        budget: &'a ParseBudget,
        timings: &'a mut PdfTextExtractionTimings,
    ) -> Self {
        Self {
            bytes,
            budget,
            timings,
            index: 0,
            operands: Vec::new(),
            operations: Vec::new(),
        }
    }

    fn parse(mut self) -> Result<TextOnlyContent> {
        while self.index < self.bytes.len() {
            self.check_deadline()?;
            self.skip_whitespace_and_comments();
            if self.index >= self.bytes.len() {
                break;
            }

            match self.bytes[self.index] {
                b'/' => {
                    self.index += 1;
                    let name = self.parse_name()?;
                    self.operands.push(TextOnlyOperand::Name(name));
                }
                b'(' => {
                    self.index += 1;
                    let string = self.parse_literal_string_operand()?;
                    self.operands.push(TextOnlyOperand::String(string));
                }
                b'<' if self.bytes.get(self.index + 1) == Some(&b'<') => {
                    self.index += 2;
                    self.operands.push(TextOnlyOperand::Other);
                }
                b'>' if self.bytes.get(self.index + 1) == Some(&b'>') => {
                    self.index += 2;
                    self.operands.push(TextOnlyOperand::Other);
                }
                b'<' => {
                    self.index += 1;
                    let string = self.parse_hex_string_operand()?;
                    self.operands.push(TextOnlyOperand::String(string));
                }
                b'[' => {
                    self.index += 1;
                    let array = self.parse_array()?;
                    self.operands.push(TextOnlyOperand::Array(array));
                }
                b']' => {
                    return Err(ParserError::corrupted("pdf content stream is invalid"));
                }
                b'\'' => {
                    self.index += 1;
                    self.handle_operator(b"'");
                }
                b'"' => {
                    self.index += 1;
                    self.handle_operator(b"\"");
                }
                byte if is_pdf_number_start(byte) => {
                    let token = self.parse_regular_token();
                    self.operands.push(number_operand(token));
                }
                _ => {
                    let token = self.parse_regular_token();
                    if token.is_empty() {
                        self.index += 1;
                        continue;
                    }
                    self.handle_operator(token);
                    if token == b"BI" {
                        self.skip_inline_image_data()?;
                    }
                }
            }
        }

        Ok(TextOnlyContent {
            operations: self.operations,
        })
    }

    fn parse_array(&mut self) -> Result<Vec<TextOnlyOperand>> {
        let mut items = Vec::new();
        loop {
            self.check_deadline()?;
            self.skip_whitespace_and_comments();
            if self.index >= self.bytes.len() {
                return Err(ParserError::corrupted("pdf content stream is invalid"));
            }

            match self.bytes[self.index] {
                b']' => {
                    self.index += 1;
                    return Ok(items);
                }
                b'/' => {
                    self.index += 1;
                    items.push(TextOnlyOperand::Name(self.parse_name()?));
                }
                b'(' => {
                    self.index += 1;
                    items.push(TextOnlyOperand::String(
                        self.parse_literal_string_operand()?,
                    ));
                }
                b'<' if self.bytes.get(self.index + 1) == Some(&b'<') => {
                    self.index += 2;
                    items.push(TextOnlyOperand::Other);
                }
                b'>' if self.bytes.get(self.index + 1) == Some(&b'>') => {
                    self.index += 2;
                    items.push(TextOnlyOperand::Other);
                }
                b'<' => {
                    self.index += 1;
                    items.push(TextOnlyOperand::String(self.parse_hex_string_operand()?));
                }
                b'[' => {
                    self.index += 1;
                    items.push(TextOnlyOperand::Array(self.parse_array()?));
                }
                byte if is_pdf_number_start(byte) => {
                    let token = self.parse_regular_token();
                    items.push(number_operand(token));
                }
                _ => {
                    let token = self.parse_regular_token();
                    if token.is_empty() {
                        self.index += 1;
                    } else {
                        items.push(TextOnlyOperand::Other);
                    }
                }
            }
        }
    }

    fn handle_operator(&mut self, operator: &[u8]) {
        match operator {
            b"Tf" => {
                if let Some(TextOnlyOperand::Name(font_name)) = self.operands.first() {
                    self.operations
                        .push(TextOnlyOperation::SetFont(font_name.clone()));
                }
            }
            b"Tj" | b"TJ" => {
                self.operations
                    .push(TextOnlyOperation::Show(std::mem::take(&mut self.operands)));
                return;
            }
            b"'" => {
                self.operations
                    .push(TextOnlyOperation::ShowLine(std::mem::take(
                        &mut self.operands,
                    )));
                return;
            }
            b"\"" => {
                let text_operand = self.operands.get(2).cloned();
                self.operations.push(TextOnlyOperation::ShowLine(
                    text_operand.into_iter().collect(),
                ));
            }
            b"T*" => self.operations.push(TextOnlyOperation::NewLine),
            b"ET" => self.operations.push(TextOnlyOperation::EndText),
            _ => {}
        }
        self.operands.clear();
    }

    fn parse_name(&mut self) -> Result<Vec<u8>> {
        let start = self.index;
        while self.index < self.bytes.len() && !is_pdf_token_delimiter(self.bytes[self.index]) {
            self.index += 1;
        }
        decode_pdf_name(&self.bytes[start..self.index])
    }

    fn parse_literal_string_operand(&mut self) -> Result<Vec<u8>> {
        let result = if self.timings.content_string_operands == 0 {
            let started = Instant::now();
            let result = self.parse_literal_string();
            self.timings.content_string_parse += started.elapsed();
            result
        } else {
            self.parse_literal_string()
        };
        if let Ok(bytes) = &result {
            self.record_content_string_operand(bytes.len());
        }
        result
    }

    fn parse_literal_string(&mut self) -> Result<Vec<u8>> {
        let mut nesting = 1_usize;
        let mut output = Vec::new();
        while self.index < self.bytes.len() {
            self.check_deadline()?;
            match self.bytes[self.index] {
                b'\\' => {
                    self.index += 1;
                    if self.index >= self.bytes.len() {
                        return Err(ParserError::corrupted("pdf content stream is invalid"));
                    }
                    match self.bytes[self.index] {
                        b'n' => output.push(b'\n'),
                        b'r' => output.push(b'\r'),
                        b't' => output.push(b'\t'),
                        b'b' => output.push(0x08),
                        b'f' => output.push(0x0c),
                        b'(' | b')' | b'\\' => output.push(self.bytes[self.index]),
                        b'\r' => {
                            self.index += 1;
                            if self.bytes.get(self.index) == Some(&b'\n') {
                                self.index += 1;
                            }
                            continue;
                        }
                        b'\n' => {
                            self.index += 1;
                            continue;
                        }
                        byte if byte.is_ascii_digit() && byte < b'8' => {
                            output.push(self.parse_octal_escape(byte));
                            continue;
                        }
                        byte => output.push(byte),
                    }
                    self.index += 1;
                }
                b'(' => {
                    nesting += 1;
                    output.push(b'(');
                    self.index += 1;
                }
                b')' => {
                    nesting -= 1;
                    self.index += 1;
                    if nesting == 0 {
                        return Ok(output);
                    }
                    output.push(b')');
                }
                byte => {
                    output.push(byte);
                    self.index += 1;
                }
            }
        }
        Err(ParserError::corrupted("pdf content stream is invalid"))
    }

    fn parse_octal_escape(&mut self, first_digit: u8) -> u8 {
        let mut value = first_digit - b'0';
        self.index += 1;
        for _ in 0..2 {
            let Some(&byte) = self.bytes.get(self.index) else {
                break;
            };
            if !byte.is_ascii_digit() || byte >= b'8' {
                break;
            }
            value = value.saturating_mul(8).saturating_add(byte - b'0');
            self.index += 1;
        }
        value
    }

    fn parse_hex_string_operand(&mut self) -> Result<Vec<u8>> {
        let result = if self.timings.content_string_operands == 0 {
            let started = Instant::now();
            let result = self.parse_hex_string();
            self.timings.content_string_parse += started.elapsed();
            result
        } else {
            self.parse_hex_string()
        };
        if let Ok(bytes) = &result {
            self.record_content_string_operand(bytes.len());
        }
        result
    }

    fn parse_hex_string(&mut self) -> Result<Vec<u8>> {
        let mut nibbles = Vec::new();
        while self.index < self.bytes.len() {
            self.check_deadline()?;
            match self.bytes[self.index] {
                b'>' => {
                    self.index += 1;
                    if nibbles.len() % 2 == 1 {
                        nibbles.push(0);
                    }
                    return Ok(nibbles
                        .chunks_exact(2)
                        .map(|chunk| (chunk[0] << 4) | chunk[1])
                        .collect());
                }
                byte if byte.is_ascii_whitespace() => self.index += 1,
                byte => {
                    let Some(nibble) = hex_nibble(byte) else {
                        return Err(ParserError::corrupted("pdf content stream is invalid"));
                    };
                    nibbles.push(nibble);
                    self.index += 1;
                }
            }
        }
        Err(ParserError::corrupted("pdf content stream is invalid"))
    }

    fn parse_regular_token(&mut self) -> &'a [u8] {
        let start = self.index;
        while self.index < self.bytes.len() && !is_pdf_token_delimiter(self.bytes[self.index]) {
            self.index += 1;
        }
        &self.bytes[start..self.index]
    }

    fn record_content_string_operand(&mut self, byte_len: usize) {
        self.timings.content_string_operands += 1;
        self.timings.content_string_bytes += byte_len as u64;
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while self
                .bytes
                .get(self.index)
                .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                self.index += 1;
            }
            if self.bytes.get(self.index) != Some(&b'%') {
                return;
            }
            self.index = skip_pdf_comment(self.bytes, self.index + 1);
        }
    }

    fn skip_inline_image_data(&mut self) -> Result<()> {
        let Some(data_start) = find_inline_image_data_start(self.bytes, self.index) else {
            return Err(ParserError::corrupted("pdf content stream is invalid"));
        };
        let Some(image_end) = find_inline_image_end(self.bytes, data_start) else {
            return Err(ParserError::corrupted("pdf content stream is invalid"));
        };
        self.index = image_end;
        Ok(())
    }

    fn check_deadline(&self) -> Result<()> {
        if self.index.is_multiple_of(DEADLINE_CHECK_STRIDE) {
            self.budget.check_deadline()?;
        }
        Ok(())
    }
}

#[cfg(test)]
fn collect_text_only_page_text(
    content: &TextOnlyContent,
    encodings: &BTreeMap<Vec<u8>, Rc<Encoding<'_>>>,
    budget: &ParseBudget,
) -> Result<String> {
    let mut timings = PdfTextExtractionTimings::default();
    collect_text_only_page_text_with_timings(content, encodings, budget, &mut timings)
}

fn collect_text_only_page_text_with_timings(
    content: &TextOnlyContent,
    encodings: &BTreeMap<Vec<u8>, Rc<Encoding<'_>>>,
    budget: &ParseBudget,
    timings: &mut PdfTextExtractionTimings,
) -> Result<String> {
    let mut accumulator = TextAccumulator::default();
    let mut current_text = String::new();
    let mut current_encoding: Option<Rc<Encoding<'_>>> = None;

    for operation in &content.operations {
        budget.check_deadline()?;
        match operation {
            TextOnlyOperation::SetFont(font_name) => {
                flush_page_text_with_timings(&mut current_text, &mut accumulator, timings)?;
                current_encoding = encodings.get(font_name).cloned();
            }
            TextOnlyOperation::Show(operands) => {
                if let Some(encoding) = current_encoding.as_deref() {
                    collect_text_only_operands(&mut current_text, encoding, operands, timings)?;
                }
            }
            TextOnlyOperation::ShowLine(operands) => {
                if let Some(encoding) = current_encoding.as_deref() {
                    if !current_text.ends_with('\n') {
                        current_text.push('\n');
                    }
                    collect_text_only_operands(&mut current_text, encoding, operands, timings)?;
                }
            }
            TextOnlyOperation::NewLine if !current_text.ends_with('\n') => {
                current_text.push('\n');
            }
            TextOnlyOperation::NewLine => {}
            TextOnlyOperation::EndText if !current_text.ends_with('\n') => {
                current_text.push('\n');
            }
            TextOnlyOperation::EndText => {}
        }
    }

    flush_page_text_with_timings(&mut current_text, &mut accumulator, timings)?;
    Ok(accumulator.into_string())
}

fn collect_text_only_operands(
    text: &mut String,
    encoding: &Encoding<'_>,
    operands: &[TextOnlyOperand],
    timings: &mut PdfTextExtractionTimings,
) -> Result<()> {
    for operand in operands {
        match operand {
            TextOnlyOperand::String(bytes) => {
                if timings.text_decode_runs == 0 {
                    measure_pdf_timing(&mut timings.text_byte_decode, || {
                        decode_text_bytes_into(encoding, bytes, text)
                    })?
                } else {
                    decode_text_bytes_into(encoding, bytes, text)?
                }
                timings.text_decode_runs += 1;
                timings.text_decode_input_bytes += bytes.len() as u64;
            }
            TextOnlyOperand::Array(items) => {
                collect_text_only_operands(text, encoding, items, timings)?;
                text.push(' ');
            }
            TextOnlyOperand::Integer(value) if *value < -100 => {
                text.push(' ');
            }
            _ => {}
        }
    }
    Ok(())
}

fn flush_page_text_with_timings(
    buffer: &mut String,
    accumulator: &mut TextAccumulator,
    timings: &mut PdfTextExtractionTimings,
) -> Result<()> {
    measure_pdf_timing(&mut timings.text_accumulation, || {
        flush_page_text(buffer, accumulator)
    })
}

fn number_operand(token: &[u8]) -> TextOnlyOperand {
    std::str::from_utf8(token)
        .ok()
        .and_then(|token| token.parse::<i64>().ok())
        .map(TextOnlyOperand::Integer)
        .unwrap_or(TextOnlyOperand::Other)
}

fn is_pdf_number_start(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')
}

fn is_pdf_token_delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'%' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/'
        )
}

fn decode_pdf_name(raw: &[u8]) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'#' {
            let Some(first) = raw.get(index + 1).and_then(|byte| hex_nibble(*byte)) else {
                return Err(ParserError::corrupted("pdf content stream is invalid"));
            };
            let Some(second) = raw.get(index + 2).and_then(|byte| hex_nibble(*byte)) else {
                return Err(ParserError::corrupted("pdf content stream is invalid"));
            };
            output.push((first << 4) | second);
            index += 3;
        } else {
            output.push(raw[index]);
            index += 1;
        }
    }
    Ok(output)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn find_inline_image_data_start(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index + 2 <= bytes.len() {
        if text_operator_at(bytes, index, b"ID") {
            let mut data_start = index + 2;
            if bytes
                .get(data_start)
                .is_some_and(|byte| byte.is_ascii_whitespace())
            {
                data_start += 1;
            }
            return Some(data_start);
        }
        index += 1;
    }
    None
}

fn find_inline_image_end(bytes: &[u8], mut index: usize) -> Option<usize> {
    while index + 2 <= bytes.len() {
        if text_operator_at(bytes, index, b"EI") {
            return Some(index + 2);
        }
        index += 1;
    }
    None
}

#[derive(Default)]
struct FontEncodingCache<'a> {
    encodings_by_font_ptr: BTreeMap<usize, Rc<Encoding<'a>>>,
}

impl FontEncodingCache<'_> {
    #[cfg(test)]
    fn len(&self) -> usize {
        self.encodings_by_font_ptr.len()
    }
}

fn page_font_encodings_with_cache<'a>(
    document: &'a LoPdfDocument,
    page_id: ObjectId,
    cache: &mut FontEncodingCache<'a>,
) -> Result<BTreeMap<Vec<u8>, Rc<Encoding<'a>>>> {
    let fonts = document
        .get_page_fonts(page_id)
        .map_err(|_| ParserError::corrupted("pdf structure is invalid"))?;
    let mut encodings = BTreeMap::new();
    for (name, font) in fonts {
        let font_key = font as *const _ as usize;
        let encoding = match cache.encodings_by_font_ptr.get(&font_key) {
            Some(encoding) => Rc::clone(encoding),
            None => {
                let encoding = Rc::new(
                    font.get_font_encoding(document)
                        .map_err(|_| ParserError::corrupted("pdf structure is invalid"))?,
                );
                cache
                    .encodings_by_font_ptr
                    .insert(font_key, Rc::clone(&encoding));
                encoding
            }
        };
        encodings.insert(name, encoding);
    }
    Ok(encodings)
}

fn content_may_show_text(content_data: &[u8]) -> bool {
    let mut index = 0;
    while index < content_data.len() {
        match content_data[index] {
            b'%' => index = skip_pdf_comment(content_data, index + 1),
            b'(' => index = skip_pdf_literal_string(content_data, index + 1),
            b'<' if content_data.get(index + 1) != Some(&b'<') => {
                index = skip_pdf_hex_string(content_data, index + 1);
            }
            _ if text_showing_operator_at(content_data, index) => return true,
            _ => index += 1,
        }
    }
    false
}

fn text_showing_operator_at(content_data: &[u8], index: usize) -> bool {
    text_operator_at(content_data, index, b"Tj")
        || text_operator_at(content_data, index, b"TJ")
        || text_operator_at(content_data, index, b"'")
        || text_operator_at(content_data, index, b"\"")
}

fn text_operator_at(content_data: &[u8], index: usize, operator: &[u8]) -> bool {
    if index + operator.len() > content_data.len() {
        return false;
    }

    let previous = if index == 0 {
        None
    } else {
        content_data.get(index - 1).copied()
    };
    content_data[index..].starts_with(operator)
        && is_pdf_operator_boundary(previous)
        && is_pdf_operator_boundary(content_data.get(index + operator.len()).copied())
}

fn skip_pdf_comment(content_data: &[u8], mut index: usize) -> usize {
    while index < content_data.len() && !matches!(content_data[index], b'\n' | b'\r') {
        index += 1;
    }
    index
}

fn skip_pdf_hex_string(content_data: &[u8], mut index: usize) -> usize {
    while index < content_data.len() {
        if content_data[index] == b'>' {
            return index + 1;
        }
        index += 1;
    }
    index
}

fn skip_pdf_literal_string(content_data: &[u8], mut index: usize) -> usize {
    let mut nesting = 1_usize;
    while index < content_data.len() {
        match content_data[index] {
            b'\\' => {
                index = (index + 2).min(content_data.len());
            }
            b'(' => {
                nesting += 1;
                index += 1;
            }
            b')' => {
                nesting -= 1;
                index += 1;
                if nesting == 0 {
                    return index;
                }
            }
            _ => index += 1,
        }
    }
    index
}

fn is_pdf_operator_boundary(byte: Option<u8>) -> bool {
    byte.is_none_or(|byte| {
        byte.is_ascii_whitespace()
            || matches!(
                byte,
                b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/'
            )
    })
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

#[cfg(test)]
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

#[cfg(test)]
fn decode_text_bytes(encoding: &Encoding<'_>, bytes: &[u8]) -> Result<String> {
    let mut text = String::new();
    decode_text_bytes_into(encoding, bytes, &mut text)?;
    Ok(text)
}

fn decode_text_bytes_into(
    encoding: &Encoding<'_>,
    bytes: &[u8],
    output: &mut String,
) -> Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    if decode_utf16_with_bom_into(bytes, output)? {
        return Ok(());
    }

    let original_len = output.len();
    if encoding.write_to_string(bytes, output).is_err() {
        output.truncate(original_len);
        return Err(ParserError::corrupted("pdf structure is invalid"));
    }
    Ok(())
}

fn decode_utf16_with_bom_into(bytes: &[u8], output: &mut String) -> Result<bool> {
    let Some((&first, rest)) = bytes.split_first() else {
        return Ok(false);
    };
    let Some((&second, payload)) = rest.split_first() else {
        return Ok(false);
    };
    let endianness = match (first, second) {
        (0xFE, 0xFF) => Utf16Endianness::Big,
        (0xFF, 0xFE) => Utf16Endianness::Little,
        _ => return Ok(false),
    };

    if payload.len() % 2 != 0 {
        return Err(ParserError::corrupted(
            "pdf utf-16 text run has odd byte length",
        ));
    }

    let original_len = output.len();
    for decoded in std::char::decode_utf16(payload.chunks_exact(2).map(|chunk| match endianness {
        Utf16Endianness::Big => u16::from_be_bytes([chunk[0], chunk[1]]),
        Utf16Endianness::Little => u16::from_le_bytes([chunk[0], chunk[1]]),
    })) {
        match decoded {
            Ok(character) => output.push(character),
            Err(_) => {
                output.truncate(original_len);
                return Err(ParserError::corrupted("pdf utf-16 text run is invalid"));
            }
        }
    }
    Ok(true)
}

#[derive(Clone, Copy)]
enum Utf16Endianness {
    Big,
    Little,
}

fn has_text_signal(text: &str) -> bool {
    let mut non_whitespace_chars = 0;
    for character in text.chars() {
        if character.is_whitespace() {
            continue;
        }
        non_whitespace_chars += 1;
        if non_whitespace_chars >= 3 {
            return true;
        }
    }
    false
}

fn has_pdf_eof_marker(bytes: &[u8], budget: &ParseBudget) -> Result<bool> {
    let search_start = bytes.len().saturating_sub(PDF_EOF_MARKER_TAIL_BYTES);
    Ok(find_bytes(bytes, b"%%EOF", search_start, budget)?.is_some())
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

fn load_document_for_text_extraction(bytes: &[u8]) -> Result<LoPdfDocument> {
    LoPdfDocument::load_mem_with_options(
        bytes,
        LoadOptions::with_filter(filter_text_extraction_object),
    )
    .map_err(|_| ParserError::corrupted("pdf structure is invalid"))
}

fn filter_text_extraction_object(
    object_id: ObjectId,
    object: &mut Object,
) -> Option<(ObjectId, Object)> {
    clear_image_xobject_stream_content(object);
    Some((object_id, object.clone()))
}

fn clear_image_xobject_stream_content(object: &mut Object) {
    if let Object::Stream(stream) = object {
        if is_image_xobject_stream_dict(&stream.dict) {
            stream.content.clear();
            stream.content.shrink_to_fit();
        }
    }
}

fn is_image_xobject_stream_dict(dict: &lopdf::Dictionary) -> bool {
    dict.get(b"Subtype")
        .ok()
        .and_then(|subtype| subtype.as_name().ok())
        .is_some_and(|subtype| subtype == b"Image")
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::Content;

    #[test]
    fn font_encoding_cache_reuses_shared_font_dictionary_across_pages() {
        let bytes = two_page_shared_tounicode_pdf();
        let document = LoPdfDocument::load_mem(&bytes).unwrap();
        let mut cache = FontEncodingCache::default();

        let first = page_font_encodings_with_cache(&document, (3, 0), &mut cache).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(cache.len(), 1);

        let second = page_font_encodings_with_cache(&document, (4, 0), &mut cache).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(
            cache.len(),
            1,
            "shared font dictionary should not parse a second encoding"
        );

        let budget = ResourceBudget::default().begin(bytes.len()).unwrap();
        let extraction = extract_text_layer(&bytes, &budget).unwrap();
        assert_eq!(extraction.page_count, 2);
        assert!(extraction.text.contains("中文简历"));
    }

    #[test]
    fn literal_encrypt_text_is_not_classified_as_encrypted() {
        let bytes = literal_encrypt_text_pdf();
        let output = PdfParser
            .parse(
                ParseInput::from_bytes(Some("pdf"), &bytes),
                ResourceBudget::default(),
            )
            .unwrap();

        assert_eq!(output.status(), ParseStatus::TextLayer);
        assert!(output.text().contains("Resume /Encrypt literal"));
    }

    #[test]
    fn eof_marker_must_be_in_pdf_tail() {
        let bytes = early_eof_marker_without_tail_eof_pdf();
        let budget = ResourceBudget::default().begin(bytes.len()).unwrap();

        assert!(!has_pdf_eof_marker(&bytes, &budget).unwrap());

        let error = PdfParser
            .parse(
                ParseInput::from_bytes(Some("pdf"), &bytes),
                ResourceBudget::default(),
            )
            .unwrap_err();
        assert_eq!(error.kind(), parser_common::ParserErrorKind::Corrupted);
        assert_eq!(error.code(), "CORRUPTED_DOCUMENT");

        let valid = literal_encrypt_text_pdf();
        let budget = ResourceBudget::default().begin(valid.len()).unwrap();
        assert!(has_pdf_eof_marker(&valid, &budget).unwrap());
    }

    #[test]
    fn page_content_without_text_showing_operator_can_skip_text_decode() {
        assert!(!content_may_show_text(b"q 100 0 0 100 0 0 cm /Im1 Do Q"));
        assert!(!content_may_show_text(b"BT /F1 12 Tf ET"));
        assert!(!content_may_show_text(b"/TjName Do"));
        assert!(!content_may_show_text(b"q (Tj) /Im1 Do Q"));
        assert!(!content_may_show_text(b"q [(TJ) (') (\")] /Im1 Do Q"));
        assert!(!content_may_show_text(b"% Tj in a comment\nq /Im1 Do Q"));

        assert!(content_may_show_text(b"BT /F1 12 Tf (Resume) Tj ET"));
        assert!(content_may_show_text(b"BT /F1 12 Tf [(Resume)] TJ ET"));
        assert!(content_may_show_text(b"BT /F1 12 Tf (Resume) ' ET"));
        assert!(content_may_show_text(b"BT /F1 12 Tf 1 2 (Resume) \" ET"));
    }

    #[test]
    fn text_extraction_load_drops_image_xobject_stream_bodies() {
        let bytes = text_and_image_pdf();
        let unfiltered = LoPdfDocument::load_mem(&bytes).unwrap();
        let unfiltered_image = unfiltered
            .objects
            .get(&(6, 0))
            .and_then(|object| object.as_stream().ok())
            .unwrap();
        assert!(!unfiltered_image.content.is_empty());

        let filtered = load_document_for_text_extraction(&bytes).unwrap();
        let filtered_image = filtered
            .objects
            .get(&(6, 0))
            .and_then(|object| object.as_stream().ok())
            .unwrap();
        assert!(filtered_image.content.is_empty());

        let budget = ResourceBudget::default().begin(bytes.len()).unwrap();
        let extraction = extract_text_layer(&bytes, &budget).unwrap();
        assert!(extraction.text.contains("Resume with image"));
    }

    #[test]
    fn parse_with_timings_reports_text_extraction_subphases() {
        let bytes = text_and_image_pdf();

        let (output, timings) = PdfParser
            .parse_with_timings(
                ParseInput::from_bytes(Some("pdf"), &bytes),
                ResourceBudget::default(),
            )
            .unwrap();

        assert_eq!(output.status(), ParseStatus::TextLayer);
        assert!(output.text().contains("Resume with image"));
        for (label, elapsed) in [
            ("document_load", timings.document_load),
            ("page_content_fetch", timings.page_content_fetch),
            ("text_operator_prefilter", timings.text_operator_prefilter),
            ("font_encoding", timings.font_encoding),
            ("content_decode", timings.content_decode),
            ("content_string_parse", timings.content_string_parse),
            ("text_collection", timings.text_collection),
            ("text_byte_decode", timings.text_byte_decode),
            ("text_accumulation", timings.text_accumulation),
        ] {
            assert!(
                elapsed > std::time::Duration::ZERO,
                "{label} timing should be recorded: {timings:?}"
            );
        }
        for (label, count) in [
            ("content_string_operands", timings.content_string_operands),
            ("content_string_bytes", timings.content_string_bytes),
            ("text_decode_runs", timings.text_decode_runs),
            ("text_decode_input_bytes", timings.text_decode_input_bytes),
        ] {
            assert!(count > 0, "{label} counter should be recorded: {timings:?}");
        }
    }

    #[test]
    fn decode_text_bytes_into_appends_to_existing_buffer() {
        let encoding = Encoding::SimpleEncoding(b"WinAnsiEncoding");
        let mut text = String::from("prefix:");

        decode_text_bytes_into(&encoding, b"Resume", &mut text).unwrap();

        assert_eq!(text, "prefix:Resume");
    }

    #[test]
    fn decode_text_bytes_into_preserves_buffer_when_utf16_bom_is_invalid() {
        let encoding = Encoding::SimpleEncoding(b"WinAnsiEncoding");
        let mut text = String::from("prefix:");

        let error =
            decode_text_bytes_into(&encoding, &[0xFE, 0xFF, 0xD8, 0x00], &mut text).unwrap_err();

        assert_eq!(error.kind(), parser_common::ParserErrorKind::Corrupted);
        assert_eq!(text, "prefix:");
    }

    #[test]
    fn text_only_content_decoder_matches_lopdf_for_supported_text_operators() {
        let bytes = text_operator_variants_pdf();
        let budget = ResourceBudget::default().begin(bytes.len()).unwrap();
        let document = load_document_for_text_extraction(&bytes).unwrap();
        let content_data = document.get_page_content((3, 0)).unwrap();
        let mut font_cache = FontEncodingCache::default();
        let encodings = page_font_encodings_with_cache(&document, (3, 0), &mut font_cache).unwrap();

        let decoded_content = Content::decode(&content_data).unwrap();
        let lopdf_text = collect_decoded_page_text(&decoded_content, &encodings, &budget).unwrap();
        let text_only_content = decode_text_only_content(&content_data, &budget).unwrap();
        let text_only_text =
            collect_text_only_page_text(&text_only_content, &encodings, &budget).unwrap();

        assert_eq!(text_only_text, lopdf_text);
        assert!(text_only_text.contains("Simple"));
        assert!(text_only_text.contains("Array Spacing"));
        assert!(text_only_text.contains("Quote"));
        assert!(text_only_text.contains("Double Quote"));
        assert!(text_only_text.contains("中文简历"));
    }

    #[test]
    fn text_only_content_decoder_rejects_malformed_text_operands() {
        let budget = ResourceBudget::default()
            .begin(b"BT /F1 12 Tf (unterminated Tj ET".len())
            .unwrap();
        let error =
            decode_text_only_content(b"BT /F1 12 Tf (unterminated Tj ET", &budget).unwrap_err();

        assert_eq!(error.kind(), parser_common::ParserErrorKind::Corrupted);
    }

    #[test]
    fn regular_content_tokens_are_borrowed_from_the_content_stream() {
        let content = b"q 100 0 0 100 0 0 cm Q";
        let budget = ResourceBudget::default().begin(content.len()).unwrap();
        let mut timings = PdfTextExtractionTimings::default();
        let mut parser = TextOnlyContentParser::new(content, &budget, &mut timings);

        let token = parser.parse_regular_token();

        assert_eq!(token, b"q");
        assert_eq!(token.as_ptr(), content.as_ptr());
    }

    #[test]
    fn text_extraction_load_filter_clears_image_stream_content() {
        let bytes = text_and_image_pdf();
        let mut image_object = LoPdfDocument::load_mem(&bytes)
            .unwrap()
            .objects
            .remove(&(6, 0))
            .unwrap();

        let (_, filtered_object) =
            filter_text_extraction_object((6, 0), &mut image_object).unwrap();
        let filtered_stream = filtered_object.as_stream().unwrap();
        assert!(filtered_stream.content.is_empty());
        assert!(is_image_xobject_stream_dict(&filtered_stream.dict));
    }

    fn literal_encrypt_text_pdf() -> Vec<u8> {
        let content = b"BT /F1 12 Tf 72 720 Td (Resume /Encrypt literal) Tj ET\n";

        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
                content.to_vec(),
                b"endstream".to_vec(),
            ]
            .concat(),
        ])
    }

    fn text_and_image_pdf() -> Vec<u8> {
        let content =
            b"q 100 0 0 100 0 0 cm /Im1 Do Q\nBT /F1 12 Tf 72 720 Td (Resume with image) Tj ET\n";

        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> /XObject << /Im1 6 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
                content.to_vec(),
                b"endstream".to_vec(),
            ]
            .concat(),
            b"<< /Type /XObject /Subtype /Image /Width 100 /Height 100 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 11 >>\nstream\nimage bytes\nendstream".to_vec(),
        ])
    }

    fn text_operator_variants_pdf() -> Vec<u8> {
        let content = br#"BT /F1 12 Tf 72 720 Td (Simple) Tj [(Array) -120 (Spacing)] TJ (Quote) ' 1 2 (Double Quote) " <FEFF4E2D65877B805386> Tj ET
"#;

        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
                content.to_vec(),
                b"endstream".to_vec(),
            ]
            .concat(),
        ])
    }

    fn early_eof_marker_without_tail_eof_pdf() -> Vec<u8> {
        let content = b"BT /F1 12 Tf 72 720 Td (Resume %%EOF literal) Tj ET\n";
        let mut bytes = build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
                content.to_vec(),
                b"endstream".to_vec(),
            ]
            .concat(),
        ]);
        bytes.truncate(bytes.len() - b"%%EOF".len());
        bytes.extend(std::iter::repeat_n(b' ', PDF_EOF_MARKER_TAIL_BYTES + 1));
        bytes
    }

    fn two_page_shared_tounicode_pdf() -> Vec<u8> {
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
            b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 612 792] /Contents 8 0 R >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 612 792] /Contents 9 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type0 /BaseFont /TestFont /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 7 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /TestFont /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 10 0 R /DW 1000 /W [1 [1000 1000]] >>".to_vec(),
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
                "trailer << /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
                objects.len() + 1,
                xref_offset
            )
            .as_bytes(),
        );
        pdf
    }
}
