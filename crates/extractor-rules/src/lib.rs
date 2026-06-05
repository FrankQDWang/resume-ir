pub fn crate_name() -> &'static str {
    "extractor-rules"
}

use std::collections::BTreeSet;
use std::fmt;

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldType {
    Name,
    Email,
    Phone,
    DateRange,
    School,
    Degree,
    Company,
    Title,
    Skill,
    Certificate,
    YearsExperience,
}

#[derive(Clone, PartialEq)]
pub struct RuleMatch {
    pub field_type: FieldType,
    pub raw_value: String,
    pub normalized_value: Option<String>,
    pub span_start: usize,
    pub span_end: usize,
    pub confidence: f32,
}

impl fmt::Debug for RuleMatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuleMatch")
            .field("field_type", &self.field_type)
            .field("raw_value", &"<redacted>")
            .field(
                "normalized_value",
                &self.normalized_value.as_ref().map(|_| "<redacted>"),
            )
            .field("span_start", &self.span_start)
            .field("span_end", &self.span_end)
            .field("confidence", &self.confidence)
            .finish()
    }
}

pub fn extract_strong_fields(text: &str) -> Vec<RuleMatch> {
    let mut matches = Vec::new();
    extract_names(text, &mut matches);
    extract_emails(text, &mut matches);
    extract_phones(text, &mut matches);
    extract_numeric_date_ranges(text, &mut matches);
    extract_named_month_date_ranges(text, &mut matches);
    derive_years_experience(text, &mut matches);
    extract_schools(text, &mut matches);
    extract_degrees(text, &mut matches);
    extract_companies(text, &mut matches);
    extract_titles(text, &mut matches);
    extract_skills(text, &mut matches);
    extract_certificates(text, &mut matches);
    matches.sort_by_key(|field| field.span_start);
    matches
}

fn extract_names(text: &str, matches: &mut Vec<RuleMatch>) {
    if extract_labeled_name(text, matches) {
        return;
    }

    extract_heading_name(text, matches);
}

fn extract_labeled_name(text: &str, matches: &mut Vec<RuleMatch>) -> bool {
    let regex = Regex::new(r"(?i)^(?:name|candidate|姓名|候选人)\s*[:：]\s*(?P<name>.+)$").unwrap();
    for (line_start, line) in indexed_lines(text).into_iter().take(12) {
        let leading = line.len() - line.trim_start().len();
        let trimmed_line = line.trim();
        let Some(captures) = regex.captures(trimmed_line) else {
            continue;
        };
        let Some(found) = captures.name("name") else {
            continue;
        };
        if push_name_match(
            matches,
            found.as_str(),
            line_start + leading + found.start(),
            0.93,
        ) {
            return true;
        }
    }

    false
}

fn extract_heading_name(text: &str, matches: &mut Vec<RuleMatch>) {
    for (line_start, line) in indexed_lines(text).into_iter().take(5) {
        let leading = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if push_name_match(matches, trimmed, line_start + leading, 0.84) {
            return;
        }

        if !looks_like_contact_line(trimmed) {
            break;
        }
    }
}

fn push_name_match(
    matches: &mut Vec<RuleMatch>,
    raw: &str,
    raw_span_start: usize,
    confidence: f32,
) -> bool {
    let leading = raw.len() - raw.trim_start().len();
    let trimmed = raw.trim();
    let span_start = raw_span_start + leading;
    let span_end = span_start + trimmed.len();
    let Some(normalized) = normalize_candidate_name(trimmed) else {
        return false;
    };

    matches.push(RuleMatch {
        field_type: FieldType::Name,
        raw_value: trimmed.to_string(),
        normalized_value: Some(normalized),
        span_start,
        span_end,
        confidence,
    });
    true
}

fn normalize_candidate_name(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_matches(|character: char| {
            matches!(character, ',' | ';' | '|' | '/' | '\\' | '，' | '；')
        })
        .trim();
    if value.is_empty() || value.len() > 80 {
        return None;
    }
    if value.contains('@') || value.chars().any(|character| character.is_ascii_digit()) {
        return None;
    }
    if looks_like_section_header(value)
        || looks_like_contact_line(value)
        || looks_like_school(value)
        || looks_like_company(value)
        || normalize_title(value).is_some()
    {
        return None;
    }

    if is_likely_english_name(value) || is_likely_cjk_name(value) {
        Some(
            value
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase(),
        )
    } else {
        None
    }
}

