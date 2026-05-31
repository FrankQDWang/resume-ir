//! Resume text sectioning helpers.

use core_domain::SectionType;
use std::fmt;

const DEFAULT_MAX_CHARS: usize = 600;

/// Options for fallback sectioning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SectionizeOptions {
    max_chars: usize,
}

impl SectionizeOptions {
    /// Creates options with a maximum fallback chunk size in Unicode scalar values.
    #[must_use]
    pub fn new(max_chars: usize) -> Self {
        Self {
            max_chars: max_chars.max(1),
        }
    }

    /// Returns the configured fallback chunk size.
    #[must_use]
    pub fn max_chars(&self) -> usize {
        self.max_chars
    }
}

impl Default for SectionizeOptions {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CHARS)
    }
}

/// A local text section with semantic type, span, and confidence.
#[derive(Clone, PartialEq)]
pub struct SectionChunk {
    section_type: SectionType,
    text: String,
    char_start: u32,
    char_end: u32,
    confidence: f32,
}

impl SectionChunk {
    /// Returns the semantic section type.
    #[must_use]
    pub fn section_type(&self) -> SectionType {
        self.section_type.clone()
    }

    /// Returns the local section text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the section start as a character offset in the input text.
    #[must_use]
    pub fn char_start(&self) -> u32 {
        self.char_start
    }

    /// Returns the section end as a character offset in the input text.
    #[must_use]
    pub fn char_end(&self) -> u32 {
        self.char_end
    }

    /// Returns sectioning confidence.
    #[must_use]
    pub fn confidence(&self) -> f32 {
        self.confidence
    }
}

impl fmt::Debug for SectionChunk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SectionChunk")
            .field("section_type", &self.section_type)
            .field("text", &"[redacted section text]")
            .field("char_start", &self.char_start)
            .field("char_end", &self.char_end)
            .field("confidence", &self.confidence)
            .finish()
    }
}

#[derive(Clone, Copy)]
struct LineRange<'a> {
    text: &'a str,
    byte_start: usize,
    byte_end: usize,
    char_start: usize,
}

#[derive(Clone, Copy)]
struct ParagraphRange {
    byte_start: usize,
    byte_end: usize,
    char_start: usize,
    char_end: usize,
}

/// Splits text into semantic sections when headings exist, otherwise fallback chunks.
#[must_use]
pub fn sectionize(text: &str) -> Vec<SectionChunk> {
    sectionize_with_options(text, SectionizeOptions::default())
}

/// Splits text into sections using caller-supplied fallback options.
#[must_use]
pub fn sectionize_with_options(text: &str, options: SectionizeOptions) -> Vec<SectionChunk> {
    let lines = line_ranges(text);
    let headings = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| heading_type(line.text.trim()).map(|kind| (index, kind)))
        .collect::<Vec<_>>();

    if headings.is_empty() {
        fallback_chunks(text, options)
    } else {
        heading_chunks(text, &lines, &headings)
    }
}

