pub fn crate_name() -> &'static str {
    "extractor-rules"
}

use std::collections::BTreeSet;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldType {
    Name,
    Email,
    Phone,
    DateRange,
    School,
    SchoolTier,
    Degree,
    Major,
    Company,
    Title,
    Location,
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
    extract_numeric_present_date_ranges(text, &mut matches);
    extract_chinese_year_month_date_ranges(text, &mut matches);
    extract_chinese_present_date_ranges(text, &mut matches);
    extract_named_month_date_ranges(text, &mut matches);
    extract_named_month_present_date_ranges(text, &mut matches);
    derive_years_experience(text, &mut matches);
    extract_schools(text, &mut matches);
    extract_school_tiers(text, &mut matches);
    extract_degrees(text, &mut matches);
    extract_majors(text, &mut matches);
    extract_companies(text, &mut matches);
    extract_titles(text, &mut matches);
    extract_locations(text, &mut matches);
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
    let mut claimed_spans = Vec::<(usize, usize)>::new();
    extract_chinese_mobile_phones(text, matches, &mut claimed_spans);
    extract_general_phones(text, matches, &mut claimed_spans);
}

fn extract_chinese_mobile_phones(
    text: &str,
    matches: &mut Vec<RuleMatch>,
    claimed_spans: &mut Vec<(usize, usize)>,
) {
    let regex = Regex::new(
        r"(?x)
        (?:^|[^\d])
        (?P<phone>(?:(?:\+|00)?86[\s.-]*)?(?P<n>1[3-9]\d[\s.-]*\d{4}[\s.-]*\d{4}))
        (?:$|[^\d])
        ",
    )
    .unwrap();

    for captures in regex.captures_iter(text) {
        let Some(found) = captures.name("phone") else {
            continue;
        };
        let span = (found.start(), found.end());
        if claimed_spans
            .iter()
            .any(|claimed| ranges_overlap(*claimed, span))
        {
            continue;
        }
        let digits = captures["n"]
            .chars()
            .filter(|character| character.is_ascii_digit())
            .collect::<String>();
        if digits.len() != 11 {
            continue;
        }
        claimed_spans.push(span);
        matches.push(RuleMatch {
            field_type: FieldType::Phone,
            raw_value: found.as_str().to_string(),
            normalized_value: Some(format!("+86{digits}")),
            span_start: found.start(),
            span_end: found.end(),
            confidence: 0.98,
        });
    }
}