fn is_likely_english_name(value: &str) -> bool {
    let tokens = value.split_whitespace().collect::<Vec<_>>();
    if !(2..=4).contains(&tokens.len()) {
        return false;
    }

    tokens.iter().all(|token| {
        let mut chars = token.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        first.is_ascii_uppercase()
            && chars.all(|character| {
                character.is_ascii_alphabetic()
                    || character == '\''
                    || character == '-'
                    || character == '.'
            })
    })
}

fn is_likely_cjk_name(value: &str) -> bool {
    let count = value.chars().count();
    (2..=6).contains(&count)
        && value
            .chars()
            .all(|character| ('\u{4e00}'..='\u{9fff}').contains(&character))
}

fn looks_like_section_header(value: &str) -> bool {
    let lower = value.to_lowercase();
    matches!(
        lower.as_str(),
        "profile"
            | "summary"
            | "contact"
            | "contacts"
            | "education"
            | "experience"
            | "project"
            | "projects"
            | "skill"
            | "skills"
            | "technical skills"
            | "technical stack"
            | "tech stack"
            | "certificate"
            | "certificates"
            | "certifications"
            | "个人信息"
            | "联系方式"
            | "教育经历"
            | "工作经历"
            | "项目经历"
            | "技能"
            | "专业技能"
            | "技术栈"
            | "证书"
    )
}

fn looks_like_contact_line(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains('@')
        || lower.starts_with("email")
        || lower.starts_with("phone")
        || lower.starts_with("mobile")
        || lower.starts_with("tel")
        || lower.starts_with("邮箱")
        || lower.starts_with("电话")
        || lower.starts_with("手机")
}

fn extract_emails(text: &str, matches: &mut Vec<RuleMatch>) {
    let regex = Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").unwrap();
    for found in regex.find_iter(text) {
        matches.push(RuleMatch {
            field_type: FieldType::Email,
            raw_value: found.as_str().to_string(),
            normalized_value: Some(found.as_str().to_ascii_lowercase()),
            span_start: found.start(),
            span_end: found.end(),
            confidence: 0.99,
        });
    }
}

fn extract_phones(text: &str, matches: &mut Vec<RuleMatch>) {
    let regex =
        Regex::new(r"(?x)(?:\+\d{1,3}[\s.-]*)?(?:\(\d{3}\)|\d{3,4})[\s.-]+\d{3,4}[\s.-]+\d{4}")
            .unwrap();

    for found in regex.find_iter(text) {
        let raw = found.as_str();
        let digits = raw
            .chars()
            .filter(|character| character.is_ascii_digit())
            .collect::<String>();
        let normalized = if raw.trim_start().starts_with('+') {
            normalize_international_phone(&digits)
        } else if digits.len() == 10 {
            Some(format!("+1{digits}"))
        } else if digits.len() == 11 && digits.starts_with('1') {
            Some(format!("+{digits}"))
        } else {
            None
        };

        if let Some(normalized) = normalized {
            matches.push(RuleMatch {
                field_type: FieldType::Phone,
                raw_value: raw.to_string(),
                normalized_value: Some(normalized),
                span_start: found.start(),
                span_end: found.end(),
                confidence: 0.98,
            });
        }
    }
}

fn normalize_international_phone(digits: &str) -> Option<String> {
    if (11..=15).contains(&digits.len()) {
        Some(format!("+{digits}"))
    } else {
        None
    }
}

