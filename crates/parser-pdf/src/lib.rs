use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use parser_common::{
    FileProbe, ParseBudget, ParseInput, ParseOutput, ParseStatus, Parser, ParserError,
    ResourceBudget, Result, SupportLevel,
};
use pdfium_render::prelude::{PdfPageTextRenderMode, PdfRect, Pdfium};

const MAX_EXTRACTED_TEXT_CHARS: usize = 1_000_000;
const MIN_TEXT_SIGNAL_CHARS: usize = 40;
const MAX_CONTROL_RATIO: f64 = 0.01;
const MAX_SUSPICIOUS_RATIO: f64 = 0.08;
const MAX_DUPLICATE_LINE_RATIO: f64 = 0.35;
const MAX_REPEATED_LONG_TOKEN_RATIO: f64 = 0.25;
const MIN_VISIBLE_ALPHA: u8 = 8;

static PDFIUM: LazyLock<std::result::Result<Mutex<Pdfium>, ()>> = LazyLock::new(|| {
    let bindings = Pdfium::bind_to_statically_linked_library().map_err(|_| ())?;
    Ok(Mutex::new(Pdfium::new(bindings)))
});

pub fn crate_name() -> &'static str {
    "parser-pdf"
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PdfParser;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PdfTextExtractionMetrics {
    pub document_load: Duration,
    pub page_text_load: Duration,
    pub character_iteration: Duration,
    pub quality_evaluation: Duration,
    pub pages_loaded: u64,
    pub characters_seen: u64,
    pub characters_emitted: u64,
    pub source_bytes: u64,
    pub output_bytes: u64,
}

impl PdfTextExtractionMetrics {
    pub fn add_assign(&mut self, next: &Self) {
        self.document_load += next.document_load;
        self.page_text_load += next.page_text_load;
        self.character_iteration += next.character_iteration;
        self.quality_evaluation += next.quality_evaluation;
        self.pages_loaded = self.pages_loaded.saturating_add(next.pages_loaded);
        self.characters_seen = self.characters_seen.saturating_add(next.characters_seen);
        self.characters_emitted = self
            .characters_emitted
            .saturating_add(next.characters_emitted);
        self.source_bytes = self.source_bytes.saturating_add(next.source_bytes);
        self.output_bytes = self.output_bytes.saturating_add(next.output_bytes);
    }
}