fn extract_general_phones(
    text: &str,
    matches: &mut Vec<RuleMatch>,
    claimed_spans: &mut Vec<(usize, usize)>,
) {
    let regex =
        Regex::new(r"(?x)(?:\+\d{1,3}[\s.-]*)?(?:\(\d{3}\)|\d{3,4})[\s.-]+\d{3,4}[\s.-]+\d{4}")
            .unwrap();

    for found in regex.find_iter(text) {
        let span = (found.start(), found.end());
        if claimed_spans
            .iter()
            .any(|claimed| ranges_overlap(*claimed, span))
        {
            continue;
        }
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
            claimed_spans.push(span);
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

fn extract_numeric_present_date_ranges(text: &str, matches: &mut Vec<RuleMatch>) {
    let regex = Regex::new(
        r"(?ix)
        \b
        (?P<y1>19\d{2}|20\d{2})[./-](?P<m1>0?[1-9]|1[0-2])
        \s*(?:-|–|—|至|到|to)\s*
        (?:(?:present|current|now|ongoing)\b|至今|现在|目前|当前)",
    )
    .unwrap();

    for captures in regex.captures_iter(text) {
        let Some(found) = captures.get(0) else {
            continue;
        };
        let normalized = format!("{}-{}/PRESENT", &captures["y1"], pad_month(&captures["m1"]));
        matches.push(date_range_match(found, normalized));
    }
}

fn extract_chinese_year_month_date_ranges(text: &str, matches: &mut Vec<RuleMatch>) {
    let regex = Regex::new(
        r"(?x)
        (?P<y1>19\d{2}|20\d{2})\s*年\s*(?P<m1>0?[1-9]|1[0-2])\s*月?
        \s*(?:-|–|—|至|到)\s*
        (?P<y2>19\d{2}|20\d{2})\s*年\s*(?P<m2>0?[1-9]|1[0-2])\s*月?
        ",
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

fn extract_chinese_present_date_ranges(text: &str, matches: &mut Vec<RuleMatch>) {
    let regex = Regex::new(
        r"(?ix)
        (?P<y1>19\d{2}|20\d{2})\s*年\s*(?P<m1>0?[1-9]|1[0-2])\s*月?
        \s*(?:-|–|—|至|到)?\s*
        (?:至今|今|现在|目前|当前|(?:present|current|now|ongoing)\b)
        ",
    )
    .unwrap();

    for captures in regex.captures_iter(text) {
        let Some(found) = captures.get(0) else {
            continue;
        };
        let normalized = format!("{}-{}/PRESENT", &captures["y1"], pad_month(&captures["m1"]));
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

fn extract_named_month_present_date_ranges(text: &str, matches: &mut Vec<RuleMatch>) {
    let regex = Regex::new(
        r"(?ix)
        \b
        (?P<m1>jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)
        \s+
        (?P<y1>19\d{2}|20\d{2})
        \s*(?:-|–|—|to)\s*
        (?:present|current|now|ongoing)
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
        let normalized = format!("{}-{}/PRESENT", &captures["y1"], month_1);
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
    let (end_year, end_month) = if end == "PRESENT" {
        current_year_month()?
    } else {
        parse_year_month(end)?
    };
    let months = (end_year - start_year) * 12 + (end_month - start_month);
    (months >= 0).then_some(months.max(1))
}

fn parse_year_month(value: &str) -> Option<(i32, i32)> {
    let (year, month) = value.split_once('-')?;
    Some((year.parse().ok()?, month.parse().ok()?))
}

fn current_year_month() -> Option<(i32, i32)> {
    let days_since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() / 86_400;
    let (year, month, _) = civil_from_days(days_since_epoch as i64);
    Some((year, month))
}

// Howard Hinnant's civil date conversion keeps this crate dependency-free.
fn civil_from_days(days_since_epoch: i64) -> (i32, i32, i32) {
    let days = days_since_epoch + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_piece = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_piece + 2) / 5 + 1;
    let month = month_piece + if month_piece < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year as i32, month as i32, day as i32)
}

fn extract_schools(text: &str, matches: &mut Vec<RuleMatch>) {
    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.len() > 120 {
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed_span_start = line_start + leading;
        let Some(segment) = school_segment(trimmed, trimmed_span_start) else {
            continue;
        };
        matches.push(RuleMatch {
            field_type: FieldType::School,
            raw_value: segment.text.to_string(),
            normalized_value: Some(normalize_school(segment.text)),
            span_start: segment.span_start,
            span_end: segment.span_start + segment.text.len(),
            confidence: 0.84,
        });
    }
}

fn school_segment<'a>(
    trimmed_line: &'a str,
    trimmed_span_start: usize,
) -> Option<LabeledSegment<'a>> {
    if let Some((label, value, delimiter_len)) =
        split_labeled_value_line(trimmed_line, is_school_label)
    {
        let value_leading = value.len() - value.trim_start().len();
        let value = value.trim();
        if value.is_empty() || !looks_like_school(value) {
            return None;
        }
        return Some(LabeledSegment {
            text: value,
            span_start: trimmed_span_start + label.len() + delimiter_len + value_leading,
        });
    }

    looks_like_school(trimmed_line).then_some(LabeledSegment {
        text: trimmed_line,
        span_start: trimmed_span_start,
    })
}

fn is_school_label(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "school"
            | "university"
            | "college"
            | "institution"
            | "institute"
            | "学校"
            | "院校"
            | "毕业院校"
            | "大学"
    )
}

fn looks_like_school(line: &str) -> bool {
    let lower = line.to_lowercase();
    ["university", "college", "institute", "大学", "学院"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn normalize_school(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn extract_school_tiers(text: &str, matches: &mut Vec<RuleMatch>) {
    let mut education_context_lines = 0_usize;
    let mut seen = BTreeSet::new();

    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            education_context_lines = education_context_lines.saturating_sub(1);
            continue;
        }

        if is_education_section_header(trimmed) {
            education_context_lines = 8;
            continue;
        }

        if education_context_lines > 0 && looks_like_section_header(trimmed) {
            education_context_lines = 0;
            continue;
        }

        if trimmed.len() > 160 {
            education_context_lines = education_context_lines.saturating_sub(1);
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed_span_start = line_start + leading;
        if let Some(segment) =
            school_tier_segment(trimmed, trimmed_span_start, education_context_lines > 0)
        {
            push_school_tier_matches(segment, matches, &mut seen);
        }

        education_context_lines = education_context_lines.saturating_sub(1);
    }
}

fn school_tier_segment<'a>(
    trimmed_line: &'a str,
    trimmed_span_start: usize,
    in_education_context: bool,
) -> Option<LabeledSegment<'a>> {
    if let Some((label, value, delimiter_len)) =
        split_labeled_value_line(trimmed_line, is_school_tier_label)
    {
        let value_leading = value.len() - value.trim_start().len();
        let value = value.trim();
        return (!value.is_empty()).then_some(LabeledSegment {
            text: value,
            span_start: trimmed_span_start + label.len() + delimiter_len + value_leading,
        });
    }

    if let Some(segment) = school_segment(trimmed_line, trimmed_span_start) {
        return Some(segment);
    }

    in_education_context.then_some(LabeledSegment {
        text: trimmed_line,
        span_start: trimmed_span_start,
    })
}

fn is_school_tier_label(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "school tier"
            | "school_tier"
            | "university tier"
            | "institution tier"
            | "tier"
            | "学校层次"
            | "院校层次"
            | "高校层次"
            | "学校类型"
            | "院校类型"
            | "高校类型"
    )
}

fn push_school_tier_matches(
    segment: LabeledSegment<'_>,
    matches: &mut Vec<RuleMatch>,
    seen: &mut BTreeSet<String>,
) {
    for (normalized, confidence, pattern) in school_tier_alias_patterns() {
        let regex = Regex::new(pattern).unwrap();
        for found in regex.find_iter(segment.text) {
            if school_tier_digit_has_adjacent_digit(segment.text, found.start(), found.end()) {
                continue;
            }
            if !seen.insert(normalized.to_string()) {
                continue;
            }
            matches.push(RuleMatch {
                field_type: FieldType::SchoolTier,
                raw_value: found.as_str().to_string(),
                normalized_value: Some(normalized.to_string()),
                span_start: segment.span_start + found.start(),
                span_end: segment.span_start + found.end(),
                confidence,
            });
        }
    }
}

fn school_tier_alias_patterns() -> [(&'static str, f32, &'static str); 5] {
    [
        ("985", 0.92, r"985"),
        ("211", 0.9, r"211"),
        (
            "double_first_class",
            0.9,
            r"(?i)double[-\s_]*first[-\s_]*class|双一流",
        ),
        (
            "overseas",
            0.86,
            r"(?i)overseas|foreign\s+university|international\s+university|海外高校|海外院校|海外学校|海外|国外",
        ),
        (
            "regular",
            0.82,
            r"(?i)regular\s+university|ordinary\s+university|普通高校|普通院校|普通本科|普通大学",
        ),
    ]
}

fn school_tier_digit_has_adjacent_digit(value: &str, start: usize, end: usize) -> bool {
    let previous_is_digit = start > 0
        && value
            .as_bytes()
            .get(start - 1)
            .is_some_and(u8::is_ascii_digit);
    previous_is_digit || value.as_bytes().get(end).is_some_and(u8::is_ascii_digit)
}

fn extract_degrees(text: &str, matches: &mut Vec<RuleMatch>) {
    let mut claimed_spans = Vec::<(usize, usize)>::new();
    let mut education_context_lines = 0_usize;

    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            education_context_lines = education_context_lines.saturating_sub(1);
            continue;
        }

        if is_education_section_header(trimmed) {
            education_context_lines = 8;
            continue;
        }

        if education_context_lines > 0 && looks_like_section_header(trimmed) {
            education_context_lines = 0;
            continue;
        }

        if trimmed.len() > 140 {
            education_context_lines = education_context_lines.saturating_sub(1);
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed_span_start = line_start + leading;
        if let Some(segment) = degree_segment(trimmed, trimmed_span_start) {
            push_first_degree_match(segment, matches, &mut claimed_spans);
            education_context_lines = education_context_lines.saturating_sub(1);
            continue;
        }

        if education_context_lines > 0 {
            push_first_degree_match(
                LabeledSegment {
                    text: trimmed,
                    span_start: trimmed_span_start,
                },
                matches,
                &mut claimed_spans,
            );
        }

        education_context_lines = education_context_lines.saturating_sub(1);
    }
}

