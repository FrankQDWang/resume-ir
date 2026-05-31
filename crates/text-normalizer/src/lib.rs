//! Text cleanup and offset mapping for parsed resume text.

use std::collections::{HashMap, HashSet};
use std::fmt;

/// Cleaned text plus a byte offset map back to the original input.
#[derive(Clone, Eq, PartialEq)]
pub struct NormalizedText {
    text: String,
    map: OffsetMap,
}

impl NormalizedText {
    /// Returns cleaned text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the byte offset map from cleaned text to original text.
    #[must_use]
    pub fn map(&self) -> &OffsetMap {
        &self.map
    }
}

impl fmt::Debug for NormalizedText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalizedText")
            .field("text", &"[redacted normalized text]")
            .field("text_len", &self.text.len())
            .field("map", &self.map)
            .finish()
    }
}

/// Byte offset mapping from normalized output back to original input.
#[derive(Clone, Eq, PartialEq)]
pub struct OffsetMap {
    normalized_to_original_start: Vec<usize>,
    normalized_to_original_end: Vec<usize>,
}

impl OffsetMap {
    /// Returns the original byte offset for a normalized byte offset.
    #[must_use]
    pub fn original_offset_for(&self, normalized_offset: usize) -> Option<usize> {
        self.normalized_to_original_start
            .get(normalized_offset)
            .copied()
    }

    /// Maps a normalized byte span to an original byte span.
    #[must_use]
    pub fn normalized_span_to_original(
        &self,
        normalized_start: usize,
        normalized_end: usize,
    ) -> Option<(usize, usize)> {
        if normalized_start > normalized_end {
            return None;
        }

        let original_start = self.original_offset_for(normalized_start)?;
        let original_end = if normalized_start == normalized_end {
            original_start
        } else {
            let last_normalized_byte = normalized_end.checked_sub(1)?;
            self.normalized_to_original_end
                .get(last_normalized_byte)
                .copied()?
        };

        Some((original_start, original_end))
    }
}

impl fmt::Debug for OffsetMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OffsetMap")
            .field("entries", &self.normalized_to_original_start.len())
            .finish()
    }
}

#[derive(Clone, Copy)]
struct RawLine {
    content_start: usize,
    content_end: usize,
}

/// Normalizes parsed text with conservative whitespace and page-line cleanup.
#[must_use]
pub fn normalize_text(input: &str) -> NormalizedText {
    let pages = parse_pages(input);
    let removed_lines = repeated_page_lines(input, &pages);
    let mut output = String::new();
    let mut start_map = Vec::new();
    let mut end_map = Vec::new();
    let mut pending_blank = false;
    let mut separator_original = 0;

    for page in &pages {
        for line in page {
            let trimmed = input[line.content_start..line.content_end].trim();
            if trimmed.is_empty() {
                if !output.is_empty() {
                    pending_blank = true;
                    separator_original = line.content_start;
                }
                continue;
            }

            if removed_lines.contains(&line.content_start) || is_page_marker(trimmed) {
                continue;
            }

            let collapsed = collapse_line(input, line.content_start, line.content_end);
            if collapsed.is_empty() {
                if !output.is_empty() {
                    pending_blank = true;
                    separator_original = line.content_start;
                }
                continue;
            }

            if !output.is_empty() {
                emit_char(
                    '\n',
                    separator_original,
                    separator_original,
                    &mut output,
                    &mut start_map,
                    &mut end_map,
                );
                if pending_blank {
                    emit_char(
                        '\n',
                        separator_original,
                        separator_original,
                        &mut output,
                        &mut start_map,
                        &mut end_map,
                    );
                }
            }
            pending_blank = false;

            for (character, original_start, original_end) in collapsed {
                emit_char(
                    character,
                    original_start,
                    original_end,
                    &mut output,
                    &mut start_map,
                    &mut end_map,
                );
            }
            separator_original = line.content_end;
        }
    }

    start_map.push(input.len());
    end_map.push(input.len());
    NormalizedText {
        text: output,
        map: OffsetMap {
            normalized_to_original_start: start_map,
            normalized_to_original_end: end_map,
        },
    }
}

fn emit_char(
    character: char,
    original_start: usize,
    original_end: usize,
    output: &mut String,
    start_map: &mut Vec<usize>,
    end_map: &mut Vec<usize>,
) {
    for _ in 0..character.len_utf8() {
        start_map.push(original_start);
        end_map.push(original_end);
    }
    output.push(character);
}