fn extract_numeric_date_ranges(text: &str, matches: &mut Vec<RuleMatch>) {
    let regex = Regex::new(
        r"(?x)
        \b
        (?P<y1>19\d{2}|20\d{2})[./-](?P<m1>0?[1-9]|1[0-2])
        \s*(?:-|–|—|至|到)\s*
        (?P<y2>19\d{2}|20\d{2})[./-](?P<m2>0?[1-9]|1[0-2])
        \b",
    )
    .unwrap();

    for captures in regex.captures_iter(text) {
        let Some(found) = captures.get(0) else {
            continue;
        };
        let normalized = format!(
            "{}-{}/{}-{}",
            &captures["y1"],
            pad_month(&captures["m1"]),
            &captures["y2"],
            pad_month(&captures["m2"])
        );
        matches.push(date_range_match(found, normalized));
    }
}

fn extract_named_month_date_ranges(text: &str, matches: &mut Vec<RuleMatch>) {
    let regex = Regex::new(
        r"(?ix)
        \b
        (?P<m1>jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)
        \s+
        (?P<y1>19\d{2}|20\d{2})
        \s*(?:-|–|—|to)\s*
        (?P<m2>jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)
        \s+
        (?P<y2>19\d{2}|20\d{2})
        \b",
    )
    .unwrap();

    for captures in regex.captures_iter(text) {
        let Some(found) = captures.get(0) else {
            continue;
        };
        let Some(month_1) = month_number(&captures["m1"]) else {
            continue;
        };
        let Some(month_2) = month_number(&captures["m2"]) else {
            continue;
        };
        let normalized = format!(
            "{}-{}/{}-{}",
            &captures["y1"], month_1, &captures["y2"], month_2
        );
        matches.push(date_range_match(found, normalized));
    }
}

fn derive_years_experience(text: &str, matches: &mut Vec<RuleMatch>) {
    let date_ranges = matches
        .iter()
        .filter(|field| field.field_type == FieldType::DateRange)
        .filter_map(|field| {
            let normalized = field.normalized_value.as_deref()?;
            let months = months_in_normalized_range(normalized)?;
            Some((field.span_start, field.span_end, months))
        })
        .collect::<Vec<_>>();

    if date_ranges.is_empty() {
        return;
    }

    let total_months = date_ranges
        .iter()
        .map(|(_, _, months)| *months)
        .sum::<i32>();
    if total_months < 1 {
        return;
    }

    let span_start = date_ranges
        .iter()
        .map(|(span_start, _, _)| *span_start)
        .min()
        .unwrap();
    let span_end = date_ranges
        .iter()
        .map(|(_, span_end, _)| *span_end)
        .max()
        .unwrap();

    matches.push(RuleMatch {
        field_type: FieldType::YearsExperience,
        raw_value: text[span_start..span_end].to_string(),
        normalized_value: Some(format!("{:.1}", total_months as f32 / 12.0)),
        span_start,
        span_end,
        confidence: 0.82,
    });
}

fn months_in_normalized_range(normalized: &str) -> Option<i32> {
    let (start, end) = normalized.split_once('/')?;
    let (start_year, start_month) = parse_year_month(start)?;
    let (end_year, end_month) = parse_year_month(end)?;
    let months = (end_year - start_year) * 12 + (end_month - start_month);
    (months >= 0).then_some(months.max(1))
}

fn parse_year_month(value: &str) -> Option<(i32, i32)> {
    let (year, month) = value.split_once('-')?;
    Some((year.parse().ok()?, month.parse().ok()?))
}

fn extract_schools(text: &str, matches: &mut Vec<RuleMatch>) {
    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.len() > 120 || !looks_like_school(trimmed) {
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let span_start = line_start + leading;
        let span_end = span_start + trimmed.len();
        matches.push(RuleMatch {
            field_type: FieldType::School,
            raw_value: trimmed.to_string(),
            normalized_value: Some(trimmed.to_lowercase()),
            span_start,
            span_end,
            confidence: 0.84,
        });
    }
}