fn push_first_degree_match(
    segment: LabeledSegment<'_>,
    matches: &mut Vec<RuleMatch>,
    claimed_spans: &mut Vec<(usize, usize)>,
) -> bool {
    let Some((normalized, confidence, relative_start, relative_end)) =
        first_degree_match(segment.text)
    else {
        return false;
    };
    let span = (
        segment.span_start + relative_start,
        segment.span_start + relative_end,
    );
    if claimed_spans
        .iter()
        .any(|claimed| ranges_overlap(*claimed, span))
    {
        return false;
    }
    claimed_spans.push(span);
    matches.push(RuleMatch {
        field_type: FieldType::Degree,
        raw_value: segment.text[relative_start..relative_end].to_string(),
        normalized_value: Some(normalized.to_string()),
        span_start: span.0,
        span_end: span.1,
        confidence,
    });
    true
}

fn degree_segment<'a>(
    trimmed_line: &'a str,
    trimmed_span_start: usize,
) -> Option<LabeledSegment<'a>> {
    let (label, value, delimiter_len) = split_labeled_value_line(trimmed_line, is_degree_label)?;
    let value_leading = value.len() - value.trim_start().len();
    let value = value.trim();
    (!value.is_empty()).then_some(LabeledSegment {
        text: value,
        span_start: trimmed_span_start + label.len() + delimiter_len + value_leading,
    })
}

fn is_degree_label(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "degree"
            | "education"
            | "education level"
            | "qualification"
            | "学历"
            | "学位"
            | "教育"
            | "最高学历"
    )
}

