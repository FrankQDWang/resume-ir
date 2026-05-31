pub fn crate_name() -> &'static str {
    "sectionizer"
}

use std::fmt;

use core_domain::SectionType;

const DEFAULT_MAX_CHARS: usize = 1_200;

#[derive(Clone, PartialEq)]
pub struct SectionChunk {
    pub section_type: SectionType,
    pub order_no: u32,
    pub text: String,
    pub char_start: usize,
    pub char_end: usize,
    pub confidence: f32,
}

impl fmt::Debug for SectionChunk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SectionChunk")
            .field("section_type", &self.section_type)
            .field("order_no", &self.order_no)
            .field("text", &"<redacted>")
            .field("char_start", &self.char_start)
            .field("char_end", &self.char_end)
            .field("confidence", &self.confidence)
            .finish()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Sectionizer {
    max_chars: usize,
}

impl Default for Sectionizer {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_MAX_CHARS,
        }
    }
}

impl Sectionizer {
    pub fn with_max_chars(max_chars: usize) -> Self {
        Self {
            max_chars: max_chars.max(1),
        }
    }

    pub fn sectionize(&self, clean_text: &str) -> Vec<SectionChunk> {
        let lines = indexed_lines(clean_text);
        let heading_positions = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                detect_heading(line.text).map(|section_type| (index, section_type))
            })
            .collect::<Vec<_>>();

        if heading_positions.is_empty() {
            return self.fallback_chunks(clean_text);
        }

        let mut sections = Vec::new();
        for (order_no, (heading_index, section_type)) in heading_positions.iter().enumerate() {
            let start = lines[*heading_index].start;
            let next_heading_index = heading_positions
                .get(order_no + 1)
                .map(|(line_index, _)| *line_index)
                .unwrap_or(lines.len());
            let end = lines[..next_heading_index]
                .iter()
                .rev()
                .find(|line| !line.text.trim().is_empty())
                .map(|line| line.end)
                .unwrap_or(start);

            if start >= end {
                continue;
            }

            sections.push(SectionChunk {
                section_type: section_type.clone(),
                order_no: order_no as u32,
                text: clean_text[start..end].to_string(),
                char_start: lines[*heading_index].char_start,
                char_end: lines[..next_heading_index]
                    .iter()
                    .rev()
                    .find(|line| !line.text.trim().is_empty())
                    .map(|line| line.char_end)
                    .unwrap_or(lines[*heading_index].char_start),
                confidence: 0.92,
            });
        }

        sections
    }

    fn fallback_chunks(&self, clean_text: &str) -> Vec<SectionChunk> {
        let paragraphs = paragraphs(clean_text);
        let mut sections = Vec::new();
        let mut current_start = None;
        let mut current_end = 0_usize;
        let mut current_text = String::new();
        let mut current_chars = 0_usize;

        for paragraph in paragraphs {
            if paragraph.char_len() > self.max_chars {
                if !current_text.is_empty() {
                    push_fallback_chunk(
                        &mut sections,
                        current_start.unwrap_or(0),
                        current_end,
                        &current_text,
                    );
                    current_start = None;
                    current_text.clear();
                    current_chars = 0;
                }

                for piece in split_paragraph_by_chars(paragraph, self.max_chars) {
                    push_fallback_chunk(
                        &mut sections,
                        piece.char_start,
                        piece.char_end,
                        piece.text,
                    );
                }
                continue;
            }

            let would_chars =
                current_chars + usize::from(!current_text.is_empty()) * 2 + paragraph.char_len();
            if !current_text.is_empty() && would_chars > self.max_chars {
                push_fallback_chunk(
                    &mut sections,
                    current_start.unwrap_or(0),
                    current_end,
                    &current_text,
                );
                current_start = None;
                current_text.clear();
                current_chars = 0;
            }

            if current_text.is_empty() {
                current_start = Some(paragraph.char_start);
            } else {
                current_text.push_str("\n\n");
                current_chars += 2;
            }
            current_text.push_str(paragraph.text);
            current_chars += paragraph.char_len();
            current_end = paragraph.char_end;
        }

        if !current_text.is_empty() {
            push_fallback_chunk(
                &mut sections,
                current_start.unwrap_or(0),
                current_end,
                &current_text,
            );
        }

        sections
    }
}

#[derive(Clone, Copy)]
struct IndexedLine<'a> {
    text: &'a str,
    start: usize,
    end: usize,
    char_start: usize,
    char_end: usize,
}

fn indexed_lines(text: &str) -> Vec<IndexedLine<'_>> {
    let mut lines = Vec::new();
    let mut start = 0_usize;
    let mut char_start = 0_usize;

    for line in text.split_inclusive('\n') {
        let raw_end = start + line.len();
        let end = raw_end - usize::from(line.ends_with('\n'));
        let line_without_newline = &text[start..end];
        let char_end = char_start + line_without_newline.chars().count();
        lines.push(IndexedLine {
            text: &text[start..end],
            start,
            end,
            char_start,
            char_end,
        });
        char_start += line.chars().count();
        start = raw_end;
    }

    if start < text.len() || text.is_empty() {
        lines.push(IndexedLine {
            text: &text[start..],
            start,
            end: text.len(),
            char_start,
            char_end: char_start + text[start..].chars().count(),
        });
    }

    lines
}