fn looks_like_school(line: &str) -> bool {
    let lower = line.to_lowercase();
    ["university", "college", "institute", "大学", "学院"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn extract_degrees(text: &str, matches: &mut Vec<RuleMatch>) {
    for (normalized, confidence, pattern) in [
        (
            "doctor",
            0.96,
            r"(?i)\b(?:ph\.?d\.?|doctor(?:ate)?(?:\s+of\s+[A-Za-z ]+)?)\b|博士",
        ),
        (
            "master",
            0.95,
            r"(?i)\b(?:master(?:'s)?(?:\s+of\s+[A-Za-z ]+)?|m\.?s\.?|m\.?a\.?|mba)\b|硕士|研究生",
        ),
        (
            "bachelor",
            0.95,
            r"(?i)\b(?:bachelor(?:'s)?(?:\s+of\s+[A-Za-z ]+)?|b\.?s\.?|b\.?a\.?|beng)\b|本科|学士",
        ),
        (
            "associate",
            0.9,
            r"(?i)\bassociate(?:\s+degree)?\b|大专|专科",
        ),
        ("high_school", 0.9, r"(?i)\bhigh\s+school\b|高中"),
    ] {
        let regex = Regex::new(pattern).unwrap();
        for found in regex.find_iter(text) {
            matches.push(RuleMatch {
                field_type: FieldType::Degree,
                raw_value: found.as_str().to_string(),
                normalized_value: Some(normalized.to_string()),
                span_start: found.start(),
                span_end: found.end(),
                confidence,
            });
        }
    }
}

fn extract_skills(text: &str, matches: &mut Vec<RuleMatch>) {
    let mut skill_context_lines = 0_usize;
    let mut seen = BTreeSet::new();
    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            skill_context_lines = skill_context_lines.saturating_sub(1);
            continue;
        }

        if is_skill_section_header(trimmed) {
            skill_context_lines = 8;
            continue;
        }

        if skill_context_lines > 0 && looks_like_section_header(trimmed) {
            skill_context_lines = 0;
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed_span_start = line_start + leading;
        let Some(segment) = skill_segment(trimmed, trimmed_span_start, skill_context_lines > 0)
        else {
            skill_context_lines = skill_context_lines.saturating_sub(1);
            continue;
        };

        push_skill_alias_matches(segment.text, segment.span_start, matches, &mut seen);
        skill_context_lines = skill_context_lines.saturating_sub(1);
    }
}

struct SkillSegment<'a> {
    text: &'a str,
    span_start: usize,
}

fn skill_segment<'a>(
    trimmed_line: &'a str,
    trimmed_span_start: usize,
    in_skill_context: bool,
) -> Option<SkillSegment<'a>> {
    if trimmed_line.len() > 180 {
        return None;
    }

    if let Some((label, value, delimiter_len)) = split_labeled_skill_line(trimmed_line) {
        let value_leading = value.len() - value.trim_start().len();
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        return Some(SkillSegment {
            text: value,
            span_start: trimmed_span_start + label.len() + delimiter_len + value_leading,
        });
    }

    if in_skill_context || looks_like_skill_line(trimmed_line) {
        return Some(SkillSegment {
            text: trimmed_line,
            span_start: trimmed_span_start,
        });
    }

    None
}

fn split_labeled_skill_line(line: &str) -> Option<(&str, &str, usize)> {
    let delimiter_start = line.find([':', '：'])?;
    let delimiter_len = line[delimiter_start..].chars().next()?.len_utf8();
    let label = &line[..delimiter_start];
    let value = &line[delimiter_start + delimiter_len..];
    is_skill_section_header(label.trim()).then_some((label, value, delimiter_len))
}

fn push_skill_alias_matches(
    text: &str,
    span_start: usize,
    matches: &mut Vec<RuleMatch>,
    seen: &mut BTreeSet<String>,
) {
    let mut claimed_spans = Vec::<(usize, usize)>::new();
    for (canonical, pattern) in skill_alias_patterns() {
        let regex = Regex::new(pattern).unwrap();
        for found in regex.find_iter(text) {
            let span = (found.start(), found.end());
            if claimed_spans
                .iter()
                .any(|claimed| ranges_overlap(*claimed, span))
            {
                continue;
            }
            if !seen.insert(canonical.to_string()) {
                continue;
            }
            claimed_spans.push(span);
            matches.push(RuleMatch {
                field_type: FieldType::Skill,
                raw_value: found.as_str().to_string(),
                normalized_value: Some(canonical.to_string()),
                span_start: span_start + found.start(),
                span_end: span_start + found.end(),
                confidence: 0.91,
            });
        }
    }
}

