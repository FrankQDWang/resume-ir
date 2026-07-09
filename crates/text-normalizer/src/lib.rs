pub fn crate_name() -> &'static str {
    "text-normalizer"
}

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

#[derive(Clone, PartialEq, Eq)]
pub struct NormalizedText {
    text: String,
    byte_origins: Vec<Range<usize>>,
}

impl NormalizedText {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn original_span_for_clean_range(&self, range: Range<usize>) -> Option<Range<usize>> {
        if range.start >= range.end || range.end > self.byte_origins.len() {
            return None;
        }

        let mut start = usize::MAX;
        let mut end = 0_usize;
        for origin in self.byte_origins[range]
            .iter()
            .filter(|origin| origin.start < origin.end)
        {
            start = start.min(origin.start);
            end = end.max(origin.end);
        }

        (start != usize::MAX && start <= end).then_some(start..end)
    }
}

impl fmt::Debug for NormalizedText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NormalizedText")
            .field("text", &"<redacted>")
            .field("byte_len", &self.text.len())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TextNormalizer;

impl TextNormalizer {
    pub fn normalize(source: &str) -> NormalizedText {
        let lines = source_lines(source);
        let mut normalized_lines = lines.into_iter().map(normalize_line).collect::<Vec<_>>();

        remove_repeated_headers_and_footers(&mut normalized_lines);
        let normalized_lines = collapse_blank_lines(normalized_lines);
        join_lines(normalized_lines)
    }

    pub fn normalize_text_only(source: &str) -> String {
        let lines = source_text_lines(source);
        let mut normalized_lines = lines
            .into_iter()
            .map(normalize_text_line)
            .collect::<Vec<_>>();

        remove_repeated_text_headers_and_footers(&mut normalized_lines);
        let normalized_lines = collapse_blank_text_lines(normalized_lines);
        normalized_lines.join("\n")
    }
}

#[derive(Clone, Debug, Default)]
struct Line {
    text: String,
    byte_origins: Vec<Range<usize>>,
}

impl Line {
    fn is_blank(&self) -> bool {
        self.text.is_empty()
    }

    fn push_char(&mut self, character: char, origin: Range<usize>) {
        self.text.push(character);
        for _ in 0..character.len_utf8() {
            self.byte_origins.push(origin.clone());
        }
    }
}

fn source_lines(source: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut line = Line::default();
    let mut chars = source.char_indices().peekable();

    while let Some((start, character)) = chars.next() {
        let end = start + character.len_utf8();
        match character {
            '\r' => {
                if matches!(chars.peek(), Some((_, '\n'))) {
                    chars.next();
                }
                lines.push(std::mem::take(&mut line));
            }
            '\n' | '\u{000c}' => lines.push(std::mem::take(&mut line)),
            _ => line.push_char(character, start..end),
        }
    }

    lines.push(line);
    lines
}

fn normalize_line(line: Line) -> Line {
    let chars = line
        .text
        .char_indices()
        .map(|(start, character)| {
            let end = start + character.len_utf8();
            (character, merged_origin(&line.byte_origins[start..end]))
        })
        .collect::<Vec<_>>();

    let first_non_space = chars
        .iter()
        .position(|(character, _)| !is_horizontal_space(*character));
    let Some(first_non_space) = first_non_space else {
        return Line::default();
    };
    let last_non_space = chars
        .iter()
        .rposition(|(character, _)| !is_horizontal_space(*character))
        .unwrap();

    let mut output = Line::default();
    let mut pending_space: Option<Range<usize>> = None;
    for (character, origin) in chars[first_non_space..=last_non_space].iter().cloned() {
        if is_horizontal_space(character) {
            pending_space = Some(match pending_space {
                Some(existing) => existing.start.min(origin.start)..existing.end.max(origin.end),
                None => origin,
            });
            continue;
        }

        if let Some(space_origin) = pending_space.take() {
            output.push_char(' ', space_origin);
        }
        output.push_char(character, origin);
    }

    repair_spaced_ascii_word(output)
}

fn repair_spaced_ascii_word(line: Line) -> Line {
    let tokens = line.text.split(' ').collect::<Vec<_>>();
    if tokens.len() < 4
        || !tokens
            .iter()
            .all(|token| token.len() == 1 && token.as_bytes()[0].is_ascii_alphabetic())
    {
        return line;
    }

    let mut output = Line::default();
    for (start, character) in line.text.char_indices() {
        if character == ' ' {
            continue;
        }

        let end = start + character.len_utf8();
        output.push_char(character, merged_origin(&line.byte_origins[start..end]));
    }
    output
}

fn remove_repeated_headers_and_footers(lines: &mut [Line]) {
    let repeated_short_lines =
        repeated_short_line_flags(lines.iter().map(|line| line.text.as_str()));

    for (line, repeated_short_line) in lines.iter_mut().zip(repeated_short_lines) {
        if line.is_blank() || is_page_number_line(&line.text) {
            line.text.clear();
            line.byte_origins.clear();
            continue;
        }

        if repeated_short_line {
            line.text.clear();
            line.byte_origins.clear();
        }
    }
}

fn collapse_blank_lines(lines: Vec<Line>) -> Vec<Line> {
    let mut collapsed = Vec::new();
    let mut previous_blank = true;

    for line in lines {
        if line.is_blank() {
            if !previous_blank {
                collapsed.push(line);
            }
            previous_blank = true;
        } else {
            collapsed.push(line);
            previous_blank = false;
        }
    }

    while collapsed.last().is_some_and(Line::is_blank) {
        collapsed.pop();
    }

    collapsed
}