impl PdfParser {
    pub fn parse_with_metrics(
        &self,
        input: ParseInput<'_>,
        budget: ResourceBudget,
    ) -> Result<(ParseOutput, PdfTextExtractionMetrics)> {
        let parse_budget = budget.begin(input.bytes().len())?;
        if self.supports(input.probe()) == SupportLevel::Unsupported {
            return Err(ParserError::unsupported(
                "pdf parser received unsupported probe",
            ));
        }
        if !input.probe().has_pdf_header() {
            return Err(ParserError::corrupted("pdf header is missing"));
        }
        parse_budget.check_deadline()?;
        let mut metrics = PdfTextExtractionMetrics::default();
        let extraction = extract_visible_text(input.bytes(), &parse_budget, &mut metrics)?;
        let quality_started = Instant::now();
        let quality_is_acceptable = text_quality_is_acceptable(&extraction.text);
        metrics.quality_evaluation += quality_started.elapsed();
        let status = if quality_is_acceptable {
            ParseStatus::TextLayer
        } else {
            ParseStatus::OcrRequired
        };
        let text = if status == ParseStatus::TextLayer {
            extraction.text
        } else {
            String::new()
        };
        Ok((
            ParseOutput::new(status, text).with_page_count(extraction.page_count),
            metrics,
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
        self.parse_with_metrics(input, budget)
            .map(|(output, _metrics)| output)
    }
}

struct PdfiumTextExtraction {
    text: String,
    page_count: usize,
}

fn process_pdfium() -> Result<MutexGuard<'static, Pdfium>> {
    PDFIUM
        .as_ref()
        .map_err(|()| ParserError::internal("pdfium runtime is unavailable"))?
        .lock()
        .map_err(|_| ParserError::internal("pdfium runtime is unavailable"))
}

fn extract_visible_text(
    bytes: &[u8],
    budget: &ParseBudget,
    metrics: &mut PdfTextExtractionMetrics,
) -> Result<PdfiumTextExtraction> {
    let load_started = Instant::now();
    let pdfium = process_pdfium()?;
    let document = pdfium
        .load_pdf_from_byte_slice(bytes, None)
        .map_err(|_| ParserError::corrupted("pdfium rejected the document"))?;
    metrics.document_load += load_started.elapsed();
    let page_count = usize::try_from(document.pages().len())
        .map_err(|_| ParserError::resource_exhausted("pdf page count exceeds parser budget"))?;
    metrics.pages_loaded = page_count as u64;
    metrics.source_bytes = bytes.len() as u64;
    let mut output = String::new();
    let mut output_chars = 0_usize;

    for page in document.pages().iter() {
        budget.check_deadline()?;
        let page_started = Instant::now();
        let crop = page
            .boundaries()
            .crop()
            .map(|boundary| boundary.bounds)
            .unwrap_or_else(|_| {
                PdfRect::new_from_values(0.0, 0.0, page.height().value, page.width().value)
            });
        let page_text = page
            .text()
            .map_err(|_| ParserError::corrupted("pdfium could not load page text"))?;
        metrics.page_text_load += page_started.elapsed();
        let collect_started = Instant::now();
        let mut previous_was_space = true;
        let mut page_has_text = false;
        for character in page_text.chars().iter() {
            budget.check_deadline()?;
            metrics.characters_seen = metrics.characters_seen.saturating_add(1);
            let Some(value) = character.unicode_char() else {
                continue;
            };
            if matches!(value, '\r' | '\n' | '\u{2028}' | '\u{2029}') {
                if page_has_text && !output.ends_with('\n') {
                    while output.ends_with(' ') {
                        output.pop();
                        output_chars = output_chars.saturating_sub(1);
                    }
                    if output_chars >= MAX_EXTRACTED_TEXT_CHARS {
                        return Err(ParserError::resource_exhausted(
                            "pdf extracted text exceeds parser budget",
                        ));
                    }
                    output.push('\n');
                    output_chars += 1;
                }
                previous_was_space = true;
                continue;
            }
            if !character_is_visible(&character, &crop) {
                continue;
            }
            let value = normalize_character(value);
            if value == '\0' {
                continue;
            }
            let is_space = value.is_whitespace();
            if is_space && previous_was_space {
                continue;
            }
            let emitted = if is_space { ' ' } else { value };
            if output_chars >= MAX_EXTRACTED_TEXT_CHARS {
                return Err(ParserError::resource_exhausted(
                    "pdf extracted text exceeds parser budget",
                ));
            }
            output.push(emitted);
            output_chars += 1;
            previous_was_space = is_space;
            page_has_text |= !is_space;
        }
        if page_has_text {
            while output.ends_with(' ') {
                output.pop();
                output_chars = output_chars.saturating_sub(1);
            }
            if !output.ends_with('\n') {
                if output_chars >= MAX_EXTRACTED_TEXT_CHARS {
                    return Err(ParserError::resource_exhausted(
                        "pdf extracted text exceeds parser budget",
                    ));
                }
                output.push('\n');
                output_chars += 1;
            }
        }
        metrics.character_iteration += collect_started.elapsed();
    }
    while output.ends_with(char::is_whitespace) {
        output.pop();
    }
    metrics.characters_emitted = output.chars().count() as u64;
    metrics.output_bytes = output.len() as u64;
    Ok(PdfiumTextExtraction {
        text: output,
        page_count,
    })
}

fn character_is_visible(
    character: &pdfium_render::prelude::PdfPageTextChar<'_>,
    crop: &PdfRect,
) -> bool {
    let has_visible_fill = character
        .fill_color()
        .is_ok_and(|color| color.alpha() >= MIN_VISIBLE_ALPHA);
    let has_visible_stroke = character
        .stroke_color()
        .is_ok_and(|color| color.alpha() >= MIN_VISIBLE_ALPHA);
    let visibly_painted = match character.render_mode() {
        Ok(
            PdfPageTextRenderMode::FilledUnstroked | PdfPageTextRenderMode::FilledUnstrokedClipping,
        ) => has_visible_fill,
        Ok(
            PdfPageTextRenderMode::StrokedUnfilled | PdfPageTextRenderMode::StrokedUnfilledClipping,
        ) => has_visible_stroke,
        Ok(
            PdfPageTextRenderMode::FilledThenStroked
            | PdfPageTextRenderMode::FilledThenStrokedClipping,
        ) => has_visible_fill || has_visible_stroke,
        Ok(
            PdfPageTextRenderMode::Unknown
            | PdfPageTextRenderMode::Invisible
            | PdfPageTextRenderMode::InvisibleClipping,
        )
        | Err(_) => false,
    };
    if !visibly_painted {
        return false;
    }
    character
        .loose_bounds()
        .is_ok_and(|bounds| crop.does_overlap(&bounds))
}

fn normalize_character(value: char) -> char {
    match value {
        '\u{00a0}' | '\u{2000}'..='\u{200b}' | '\t' => ' ',
        value if value.is_control() => '\0',
        value => value,
    }
}

fn text_quality_is_acceptable(text: &str) -> bool {
    let chars = text.chars().collect::<Vec<_>>();
    if chars.iter().filter(|value| !value.is_whitespace()).count() < MIN_TEXT_SIGNAL_CHARS {
        return false;
    }
    let total = chars.len().max(1) as f64;
    let controls = chars
        .iter()
        .filter(|value| value.is_control() && !matches!(value, '\n' | '\r' | '\t'))
        .count() as f64;
    let suspicious = chars
        .iter()
        .filter(|value| {
            **value == '\u{fffd}'
                || ('\u{e000}'..='\u{f8ff}').contains(value)
                || ('\u{f0000}'..='\u{ffffd}').contains(value)
        })
        .count() as f64;
    controls / total <= MAX_CONTROL_RATIO
        && suspicious / total <= MAX_SUSPICIOUS_RATIO
        && duplicate_line_ratio(text) <= MAX_DUPLICATE_LINE_RATIO
        && repeated_long_token_ratio(text) <= MAX_REPEATED_LONG_TOKEN_RATIO
        && !contains_mojibake_marker(text)
        && !contains_high_entropy_run(text)
}

fn contains_mojibake_marker(text: &str) -> bool {
    ["þÿ", "ÿþ", "ï»¿", "ï¿½"]
        .into_iter()
        .any(|marker| text.contains(marker))
}

fn duplicate_line_ratio(text: &str) -> f64 {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| line.chars().count() >= 8)
        .collect::<Vec<_>>();
    if lines.len() < 4 {
        return 0.0;
    }
    let mut counts = BTreeMap::<&str, usize>::new();
    for line in &lines {
        *counts.entry(line).or_default() += 1;
    }
    counts.values().copied().max().unwrap_or(0) as f64 / lines.len() as f64
}