fn is_education_section_header(value: &str) -> bool {
    matches!(
        value.to_lowercase().as_str(),
        "education"
            | "education background"
            | "academic background"
            | "academic history"
            | "学历"
            | "学位"
            | "教育"
            | "教育经历"
            | "学习经历"
            | "教育背景"
    )
}

fn first_degree_match(value: &str) -> Option<(&'static str, f32, usize, usize)> {
    for (normalized, confidence, pattern) in degree_alias_patterns() {
        let regex = Regex::new(pattern).unwrap();
        if let Some(found) = regex.find(value) {
            return Some((normalized, confidence, found.start(), found.end()));
        }
    }

    None
}

fn degree_alias_patterns() -> [(&'static str, f32, &'static str); 5] {
    [
        (
            "doctor",
            0.96,
            r"(?i)\b(?:ph\.?\s*d\.?|phd|doctor(?:ate)?(?:\s+of\s+[A-Za-z ]+)?)\b|博士研究生|博士",
        ),
        (
            "master",
            0.95,
            r"(?i)\b(?:master(?:'s)?(?:\s+of\s+[A-Za-z ]+)?|m\.?\s*sc|m\.?\s*s\.?|m\.?\s*a\.?|mba)\b|硕士研究生|硕士|研究生",
        ),
        (
            "bachelor",
            0.95,
            r"(?i)\b(?:bachelor(?:'s)?(?:\s+of\s+[A-Za-z ]+)?|b\.?\s*sc|b\.?\s*s\.?|b\.?\s*a\.?|b\.?\s*eng|beng)\b|本科|学士",
        ),
        (
            "associate",
            0.9,
            r"(?i)\bassociate(?:\s+degree)?\b|大专|专科",
        ),
        ("high_school", 0.9, r"(?i)\bhigh\s+school\b|高中"),
    ]
}

fn extract_majors(text: &str, matches: &mut Vec<RuleMatch>) {
    let mut education_context_lines = 0_usize;
    let mut seen = BTreeSet::new();

    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            education_context_lines = education_context_lines.saturating_sub(1);
            continue;
        }

        if is_education_section_header(trimmed) {
            education_context_lines = 8;
            continue;
        }

        if education_context_lines > 0 && looks_like_section_header(trimmed) {
            education_context_lines = 0;
            continue;
        }

        if trimmed.len() > 140 {
            education_context_lines = education_context_lines.saturating_sub(1);
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed_span_start = line_start + leading;
        if let Some(segment) = major_segment(trimmed, trimmed_span_start) {
            push_first_major_match(segment, matches, &mut seen, true);
            education_context_lines = education_context_lines.saturating_sub(1);
            continue;
        }

        if education_context_lines > 0 {
            push_first_major_match(
                LabeledSegment {
                    text: trimmed,
                    span_start: trimmed_span_start,
                },
                matches,
                &mut seen,
                false,
            );
        }

        education_context_lines = education_context_lines.saturating_sub(1);
    }
}

fn major_segment<'a>(
    trimmed_line: &'a str,
    trimmed_span_start: usize,
) -> Option<LabeledSegment<'a>> {
    let (label, value, delimiter_len) = split_labeled_value_line(trimmed_line, is_major_label)?;
    let value_leading = value.len() - value.trim_start().len();
    let value = value.trim();
    (!value.is_empty()).then_some(LabeledSegment {
        text: value,
        span_start: trimmed_span_start + label.len() + delimiter_len + value_leading,
    })
}

fn is_major_label(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "major"
            | "field"
            | "field of study"
            | "study field"
            | "specialization"
            | "specialisation"
            | "concentration"
            | "专业"
            | "所学专业"
            | "主修"
            | "主修专业"
            | "专业方向"
            | "研究方向"
    )
}

fn push_first_major_match(
    segment: LabeledSegment<'_>,
    matches: &mut Vec<RuleMatch>,
    seen: &mut BTreeSet<String>,
    allow_freeform_labeled_value: bool,
) -> bool {
    let major_match = first_major_alias_match(segment.text).or_else(|| {
        allow_freeform_labeled_value.then(|| {
            let normalized = normalize_freeform_major(segment.text)?;
            Some((normalized, 0.86, 0, segment.text.len()))
        })?
    });
    let Some((normalized, confidence, relative_start, relative_end)) = major_match else {
        return false;
    };
    if !seen.insert(normalized.clone()) {
        return false;
    }

    matches.push(RuleMatch {
        field_type: FieldType::Major,
        raw_value: segment.text[relative_start..relative_end].to_string(),
        normalized_value: Some(normalized),
        span_start: segment.span_start + relative_start,
        span_end: segment.span_start + relative_end,
        confidence,
    });
    true
}

fn first_major_alias_match(value: &str) -> Option<(String, f32, usize, usize)> {
    for (normalized, confidence, pattern) in major_alias_patterns() {
        let regex = Regex::new(pattern).unwrap();
        if let Some(found) = regex.find(value) {
            return Some((
                normalized.to_string(),
                confidence,
                found.start(),
                found.end(),
            ));
        }
    }

    None
}