fn skill_alias_patterns() -> [(&'static str, &'static str); 17] {
    [
        ("Spring Cloud", r"(?i)\bspring\s+cloud\b"),
        ("JavaScript", r"(?i)\b(?:java\s*script|javascript|js)\b"),
        ("TypeScript", r"(?i)\b(?:type\s*script|typescript|ts)\b"),
        (
            "PostgreSQL",
            r"(?i)\b(?:postgre\s*sql|postgresql|postgres)\b",
        ),
        ("SQLite", r"(?i)\bsqlite\b"),
        ("Tantivy", r"(?i)\btantivy\b"),
        ("MySQL", r"(?i)\bmysql\b"),
        ("Kubernetes", r"(?i)\b(?:kubernetes|k8s)\b"),
        ("Docker", r"(?i)\bdocker\b"),
        ("Python", r"(?i)\bpython\b"),
        ("Rust", r"(?i)\brust\b"),
        ("Java", r"(?i)\bjava\b"),
        ("Go", r"(?i)\b(?:go|golang)\b"),
        ("Redis", r"(?i)\bredis\b"),
        ("React", r"(?i)\breact(?:\.js)?\b"),
        ("Node.js", r"(?i)\bnode(?:\.js|js)?\b"),
        ("SQL", r"(?i)\bsql\b"),
    ]
}

fn extract_companies(text: &str, matches: &mut Vec<RuleMatch>) {
    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.len() > 100 || !looks_like_company(trimmed) {
            continue;
        }

        let Some(normalized) = normalize_company(trimmed) else {
            continue;
        };
        let leading = line.len() - line.trim_start().len();
        let span_start = line_start + leading;
        let span_end = span_start + trimmed.len();
        matches.push(RuleMatch {
            field_type: FieldType::Company,
            raw_value: trimmed.to_string(),
            normalized_value: Some(normalized),
            span_start,
            span_end,
            confidence: 0.78,
        });
    }
}

fn looks_like_company(line: &str) -> bool {
    let lower = line.to_lowercase();
    [
        " inc.",
        " inc",
        " llc",
        " ltd",
        " corp",
        " corporation",
        " company",
        " technologies",
        " labs",
        " group",
        " bank",
        "有限公司",
        "公司",
        "集团",
    ]
    .iter()
    .any(|needle| lower.ends_with(needle) || lower.contains(needle))
}

fn normalize_company(value: &str) -> Option<String> {
    let mut normalized = value
        .trim()
        .trim_end_matches('.')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    for suffix in [
        " incorporated",
        " corporation",
        " technologies",
        " company",
        " group",
        " labs",
        " inc",
        " llc",
        " ltd",
        " corp",
        " bank",
        " 有限公司",
        " 公司",
        " 集团",
    ] {
        if normalized.ends_with(suffix) {
            normalized.truncate(normalized.len() - suffix.len());
            normalized = normalized.trim().to_string();
            break;
        }
    }

    (!normalized.is_empty()).then_some(normalized)
}

fn extract_titles(text: &str, matches: &mut Vec<RuleMatch>) {
    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.len() > 100 {
            continue;
        }

        let Some((normalized, confidence)) = normalize_title(trimmed) else {
            continue;
        };
        let leading = line.len() - line.trim_start().len();
        let span_start = line_start + leading;
        let span_end = span_start + trimmed.len();
        matches.push(RuleMatch {
            field_type: FieldType::Title,
            raw_value: trimmed.to_string(),
            normalized_value: Some(normalized.to_string()),
            span_start,
            span_end,
            confidence,
        });
    }
}

fn normalize_title(value: &str) -> Option<(&'static str, f32)> {
    let lower = value.to_lowercase();
    if lower.contains("backend engineer")
        || lower.contains("java engineer")
        || lower.contains("后端")
    {
        return Some(("backend_engineer", 0.84));
    }
    if lower.contains("software engineer") || lower.contains("software developer") {
        return Some(("software_engineer", 0.82));
    }
    if lower.contains("data engineer") || lower.contains("数据工程") {
        return Some(("data_engineer", 0.82));
    }
    if lower.contains("product manager") || lower.contains("产品经理") {
        return Some(("product_manager", 0.8));
    }

    None
}