fn join_lines(lines: Vec<Line>) -> NormalizedText {
    let mut output = NormalizedText {
        text: String::new(),
        byte_origins: Vec::new(),
    };

    for (index, line) in lines.into_iter().enumerate() {
        if index > 0 {
            output.text.push('\n');
            output.byte_origins.push(0..0);
        }
        output.text.push_str(&line.text);
        output.byte_origins.extend(line.byte_origins);
    }

    output
}

fn source_text_lines(source: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut chars = source.chars().peekable();

    while let Some(character) = chars.next() {
        match character {
            '\r' => {
                if matches!(chars.peek(), Some('\n')) {
                    chars.next();
                }
                lines.push(std::mem::take(&mut line));
            }
            '\n' | '\u{000c}' => lines.push(std::mem::take(&mut line)),
            _ => line.push(character),
        }
    }

    lines.push(line);
    lines
}

fn normalize_text_line(line: String) -> String {
    let Some(first_non_space) = line
        .char_indices()
        .find_map(|(index, character)| (!is_horizontal_space(character)).then_some(index))
    else {
        return String::new();
    };
    let last_non_space = line
        .char_indices()
        .filter_map(|(index, character)| {
            (!is_horizontal_space(character)).then_some(index + character.len_utf8())
        })
        .next_back()
        .unwrap();

    let mut output = String::new();
    let mut pending_space = false;
    for character in line[first_non_space..last_non_space].chars() {
        if is_horizontal_space(character) {
            pending_space = true;
            continue;
        }

        if pending_space && !output.is_empty() {
            output.push(' ');
        }
        pending_space = false;
        output.push(character);
    }

    repair_spaced_ascii_word_text(output)
}

fn repair_spaced_ascii_word_text(line: String) -> String {
    let tokens = line.split(' ').collect::<Vec<_>>();
    if tokens.len() < 4
        || !tokens
            .iter()
            .all(|token| token.len() == 1 && token.as_bytes()[0].is_ascii_alphabetic())
    {
        return line;
    }

    line.chars()
        .filter(|character| *character != ' ')
        .collect::<String>()
}

fn remove_repeated_text_headers_and_footers(lines: &mut [String]) {
    let repeated_short_lines = repeated_short_line_flags(lines.iter().map(String::as_str));

    for (line, repeated_short_line) in lines.iter_mut().zip(repeated_short_lines) {
        if line.is_empty() || is_page_number_line(line) {
            line.clear();
            continue;
        }

        if repeated_short_line {
            line.clear();
        }
    }
}

fn repeated_short_line_flags<'a>(lines: impl Iterator<Item = &'a str> + Clone) -> Vec<bool> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for line in lines.clone() {
        if line.is_empty() || line.len() > 80 {
            continue;
        }
        *counts.entry(line).or_insert(0) += 1;
    }

    lines
        .map(|line| {
            !line.is_empty() && line.len() <= 80 && counts.get(line).is_some_and(|count| *count > 1)
        })
        .collect()
}

#[cfg(test)]
fn repeated_short_line_counts<'a>(lines: impl Iterator<Item = &'a str>) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for line in lines {
        if line.is_empty() || line.len() > 80 {
            continue;
        }
        *counts.entry(line.to_string()).or_insert(0) += 1;
    }
    counts
}

fn collapse_blank_text_lines(lines: Vec<String>) -> Vec<String> {
    let mut collapsed = Vec::new();
    let mut previous_blank = true;

    for line in lines {
        if line.is_empty() {
            if !previous_blank {
                collapsed.push(line);
            }
            previous_blank = true;
        } else {
            collapsed.push(line);
            previous_blank = false;
        }
    }

    while collapsed.last().is_some_and(String::is_empty) {
        collapsed.pop();
    }

    collapsed
}

fn merged_origin(origins: &[Range<usize>]) -> Range<usize> {
    let mut start = usize::MAX;
    let mut end = 0_usize;
    for origin in origins {
        start = start.min(origin.start);
        end = end.max(origin.end);
    }

    if start == usize::MAX {
        0..0
    } else {
        start..end
    }
}

fn is_horizontal_space(character: char) -> bool {
    character.is_whitespace() && character != '\n' && character != '\r'
}

fn is_page_number_line(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.starts_with("page ")
        && lower.contains(" of ")
        && lower.chars().any(|character| character.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_short_line_counts_ignores_blank_and_long_lines() {
        let long_line = "x".repeat(81);
        let lines = [
            "",
            "Confidential Resume",
            "Unique",
            long_line.as_str(),
            "Confidential Resume",
            long_line.as_str(),
        ];

        let counts = repeated_short_line_counts(lines.iter().copied());

        assert_eq!(counts.get("Confidential Resume"), Some(&2));
        assert_eq!(counts.get("Unique"), Some(&1));
        assert_eq!(counts.get(long_line.as_str()), None);
        assert_eq!(counts.get(""), None);
    }

    #[test]
    fn repeated_short_line_flags_marks_only_duplicate_short_lines() {
        let long_line = "x".repeat(81);
        let lines = [
            "",
            "Confidential Resume",
            "Unique",
            long_line.as_str(),
            "Confidential Resume",
            long_line.as_str(),
        ];

        let flags = repeated_short_line_flags(lines.iter().copied());

        assert_eq!(flags, [false, true, false, false, true, false]);
    }
}