fn major_alias_patterns() -> [(&'static str, f32, &'static str); 20] {
    [
        (
            "computer_science",
            0.94,
            r"(?i)\bcomputer\s+science\b|计算机科学(?:与技术)?",
        ),
        (
            "software_engineering",
            0.94,
            r"(?i)\bsoftware\s+engineering\b|软件工程",
        ),
        ("data_science", 0.94, r"(?i)\bdata\s+science\b|数据科学"),
        (
            "information_systems",
            0.9,
            r"(?i)\binformation\s+systems?\b|信息系统",
        ),
        (
            "electronic_engineering",
            0.9,
            r"(?i)\belectronic(?:s)?\s+engineering\b|电子(?:信息)?工程",
        ),
        ("mathematics", 0.9, r"(?i)\bmathematics?\b|数学"),
        ("statistics", 0.9, r"(?i)\bstatistics?\b|统计学?"),
        ("finance", 0.88, r"(?i)\bfinance\b|金融学?"),
        ("economics", 0.88, r"(?i)\beconomics?\b|经济学?"),
        (
            "business_administration",
            0.88,
            r"(?i)\bbusiness\s+administration\b|工商管理",
        ),
        (
            "artificial_intelligence",
            0.92,
            r"(?i)\bartificial\s+intelligence\b|人工智能",
        ),
        (
            "computer_engineering",
            0.91,
            r"(?i)\bcomputer\s+engineering\b|计算机工程",
        ),
        (
            "cybersecurity",
            0.9,
            r"(?i)\bcyber\s*security\b|网络安全|信息安全",
        ),
        (
            "network_engineering",
            0.9,
            r"(?i)\bnetwork\s+engineering\b|网络工程",
        ),
        (
            "communication_engineering",
            0.9,
            r"(?i)\bcommunications?\s+engineering\b|通信工程",
        ),
        (
            "mechanical_engineering",
            0.9,
            r"(?i)\bmechanical\s+engineering\b|机械工程",
        ),
        ("automation", 0.88, r"(?i)\bautomation\b|自动化"),
        ("accounting", 0.88, r"(?i)\baccount(?:ing|ancy)\b|会计学?"),
        ("marketing", 0.88, r"(?i)\bmarketing\b|市场营销"),
        (
            "human_resources",
            0.88,
            r"(?i)\bhuman\s+resources?(?:\s+management)?\b|人力资源管理",
        ),
    ]
}

fn normalize_freeform_major(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_matches(|character: char| {
            matches!(character, ',' | ';' | '|' | '/' | '\\' | '，' | '；' | '、')
        })
        .trim();
    if value.is_empty()
        || value.len() > 80
        || value.contains('@')
        || looks_like_contact_line(value)
        || looks_like_section_header(value)
    {
        return None;
    }
    let normalized = value
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    (!normalized.is_empty()).then_some(normalized)
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

fn skill_alias_patterns() -> [(&'static str, &'static str); 26] {
    [
        ("Spark", r"(?i)\b(?:apache\s+)?spark\b"),
        ("Hadoop", r"(?i)\b(?:apache\s+)?hadoop\b"),
        ("Airflow", r"(?i)\b(?:apache\s+)?airflow\b"),
        ("TensorFlow", r"(?i)\btensor\s*flow\b"),
        ("PyTorch", r"(?i)\bpy\s*torch\b"),
        ("scikit-learn", r"(?i)\b(?:scikit[-\s]?learn|sklearn)\b"),
        ("Vue.js", r"(?i)\bvue(?:\.js)?\b"),
        ("Angular", r"(?i)\bangular\b"),
        ("GraphQL", r"(?i)\bgraph\s*ql\b"),
        ("React", r"(?i)\breact(?:\.js)?\b"),
        ("Node.js", r"(?i)\bnode(?:\.js|js)?\b"),
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
        ("SQL", r"(?i)\bsql\b"),
    ]
}

fn extract_companies(text: &str, matches: &mut Vec<RuleMatch>) {
    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.len() > 120 {
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed_span_start = line_start + leading;
        let Some(segment) = company_segment(trimmed, trimmed_span_start) else {
            continue;
        };
        let Some(normalized) = normalize_company(segment.text) else {
            continue;
        };
        matches.push(RuleMatch {
            field_type: FieldType::Company,
            raw_value: segment.text.to_string(),
            normalized_value: Some(normalized),
            span_start: segment.span_start,
            span_end: segment.span_start + segment.text.len(),
            confidence: 0.78,
        });
    }
}

struct LabeledSegment<'a> {
    text: &'a str,
    span_start: usize,
}