fn extract_certificates(text: &str, matches: &mut Vec<RuleMatch>) {
    let mut certificate_context_lines = 0_usize;
    let mut seen = BTreeSet::new();

    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            certificate_context_lines = certificate_context_lines.saturating_sub(1);
            continue;
        }

        if is_certificate_section_header(trimmed) {
            certificate_context_lines = 6;
            continue;
        }

        if certificate_context_lines > 0 && looks_like_section_header(trimmed) {
            certificate_context_lines = 0;
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed_span_start = line_start + leading;
        let Some(segment) =
            certificate_segment(trimmed, trimmed_span_start, certificate_context_lines > 0)
        else {
            certificate_context_lines = certificate_context_lines.saturating_sub(1);
            continue;
        };

        let pushed_alias =
            push_certificate_alias_matches(segment.text, segment.span_start, matches, &mut seen);
        if !pushed_alias && segment.allow_line_fallback {
            let normalized = normalize_certificate_line(segment.text);
            if seen.insert(normalized.clone()) {
                matches.push(RuleMatch {
                    field_type: FieldType::Certificate,
                    raw_value: segment.text.to_string(),
                    normalized_value: Some(normalized),
                    span_start: segment.span_start,
                    span_end: segment.span_start + segment.text.len(),
                    confidence: 0.86,
                });
            }
        }

        certificate_context_lines = certificate_context_lines.saturating_sub(1);
    }
}

struct CertificateSegment<'a> {
    text: &'a str,
    span_start: usize,
    allow_line_fallback: bool,
}

fn certificate_segment<'a>(
    trimmed_line: &'a str,
    trimmed_span_start: usize,
    in_certificate_context: bool,
) -> Option<CertificateSegment<'a>> {
    if trimmed_line.len() > 140 {
        return None;
    }

    if let Some((label, value, delimiter_len)) = split_labeled_certificate_line(trimmed_line) {
        let value_leading = value.len() - value.trim_start().len();
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        let value_start = trimmed_span_start + label.len() + delimiter_len + value_leading;
        return Some(CertificateSegment {
            text: value,
            span_start: value_start,
            allow_line_fallback: true,
        });
    }

    if in_certificate_context {
        return Some(CertificateSegment {
            text: trimmed_line,
            span_start: trimmed_span_start,
            allow_line_fallback: false,
        });
    }

    if looks_like_certificate(trimmed_line) {
        return Some(CertificateSegment {
            text: trimmed_line,
            span_start: trimmed_span_start,
            allow_line_fallback: true,
        });
    }

    None
}

fn split_labeled_certificate_line(line: &str) -> Option<(&str, &str, usize)> {
    let delimiter_start = line.find([':', '：'])?;
    let delimiter_len = line[delimiter_start..].chars().next()?.len_utf8();
    let label = &line[..delimiter_start];
    let value = &line[delimiter_start + delimiter_len..];
    is_certificate_section_header(label.trim()).then_some((label, value, delimiter_len))
}

fn push_certificate_alias_matches(
    text: &str,
    span_start: usize,
    matches: &mut Vec<RuleMatch>,
    seen: &mut BTreeSet<String>,
) -> bool {
    let mut pushed = false;
    let mut claimed_spans = Vec::<(usize, usize)>::new();
    for (normalized, confidence, pattern) in certificate_alias_patterns() {
        let regex = Regex::new(pattern).unwrap();
        for found in regex.find_iter(text) {
            let span = (found.start(), found.end());
            if claimed_spans
                .iter()
                .any(|claimed| ranges_overlap(*claimed, span))
            {
                continue;
            }
            if !seen.insert(normalized.to_string()) {
                continue;
            }
            claimed_spans.push(span);
            matches.push(RuleMatch {
                field_type: FieldType::Certificate,
                raw_value: found.as_str().to_string(),
                normalized_value: Some(normalized.to_string()),
                span_start: span_start + found.start(),
                span_end: span_start + found.end(),
                confidence,
            });
            pushed = true;
        }
    }
    pushed
}