fn repeated_long_token_ratio(text: &str) -> f64 {
    const MIN_TOKEN_CHARS: usize = 16;
    const MIN_REPETITIONS: usize = 4;

    let mut counts = BTreeMap::<String, (usize, usize)>::new();
    let mut token = String::new();
    let mut total_signal_chars = 0_usize;
    let flush = |token: &mut String, counts: &mut BTreeMap<String, (usize, usize)>| {
        let length = token.chars().count();
        if length >= MIN_TOKEN_CHARS {
            let entry = counts.entry(std::mem::take(token)).or_insert((0, length));
            entry.0 = entry.0.saturating_add(1);
        } else {
            token.clear();
        }
    };
    for value in text.chars() {
        if value.is_alphanumeric() || matches!(value, '+' | '/' | '_' | '-' | '=') {
            token.push(value);
            total_signal_chars = total_signal_chars.saturating_add(1);
        } else {
            flush(&mut token, &mut counts);
        }
    }
    flush(&mut token, &mut counts);
    if total_signal_chars == 0 {
        return 0.0;
    }
    counts
        .values()
        .filter(|(occurrences, _)| *occurrences >= MIN_REPETITIONS)
        .map(|(occurrences, length)| occurrences.saturating_mul(*length))
        .max()
        .unwrap_or(0) as f64
        / total_signal_chars as f64
}

fn contains_high_entropy_run(text: &str) -> bool {
    text.split_whitespace().any(|token| {
        if token.len() < 96
            || !token.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'_' | b'-' | b'=')
            })
        {
            return false;
        }
        let mut frequencies = [0_u16; 256];
        for byte in token.bytes() {
            frequencies[byte as usize] = frequencies[byte as usize].saturating_add(1);
        }
        let length = token.len() as f64;
        let entropy = frequencies
            .into_iter()
            .filter(|count| *count > 0)
            .map(|count| {
                let probability = f64::from(count) / length;
                -probability * probability.log2()
            })
            .sum::<f64>();
        entropy >= 4.5
    })
}
