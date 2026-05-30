use core_domain::SectionType;

const DEFAULT_MAX_CHARS: usize = 800;

#[derive(Clone, Debug, PartialEq)]
pub struct SectionChunk {
    pub section_type: SectionType,
    pub text: String,
    pub char_start: usize,
    pub char_end: usize,
    pub confidence: f32,
}

pub fn sectionize(text: &str) -> Vec<SectionChunk> {
    sectionize_with_max_len(text, DEFAULT_MAX_CHARS)
}

pub fn sectionize_with_max_len(text: &str, max_chars: usize) -> Vec<SectionChunk> {
    let headed = sectionize_headings(text);
    if headed.is_empty() {
        fallback_chunks(text, max_chars)
    } else {
        headed
    }
}

fn sectionize_headings(text: &str) -> Vec<SectionChunk> {
    let mut sections = Vec::new();
    let mut current_type: Option<SectionType> = None;
    let mut current_text = String::new();
    let mut current_start = 0;
    let mut byte_offset = 0;

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(section_type) = heading_type(trimmed) {
            push_current(
                &mut sections,
                current_type,
                &mut current_text,
                current_start,
                byte_offset,
            );
            current_type = Some(section_type);
            current_start = byte_offset + line.len() + 1;
        } else if current_type.is_some() && !trimmed.is_empty() {
            if !current_text.is_empty() {
                current_text.push('\n');
            }
            current_text.push_str(trimmed);
        }
        byte_offset += line.len() + 1;
    }

    push_current(
        &mut sections,
        current_type,
        &mut current_text,
        current_start,
        text.len(),
    );
    sections
}

fn push_current(
    sections: &mut Vec<SectionChunk>,
    section_type: Option<SectionType>,
    current_text: &mut String,
    char_start: usize,
    char_end: usize,
) {
    let Some(section_type) = section_type else {
        return;
    };
    let text = current_text.trim().to_owned();
    if text.is_empty() {
        current_text.clear();
        return;
    }
    sections.push(SectionChunk {
        section_type,
        text,
        char_start,
        char_end,
        confidence: 0.9,
    });
    current_text.clear();
}

fn heading_type(line: &str) -> Option<SectionType> {
    match line
        .trim_end_matches(':')
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "education" | "教育经历" | "教育背景" => Some(SectionType::Education),
        "experience" | "work experience" | "工作经历" => Some(SectionType::Experience),
        "project" | "projects" | "项目经历" => Some(SectionType::Project),
        "skills" | "skill" | "技能" => Some(SectionType::Skill),
        "certificates" | "certificate" | "证书" => Some(SectionType::Certificate),
        "contact" | "联系方式" => Some(SectionType::Contact),
        _ => None,
    }
}

fn fallback_chunks(text: &str, max_chars: usize) -> Vec<SectionChunk> {
    let mut chunks = Vec::new();
    let mut search_start = 0;

    for paragraph in text
        .split("\n\n")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        let paragraph_start = text[search_start..]
            .find(paragraph)
            .map_or(search_start, |offset| search_start + offset);
        for (chunk, relative_start) in split_by_max_chars(paragraph, max_chars) {
            let char_start = paragraph_start + relative_start;
            let char_end = char_start + chunk.len();
            chunks.push(SectionChunk {
                section_type: SectionType::Other,
                text: chunk,
                char_start,
                char_end,
                confidence: 0.5,
            });
        }
        search_start = paragraph_start + paragraph.len();
    }

    chunks
}

fn split_by_max_chars(text: &str, max_chars: usize) -> Vec<(String, usize)> {
    let max_chars = max_chars.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_start = 0;

    for (byte_offset, char) in text.char_indices() {
        if current.is_empty() {
            current_start = byte_offset;
        }
        current.push(char);
        if current.chars().count() == max_chars {
            chunks.push((std::mem::take(&mut current), current_start));
        }
    }
    if !current.is_empty() {
        chunks.push((current, current_start));
    }
    chunks
}

#[must_use]
pub fn crate_name() -> &'static str {
    "sectionizer"
}
