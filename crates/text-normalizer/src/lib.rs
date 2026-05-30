#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanText {
    pub text: String,
    offset_map: Vec<usize>,
}

impl CleanText {
    #[must_use]
    pub fn original_byte_offset(&self, clean_char_index: usize) -> Option<usize> {
        self.offset_map.get(clean_char_index).copied()
    }
}

pub fn normalize_text(input: &str) -> CleanText {
    let mut text = String::new();
    let mut offset_map = Vec::new();

    for (start, end) in line_ranges(input) {
        let line = &input[start..end];
        let cleaned = clean_line(line, start);
        if cleaned.is_empty() {
            continue;
        }
        let line_text: String = cleaned.iter().map(|(char, _)| *char).collect();
        if is_header_footer(&line_text) {
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
            offset_map.push(cleaned[0].1);
        }
        for (char, offset) in cleaned {
            text.push(char);
            offset_map.push(offset);
        }
    }

    CleanText { text, offset_map }
}

fn line_ranges(input: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let bytes = input.as_bytes();
    let mut start = 0;
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'\r' => {
                ranges.push((start, index));
                index += if bytes.get(index + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                start = index;
            }
            b'\n' => {
                ranges.push((start, index));
                index += 1;
                start = index;
            }
            _ => index += 1,
        }
    }
    ranges.push((start, input.len()));
    ranges
}

fn clean_line(line: &str, line_start: usize) -> Vec<(char, usize)> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let Some(first) = chars.iter().position(|(_, char)| !char.is_whitespace()) else {
        return Vec::new();
    };
    let Some(last) = chars.iter().rposition(|(_, char)| !char.is_whitespace()) else {
        return Vec::new();
    };

    let mut output = Vec::new();
    let mut pending_space = false;
    for (offset, char) in chars[first..=last].iter().copied() {
        if char.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !output.is_empty() {
                output.push((' ', line_start + offset));
            }
            output.push((char, line_start + offset));
            pending_space = false;
        }
    }
    output
}

fn is_header_footer(line: &str) -> bool {
    let trimmed = line.trim();
    let lowercase = trimmed.to_ascii_lowercase();
    lowercase.starts_with("page ") || lowercase.starts_with("page:") || trimmed.starts_with("页码")
}

#[must_use]
pub fn crate_name() -> &'static str {
    "text-normalizer"
}