fn detect_heading(text: &str) -> Option<SectionType> {
    if text.contains('|') {
        return None;
    }

    let normalized = normalize_heading(text);
    if normalized.is_empty() || normalized.len() > 40 {
        return None;
    }

    if contains_any(&normalized, &["contact", "联系方式", "联系"]) {
        Some(SectionType::Contact)
    } else if contains_any(
        &normalized,
        &[
            "profile",
            "summary",
            "self",
            "自我评价",
            "个人简介",
            "求职意向",
        ],
    ) {
        Some(SectionType::Profile)
    } else if contains_any(&normalized, &["education", "教育", "学校", "学历"]) {
        Some(SectionType::Education)
    } else if contains_any(&normalized, &["project", "项目"]) {
        Some(SectionType::Project)
    } else if contains_any(&normalized, &["experience", "工作经历", "工作经验", "经历"]) {
        Some(SectionType::Experience)
    } else if contains_any(&normalized, &["skill", "skills", "技能", "技术栈"]) {
        Some(SectionType::Skill)
    } else if contains_any(
        &normalized,
        &["certificate", "certification", "证书", "认证", "资格"],
    ) {
        Some(SectionType::Certificate)
    } else {
        None
    }
}

fn normalize_heading(text: &str) -> String {
    text.trim()
        .trim_matches(|character: char| {
            character.is_ascii_punctuation()
                || matches!(character, '：' | '，' | '。' | '、' | '；' | ' ' | '\t')
        })
        .to_ascii_lowercase()
}

fn contains_any(normalized: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| normalized.contains(needle))
}

#[derive(Clone, Copy)]
struct Paragraph<'a> {
    text: &'a str,
    start: usize,
    char_start: usize,
    char_end: usize,
}

impl Paragraph<'_> {
    fn char_len(&self) -> usize {
        self.char_end - self.char_start
    }
}

fn paragraphs(text: &str) -> Vec<Paragraph<'_>> {
    let mut paragraphs = Vec::new();
    let mut cursor = 0_usize;

    for block in text.split("\n\n") {
        let block_start = cursor;
        let block_end = block_start + block.len();
        let trimmed_start = block
            .char_indices()
            .find(|(_, character)| !character.is_whitespace())
            .map(|(offset, _)| block_start + offset);
        if let Some(start) = trimmed_start {
            let end = block
                .char_indices()
                .rev()
                .find(|(_, character)| !character.is_whitespace())
                .map(|(offset, character)| block_start + offset + character.len_utf8())
                .unwrap_or(block_end);
            paragraphs.push(Paragraph {
                text: &text[start..end],
                start,
                char_start: text[..start].chars().count(),
                char_end: text[..end].chars().count(),
            });
        }
        cursor = block_end + 2;
    }

    paragraphs
}

fn split_paragraph_by_chars<'a>(paragraph: Paragraph<'a>, max_chars: usize) -> Vec<Paragraph<'a>> {
    let mut pieces = Vec::new();
    let mut piece_byte_start = paragraph.start;
    let mut piece_char_start = paragraph.char_start;
    let mut chars_in_piece = 0_usize;
    let mut last_byte_end = paragraph.start;

    for (relative_start, character) in paragraph.text.char_indices() {
        let absolute_start = paragraph.start + relative_start;
        if chars_in_piece == max_chars {
            pieces.push(Paragraph {
                text: &paragraph.text
                    [piece_byte_start - paragraph.start..last_byte_end - paragraph.start],
                start: piece_byte_start,
                char_start: piece_char_start,
                char_end: piece_char_start + chars_in_piece,
            });
            piece_byte_start = absolute_start;
            piece_char_start += chars_in_piece;
            chars_in_piece = 0;
        }

        chars_in_piece += 1;
        last_byte_end = absolute_start + character.len_utf8();
    }

    if chars_in_piece > 0 {
        pieces.push(Paragraph {
            text: &paragraph.text
                [piece_byte_start - paragraph.start..last_byte_end - paragraph.start],
            start: piece_byte_start,
            char_start: piece_char_start,
            char_end: piece_char_start + chars_in_piece,
        });
    }

    pieces
}

fn push_fallback_chunk(
    sections: &mut Vec<SectionChunk>,
    char_start: usize,
    char_end: usize,
    text: &str,
) {
    sections.push(SectionChunk {
        section_type: SectionType::Other("chunk".to_string()),
        order_no: sections.len() as u32,
        text: text.to_string(),
        char_start,
        char_end,
        confidence: 0.45,
    });
}