fn company_segment<'a>(
    trimmed_line: &'a str,
    trimmed_span_start: usize,
) -> Option<LabeledSegment<'a>> {
    if let Some((label, value, delimiter_len)) =
        split_labeled_value_line(trimmed_line, is_company_label)
    {
        let value_leading = value.len() - value.trim_start().len();
        let value = value.trim();
        if value.is_empty() || !looks_like_company(value) {
            return None;
        }
        return Some(LabeledSegment {
            text: value,
            span_start: trimmed_span_start + label.len() + delimiter_len + value_leading,
        });
    }

    looks_like_company(trimmed_line).then_some(LabeledSegment {
        text: trimmed_line,
        span_start: trimmed_span_start,
    })
}

fn split_labeled_value_line(line: &str, is_label: fn(&str) -> bool) -> Option<(&str, &str, usize)> {
    let delimiter_start = line.find([':', '：'])?;
    let delimiter_len = line[delimiter_start..].chars().next()?.len_utf8();
    let label = &line[..delimiter_start];
    let value = &line[delimiter_start + delimiter_len..];
    is_label(label.trim()).then_some((label, value, delimiter_len))
}

fn is_company_label(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "company"
            | "employer"
            | "organization"
            | "organisation"
            | "workplace"
            | "公司"
            | "单位"
            | "雇主"
            | "组织"
    )
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
        "有限责任公司",
        "股份有限公司",
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
        " 有限责任公司",
        " 股份有限公司",
        " 有限公司",
        " 公司",
        " 集团",
        "有限责任公司",
        "股份有限公司",
        "有限公司",
        "公司",
        "集团",
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
        if trimmed.len() > 120 {
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed_span_start = line_start + leading;
        let Some(segment) = title_segment(trimmed, trimmed_span_start) else {
            continue;
        };
        let Some((normalized, confidence)) = normalize_title(segment.text) else {
            continue;
        };
        matches.push(RuleMatch {
            field_type: FieldType::Title,
            raw_value: segment.text.to_string(),
            normalized_value: Some(normalized.to_string()),
            span_start: segment.span_start,
            span_end: segment.span_start + segment.text.len(),
            confidence,
        });
    }
}

fn title_segment<'a>(
    trimmed_line: &'a str,
    trimmed_span_start: usize,
) -> Option<LabeledSegment<'a>> {
    if let Some((label, value, delimiter_len)) =
        split_labeled_value_line(trimmed_line, is_title_label)
    {
        let value_leading = value.len() - value.trim_start().len();
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        return Some(LabeledSegment {
            text: value,
            span_start: trimmed_span_start + label.len() + delimiter_len + value_leading,
        });
    }

    Some(LabeledSegment {
        text: trimmed_line,
        span_start: trimmed_span_start,
    })
}

fn is_title_label(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "title" | "role" | "position" | "job title" | "职位" | "岗位" | "职务" | "角色"
    )
}

fn normalize_title(value: &str) -> Option<(&'static str, f32)> {
    let lower = value.to_lowercase();
    if looks_like_certificate(value)
        || looks_like_certificate_alias(value)
        || looks_like_skill_line(value)
    {
        return None;
    }
    if lower.contains("platform engineer") || lower.contains("平台工程师") {
        return Some(("platform_engineer", 0.83));
    }
    if lower.contains("security engineer")
        || lower.contains("信息安全工程师")
        || lower.contains("安全工程师")
    {
        return Some(("security_engineer", 0.83));
    }
    if lower.contains("mobile engineer")
        || lower.contains("ios engineer")
        || lower.contains("android engineer")
        || lower.contains("移动端工程师")
        || lower.contains("移动开发")
    {
        return Some(("mobile_engineer", 0.82));
    }
    if lower.contains("business analyst") || lower.contains("业务分析师") {
        return Some(("business_analyst", 0.80));
    }
    if (lower.contains("frontend") || lower.contains("front-end") || lower.contains("前端"))
        && has_engineering_role_marker(&lower)
    {
        return Some(("frontend_engineer", 0.83));
    }
    if (lower.contains("fullstack")
        || lower.contains("full-stack")
        || lower.contains("full stack")
        || lower.contains("全栈"))
        && has_engineering_role_marker(&lower)
    {
        return Some(("fullstack_engineer", 0.83));
    }
    if lower.contains("machine learning engineer")
        || lower.contains("ml engineer")
        || lower.contains("机器学习工程师")
        || lower.contains("算法工程师")
    {
        return Some(("machine_learning_engineer", 0.83));
    }
    if lower.contains("data scientist") || lower.contains("数据科学家") {
        return Some(("data_scientist", 0.83));
    }
    if lower.contains("devops engineer")
        || lower.contains("site reliability engineer")
        || lower.contains("运维工程师")
    {
        return Some(("devops_engineer", 0.83));
    }
    if lower.contains("qa engineer")
        || lower.contains("quality assurance engineer")
        || lower.contains("test engineer")
        || lower.contains("测试工程师")
    {
        return Some(("qa_engineer", 0.82));
    }
    if lower.contains("engineering manager")
        || lower.contains("研发经理")
        || lower.contains("技术经理")
        || lower.contains("工程经理")
    {
        return Some(("engineering_manager", 0.82));
    }
    if lower.contains("solutions architect")
        || lower.contains("solution architect")
        || lower.contains("架构师")
    {
        return Some(("solutions_architect", 0.82));
    }
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

fn has_engineering_role_marker(lower: &str) -> bool {
    lower.contains("engineer")
        || lower.contains("developer")
        || lower.contains("工程师")
        || lower.contains("开发")
}

fn extract_locations(text: &str, matches: &mut Vec<RuleMatch>) {
    let mut seen = BTreeSet::new();
    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.len() > 120 {
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let trimmed_span_start = line_start + leading;
        let Some(segment) = location_segment(trimmed, trimmed_span_start) else {
            continue;
        };
        let Some(normalized) = normalize_location(segment.text) else {
            continue;
        };
        if !seen.insert(normalized.clone()) {
            continue;
        }
        matches.push(RuleMatch {
            field_type: FieldType::Location,
            raw_value: segment.text.to_string(),
            normalized_value: Some(normalized),
            span_start: segment.span_start,
            span_end: segment.span_start + segment.text.len(),
            confidence: 0.86,
        });
    }
}

fn location_segment<'a>(
    trimmed_line: &'a str,
    trimmed_span_start: usize,
) -> Option<LabeledSegment<'a>> {
    let (label, value, delimiter_len) =
        split_labeled_value_line(trimmed_line, is_location_or_address_label)?;
    let value_leading = value.len() - value.trim_start().len();
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let value_span_start = trimmed_span_start + label.len() + delimiter_len + value_leading;
    if is_address_location_label(label.trim()) {
        return location_segment_from_address_value(value, value_span_start);
    }
    Some(LabeledSegment {
        text: value,
        span_start: value_span_start,
    })
}

