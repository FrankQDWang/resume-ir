pub fn crate_name() -> &'static str {
    "extractor-rules"
}

use std::collections::BTreeSet;
use std::fmt;

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldType {
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
    let mut seen = BTreeSet::new();
    for (line_start, line) in indexed_lines(text) {
        if !looks_like_skill_line(line) {
            continue;
        }

        for (canonical, pattern) in [
            ("Spring Cloud", r"(?i)\bspring\s+cloud\b"),
            ("JavaScript", r"(?i)\b(?:java\s*script|javascript|js)\b"),
            ("SQLite", r"(?i)\bsqlite\b"),
            ("Tantivy", r"(?i)\btantivy\b"),
            ("MySQL", r"(?i)\bmysql\b"),
            ("Kubernetes", r"(?i)\bkubernetes\b"),
            ("Docker", r"(?i)\bdocker\b"),
            ("Python", r"(?i)\bpython\b"),
            ("Rust", r"(?i)\brust\b"),
            ("Java", r"(?i)\bjava\b"),
            ("SQL", r"(?i)\bsql\b"),
        ] {
            let regex = Regex::new(pattern).unwrap();
            for found in regex.find_iter(line) {
                if !seen.insert(canonical.to_string()) {
                    continue;
                }

                matches.push(RuleMatch {
                    field_type: FieldType::Skill,
                    raw_value: found.as_str().to_string(),
                    normalized_value: Some(canonical.to_string()),
                    span_start: line_start + found.start(),
                    span_end: line_start + found.end(),
                    confidence: 0.91,
                });
            }
        }
    }
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
    for (line_start, line) in indexed_lines(text) {
        let trimmed = line.trim();
        if trimmed.len() > 140 || !looks_like_certificate(trimmed) {
            continue;
        }

        let leading = line.len() - line.trim_start().len();
        let span_start = line_start + leading;
        let span_end = span_start + trimmed.len();
        matches.push(RuleMatch {
            field_type: FieldType::Certificate,
            raw_value: trimmed.to_string(),
            normalized_value: Some(trimmed.to_lowercase()),
            span_start,
            span_end,
            confidence: 0.86,
        });
    }
}

fn looks_like_certificate(line: &str) -> bool {
    let lower = line.to_lowercase();
    if matches!(
        lower.as_str(),
        "certificate" | "certificates" | "certifications"
    ) {
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
    lower.contains("skill")
        || lower.contains("technical stack")
        || lower.contains("技术栈")
        || lower.contains("技能")
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