fn ranges_overlap(left: (usize, usize), right: (usize, usize)) -> bool {
    left.0 < right.1 && right.0 < left.1
}

fn certificate_alias_patterns() -> [(&'static str, f32, &'static str); 10] {
    [
        (
            "aws_solutions_architect",
            0.9,
            r"(?i)\baws\s+(?:certified\s+)?solutions?\s+architect(?:\s+associate|\s+professional)?\b|\bsaa-c0[23]\b",
        ),
        (
            "aws_developer",
            0.9,
            r"(?i)\baws\s+(?:certified\s+)?developer(?:\s+associate)?\b|\bdva-c0[12]\b",
        ),
        (
            "azure_administrator",
            0.88,
            r"(?i)\bazure\s+administrator\b|\baz-104\b",
        ),
        (
            "cka",
            0.9,
            r"(?i)\b(?:cka|certified\s+kubernetes\s+administrator)\b",
        ),
        (
            "ckad",
            0.9,
            r"(?i)\b(?:ckad|certified\s+kubernetes\s+application\s+developer)\b",
        ),
        ("cissp", 0.9, r"(?i)\bcissp\b"),
        ("pmp", 0.9, r"(?i)\bpmp\b"),
        ("cfa_level_1", 0.88, r"(?i)\bcfa\s+level\s+(?:i|1)\b"),
        ("cfa", 0.86, r"(?i)\bcfa\b"),
        ("cpa", 0.86, r"(?i)\bcpa\b|注册会计师"),
    ]
}

fn normalize_certificate_line(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn is_certificate_section_header(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    matches!(
        lower.as_str(),
        "certificate"
            | "certificates"
            | "certification"
            | "certifications"
            | "证书"
            | "认证"
            | "资格"
    )
}

fn looks_like_certificate(line: &str) -> bool {
    let lower = line.to_lowercase();
    if is_certificate_section_header(line) {
        return false;
    }

    lower.contains("certified")
        || lower.contains("certificate")
        || lower.contains("certification")
        || lower.contains("证书")
        || lower.contains("认证")
}

fn looks_like_skill_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    is_skill_section_header(line)
        || lower.contains("skill")
        || lower.contains("technical stack")
        || lower.contains("tech stack")
        || lower.contains("技术栈")
        || lower.contains("技能")
}

fn is_skill_section_header(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    matches!(
        lower.as_str(),
        "skill"
            | "skills"
            | "technical skill"
            | "technical skills"
            | "technical stack"
            | "tech stack"
            | "技能"
            | "专业技能"
            | "技术栈"
    )
}

fn indexed_lines(text: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut cursor = 0_usize;

    for line in text.split_inclusive('\n') {
        let raw_end = cursor + line.len();
        let line_end = raw_end - usize::from(line.ends_with('\n'));
        lines.push((cursor, &text[cursor..line_end]));
        cursor = raw_end;
    }

    if cursor < text.len() || text.is_empty() {
        lines.push((cursor, &text[cursor..]));
    }

    lines
}

fn date_range_match(found: regex::Match<'_>, normalized: String) -> RuleMatch {
    RuleMatch {
        field_type: FieldType::DateRange,
        raw_value: found.as_str().to_string(),
        normalized_value: Some(normalized),
        span_start: found.start(),
        span_end: found.end(),
        confidence: 0.92,
    }
}

fn pad_month(month: &str) -> String {
    if month.len() == 1 {
        format!("0{month}")
    } else {
        month.to_string()
    }
}

fn month_number(month: &str) -> Option<&'static str> {
    let lower = month.to_ascii_lowercase();
    match lower.as_str() {
        "jan" | "january" => Some("01"),
        "feb" | "february" => Some("02"),
        "mar" | "march" => Some("03"),
        "apr" | "april" => Some("04"),
        "may" => Some("05"),
        "jun" | "june" => Some("06"),
        "jul" | "july" => Some("07"),
        "aug" | "august" => Some("08"),
        "sep" | "sept" | "september" => Some("09"),
        "oct" | "october" => Some("10"),
        "nov" | "november" => Some("11"),
        "dec" | "december" => Some("12"),
        _ => None,
    }
}