fn is_location_or_address_label(label: &str) -> bool {
    is_location_label(label) || is_address_location_label(label)
}

fn is_location_label(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "location"
            | "current location"
            | "base"
            | "city"
            | "base city"
            | "preferred city"
            | "work location"
            | "所在地"
            | "地点"
            | "城市"
            | "现居地"
            | "居住地"
            | "工作地点"
            | "期望城市"
    )
}

fn is_address_location_label(label: &str) -> bool {
    matches!(
        label.to_lowercase().as_str(),
        "address"
            | "current address"
            | "mailing address"
            | "residential address"
            | "home address"
            | "地址"
            | "现居住地址"
            | "居住地址"
            | "通讯地址"
    )
}

fn location_segment_from_address_value<'a>(
    value: &'a str,
    value_span_start: usize,
) -> Option<LabeledSegment<'a>> {
    let mut component_start = 0;
    for (index, character) in value.char_indices() {
        if matches!(character, ',' | '，' | ';' | '；' | '|' | '/' | '\\' | '、') {
            if let Some(segment) =
                address_component_location_segment(value, value_span_start, component_start, index)
            {
                return Some(segment);
            }
            component_start = index + character.len_utf8();
        }
    }

    if let Some(segment) =
        address_component_location_segment(value, value_span_start, component_start, value.len())
    {
        return Some(segment);
    }

    location_alias_substring(value, value_span_start)
}

fn address_component_location_segment<'a>(
    value: &'a str,
    value_span_start: usize,
    component_start: usize,
    component_end: usize,
) -> Option<LabeledSegment<'a>> {
    let component = &value[component_start..component_end];
    let leading = component.len() - component.trim_start().len();
    let text = component.trim();
    if text.is_empty() || normalize_location(text).is_none() {
        return None;
    }
    Some(LabeledSegment {
        text,
        span_start: value_span_start + component_start + leading,
    })
}

fn location_alias_substring<'a>(
    value: &'a str,
    value_span_start: usize,
) -> Option<LabeledSegment<'a>> {
    let lower_value = value.to_lowercase();
    for &alias in address_location_alias_substrings() {
        let needle = alias.to_lowercase();
        let Some(offset) = lower_value.find(&needle) else {
            continue;
        };
        return Some(LabeledSegment {
            text: &value[offset..offset + alias.len()],
            span_start: value_span_start + offset,
        });
    }
    None
}

fn address_location_alias_substrings() -> &'static [&'static str] {
    &[
        "San Francisco",
        "New York City",
        "New York",
        "Hong Kong",
        "Los Angeles",
        "San Jose",
        "Singapore",
        "Chongqing",
        "Shanghai",
        "Hangzhou",
        "Shenzhen",
        "Beijing",
        "Guangzhou",
        "Suzhou",
        "Nanjing",
        "Chengdu",
        "Wuhan",
        "Tianjin",
        "Changsha",
        "Qingdao",
        "Toronto",
        "Vancouver",
        "Seattle",
        "Austin",
        "Boston",
        "北京市",
        "北京",
        "上海市",
        "上海",
        "深圳市",
        "深圳",
        "广州市",
        "广州",
        "杭州市",
        "杭州",
        "南京市",
        "南京",
        "苏州市",
        "苏州",
        "成都市",
        "成都",
        "武汉市",
        "武汉",
        "香港",
        "重庆市",
        "重庆",
        "天津市",
        "天津",
        "长沙市",
        "长沙",
        "青岛市",
        "青岛",
        "合肥市",
        "合肥",
        "西安市",
        "西安",
    ]
}