fn parse_pages(input: &str) -> Vec<Vec<RawLine>> {
    let mut pages = Vec::new();
    let mut page_start = 0;

    for (index, character) in input.char_indices() {
        if character == '\u{000c}' {
            pages.push(parse_lines(input, page_start, index));
            page_start = index + character.len_utf8();
        }
    }

    pages.push(parse_lines(input, page_start, input.len()));
    pages
}

fn parse_lines(input: &str, start: usize, end: usize) -> Vec<RawLine> {
    let mut lines = Vec::new();
    let mut line_start = start;
    let mut cursor = start;

    while cursor < end {
        let Some(character) = input[cursor..end].chars().next() else {
            break;
        };
        let next = cursor + character.len_utf8();
        if character == '\n' || character == '\r' {
            lines.push(RawLine {
                content_start: line_start,
                content_end: cursor,
            });
            if character == '\r' && input[next..end].starts_with('\n') {
                cursor = next + 1;
                line_start = cursor;
            } else {
                cursor = next;
                line_start = cursor;
            }
        } else {
            cursor = next;
        }
    }

    if line_start < end {
        lines.push(RawLine {
            content_start: line_start,
            content_end: end,
        });
    }

    lines
}

fn repeated_page_lines(input: &str, pages: &[Vec<RawLine>]) -> HashSet<usize> {
    let mut counts = HashMap::<String, usize>::new();

    for page in pages {
        for line in first_and_last_content_lines(input, page) {
            let trimmed = input[line.content_start..line.content_end].trim();
            if should_count_as_repeated_page_line(trimmed) {
                *counts.entry(trimmed.to_owned()).or_default() += 1;
            }
        }
    }

    let repeated = counts
        .into_iter()
        .filter_map(|(line, count)| (count > 1).then_some(line))
        .collect::<HashSet<_>>();

    pages
        .iter()
        .flat_map(|page| first_and_last_content_lines(input, page))
        .filter_map(|line| {
            let trimmed = input[line.content_start..line.content_end].trim();
            repeated.contains(trimmed).then_some(line.content_start)
        })
        .collect()
}

fn first_and_last_content_lines<'a>(input: &str, page: &'a [RawLine]) -> Vec<&'a RawLine> {
    let content_lines = page
        .iter()
        .filter(|line| {
            let trimmed = input[line.content_start..line.content_end].trim();
            !trimmed.is_empty() && !is_page_marker(trimmed)
        })
        .collect::<Vec<_>>();

    match (content_lines.first(), content_lines.last()) {
        (Some(first), Some(last)) if first.content_start != last.content_start => {
            vec![*first, *last]
        }
        (Some(first), _) => vec![*first],
        _ => Vec::new(),
    }
}

fn should_count_as_repeated_page_line(line: &str) -> bool {
    !line.is_empty() && line.chars().count() <= 80
}

fn is_page_marker(line: &str) -> bool {
    let compact = line
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();

    if let Some(number) = compact.strip_prefix("page") {
        return !number.is_empty() && number.chars().all(|character| character.is_ascii_digit());
    }

    if compact.starts_with('第') && compact.ends_with('页') {
        let inner = &compact['第'.len_utf8()..compact.len() - '页'.len_utf8()];
        return !inner.is_empty() && inner.chars().all(|character| character.is_ascii_digit());
    }

    let stripped = compact.trim_matches('-');
    !stripped.is_empty() && stripped.chars().all(|character| character.is_ascii_digit())
}

fn collapse_line(input: &str, start: usize, end: usize) -> Vec<(char, usize, usize)> {
    let mut collapsed = Vec::new();
    let mut pending_space = None;

    for (relative_offset, character) in input[start..end].char_indices() {
        let original_offset = start + relative_offset;
        if character.is_whitespace() {
            if !collapsed.is_empty() && pending_space.is_none() {
                pending_space = Some((original_offset, original_offset + character.len_utf8()));
            }
            continue;
        }

        if let Some((space_start, space_end)) = pending_space.take() {
            collapsed.push((' ', space_start, space_end));
        }
        collapsed.push((
            character,
            original_offset,
            original_offset + character.len_utf8(),
        ));
    }

    collapsed
}