fn heading_chunks(
    text: &str,
    lines: &[LineRange<'_>],
    headings: &[(usize, SectionType)],
) -> Vec<SectionChunk> {
    headings
        .iter()
        .enumerate()
        .filter_map(|(heading_order, (line_index, section_type))| {
            let start = lines[*line_index].byte_start;
            let end = headings
                .get(heading_order + 1)
                .map_or(text.len(), |(next_index, _)| lines[*next_index].byte_start);
            make_chunk(
                text,
                section_type.clone(),
                start,
                end,
                lines[*line_index].char_start,
                0.90,
            )
        })
        .collect()
}

fn fallback_chunks(text: &str, options: SectionizeOptions) -> Vec<SectionChunk> {
    let paragraphs = paragraph_ranges(text);
    let mut chunks = Vec::new();
    let mut current: Option<ParagraphRange> = None;

    for paragraph in paragraphs {
        let paragraph_len = paragraph.char_end.saturating_sub(paragraph.char_start);
        if paragraph_len > options.max_chars() {
            if let Some(existing) = current.take() {
                push_fallback_chunk(text, existing, &mut chunks);
            }
            split_large_paragraph(text, paragraph, options.max_chars(), &mut chunks);
            continue;
        }

        match current {
            Some(existing)
                if paragraph.char_end.saturating_sub(existing.char_start)
                    <= options.max_chars() =>
            {
                current = Some(ParagraphRange {
                    byte_start: existing.byte_start,
                    byte_end: paragraph.byte_end,
                    char_start: existing.char_start,
                    char_end: paragraph.char_end,
                });
            }
            Some(existing) => {
                push_fallback_chunk(text, existing, &mut chunks);
                current = Some(paragraph);
            }
            None => current = Some(paragraph),
        }
    }

    if let Some(existing) = current {
        push_fallback_chunk(text, existing, &mut chunks);
    }

    chunks
}

fn push_fallback_chunk(text: &str, paragraph: ParagraphRange, chunks: &mut Vec<SectionChunk>) {
    if let Some(chunk) = make_chunk(
        text,
        SectionType::Other,
        paragraph.byte_start,
        paragraph.byte_end,
        paragraph.char_start,
        0.25,
    ) {
        chunks.push(chunk);
    }
}

fn split_large_paragraph(
    text: &str,
    paragraph: ParagraphRange,
    max_chars: usize,
    chunks: &mut Vec<SectionChunk>,
) {
    let paragraph_text = &text[paragraph.byte_start..paragraph.byte_end];
    let mut chunk_start_byte = paragraph.byte_start;
    let mut chunk_start_char = paragraph.char_start;
    let mut current_chars = 0;

    for (relative_offset, _) in paragraph_text.char_indices() {
        if current_chars == max_chars {
            let chunk_end_byte = paragraph.byte_start + relative_offset;
            if let Some(chunk) = make_chunk(
                text,
                SectionType::Other,
                chunk_start_byte,
                chunk_end_byte,
                chunk_start_char,
                0.25,
            ) {
                chunks.push(chunk);
            }
            chunk_start_byte = chunk_end_byte;
            chunk_start_char += current_chars;
            current_chars = 0;
        }
        current_chars += 1;
    }

    if chunk_start_byte < paragraph.byte_end {
        if let Some(chunk) = make_chunk(
            text,
            SectionType::Other,
            chunk_start_byte,
            paragraph.byte_end,
            chunk_start_char,
            0.25,
        ) {
            chunks.push(chunk);
        }
    }
}

fn make_chunk(
    text: &str,
    section_type: SectionType,
    byte_start: usize,
    byte_end: usize,
    char_start: usize,
    confidence: f32,
) -> Option<SectionChunk> {
    let trimmed = text[byte_start..byte_end].trim_end();
    if trimmed.trim().is_empty() {
        return None;
    }

    let char_end = char_start + trimmed.chars().count();
    Some(SectionChunk {
        section_type,
        text: trimmed.to_owned(),
        char_start: saturating_usize_to_u32(char_start),
        char_end: saturating_usize_to_u32(char_end),
        confidence,
    })
}

fn paragraph_ranges(text: &str) -> Vec<ParagraphRange> {
    let mut paragraphs = Vec::new();
    let mut current_start: Option<(usize, usize)> = None;
    let mut current_end = (0, 0);

    for line in line_ranges(text) {
        if line.text.trim().is_empty() {
            if let Some((byte_start, char_start)) = current_start.take() {
                paragraphs.push(ParagraphRange {
                    byte_start,
                    byte_end: current_end.0,
                    char_start,
                    char_end: current_end.1,
                });
            }
            continue;
        }

        if current_start.is_none() {
            current_start = Some((line.byte_start, line.char_start));
        }
        current_end = (line.byte_end, byte_to_char_index(text, line.byte_end));
    }

    if let Some((byte_start, char_start)) = current_start {
        paragraphs.push(ParagraphRange {
            byte_start,
            byte_end: current_end.0,
            char_start,
            char_end: current_end.1,
        });
    }

    paragraphs
}

fn line_ranges(text: &str) -> Vec<LineRange<'_>> {
    let mut lines = Vec::new();
    let mut byte_start = 0;
    let mut char_start = 0;

    for (current_char, (byte_index, character)) in text.char_indices().enumerate() {
        if character == '\n' {
            lines.push(LineRange {
                text: &text[byte_start..byte_index],
                byte_start,
                byte_end: byte_index,
                char_start,
            });
            byte_start = byte_index + character.len_utf8();
            char_start = current_char + 1;
        }
    }

    lines.push(LineRange {
        text: &text[byte_start..],
        byte_start,
        byte_end: text.len(),
        char_start,
    });

    lines
}

fn heading_type(line: &str) -> Option<SectionType> {
    let normalized = line
        .trim()
        .trim_end_matches(':')
        .trim_end_matches('：')
        .trim()
        .to_ascii_lowercase();

    match normalized.as_str() {
        "contact" | "contacts" | "联系方式" | "联系信息" => Some(SectionType::Contact),
        "summary" | "profile" | "个人简介" | "简介" => Some(SectionType::Profile),
        "education" | "教育" | "教育经历" => Some(SectionType::Education),
        "experience" | "work experience" | "经历" | "工作经历" => {
            Some(SectionType::Experience)
        }
        "projects" | "project" | "project experience" | "项目" | "项目经历" => {
            Some(SectionType::Project)
        }
        "skills" | "skill" | "技能" | "专业技能" => Some(SectionType::Skill),
        "certificates" | "certificate" | "证书" | "证书资质" => {
            Some(SectionType::Certificate)
        }
        _ => None,
    }
}

fn byte_to_char_index(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().count()
}

fn saturating_usize_to_u32(value: usize) -> u32 {
    if value > u32::MAX as usize {
        u32::MAX
    } else {
        value as u32
    }
}