fn normalize_location(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_matches(|character: char| {
            matches!(character, ',' | ';' | '|' | '/' | '\\' | '，' | '；' | '、')
        })
        .trim();
    if value.is_empty() || value.len() > 80 || value.contains('@') {
        return None;
    }

    let primary = value
        .split([',', '，', ';', '；', '|', '/', '\\', '、'])
        .next()
        .unwrap_or_default()
        .trim();
    if primary.is_empty() || looks_like_section_header(primary) || looks_like_company(primary) {
        return None;
    }

    let normalized = primary
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if let Some(canonical) = canonical_location_alias(&normalized) {
        return Some(canonical.to_string());
    }

    let looks_like_plain_place = normalized.chars().all(|character| {
        character.is_ascii_alphabetic()
            || character.is_whitespace()
            || character == '-'
            || ('\u{4e00}'..='\u{9fff}').contains(&character)
    });
    (looks_like_plain_place && normalized.len() <= 48).then_some(normalized)
}

fn canonical_location_alias(value: &str) -> Option<&'static str> {
    let compact = value.replace([' ', '-', '_'], "");
    match compact.as_str() {
        "shanghai" | "上海" | "上海市" => Some("shanghai"),
        "hangzhou" | "杭州" | "杭州市" => Some("hangzhou"),
        "shenzhen" | "深圳" | "深圳市" => Some("shenzhen"),
        "beijing" | "北京" | "北京市" => Some("beijing"),
        "guangzhou" | "广州" | "广州市" => Some("guangzhou"),
        "suzhou" | "苏州" | "苏州市" => Some("suzhou"),
        "nanjing" | "南京" | "南京市" => Some("nanjing"),
        "chengdu" | "成都" | "成都市" => Some("chengdu"),
        "wuhan" | "武汉" | "武汉市" => Some("wuhan"),
        "chongqing" | "重庆" | "重庆市" => Some("chongqing"),
        "tianjin" | "天津" | "天津市" => Some("tianjin"),
        "xian" | "西安" | "西安市" => Some("xian"),
        "changsha" | "长沙" | "长沙市" => Some("changsha"),
        "hefei" | "合肥" | "合肥市" => Some("hefei"),
        "qingdao" | "青岛" | "青岛市" => Some("qingdao"),
        "hongkong" | "hongkongsar" | "香港" | "香港特别行政区" => Some("hong_kong"),
        "taipei" | "台北" | "台北市" => Some("taipei"),
        "singapore" | "新加坡" => Some("singapore"),
        "sanfrancisco" | "sanfranciscobayarea" | "sfbayarea" | "bayarea" => Some("san_francisco"),
        "sanjose" => Some("san_jose"),
        "newyork" | "newyorkcity" | "nyc" | "纽约" | "纽约市" => Some("new_york"),
        "losangeles" | "la" => Some("los_angeles"),
        "seattle" => Some("seattle"),
        "boston" => Some("boston"),
        "austin" => Some("austin"),
        "toronto" => Some("toronto"),
        "vancouver" => Some("vancouver"),
        "tokyo" | "东京" => Some("tokyo"),
        "london" | "伦敦" => Some("london"),
        "berlin" => Some("berlin"),
        "paris" | "巴黎" => Some("paris"),
        "remote" | "远程" => Some("remote"),
        _ => None,
    }
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

fn certificate_alias_patterns() -> [(&'static str, f32, &'static str); 13] {
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
            "aws_security_specialty",
            0.9,
            r"(?i)\baws\s+(?:certified\s+)?security(?:\s*-\s*|\s+)specialty\b|\bscs-c0[12]\b",
        ),
        (
            "azure_administrator",
            0.88,
            r"(?i)\bazure\s+administrator\b|\baz-104\b",
        ),
        (
            "gcp_professional_data_engineer",
            0.9,
            r"(?i)\b(?:google\s+(?:cloud\s+)?|gcp\s+)?professional\s+data\s+engineer\b",
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
        (
            "ccna",
            0.88,
            r"(?i)\b(?:ccna|cisco\s+certified\s+network\s+associate)\b",
        ),
    ]
}

fn looks_like_certificate_alias(value: &str) -> bool {
    certificate_alias_patterns()
        .iter()
        .any(|(_, _, pattern)| Regex::new(pattern).unwrap().is_match(value))
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
