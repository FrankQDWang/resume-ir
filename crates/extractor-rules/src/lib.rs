pub fn crate_name() -> &'static str {
    "extractor-rules"
}

use std::fmt;

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldType {
    Email,
    Phone,
    DateRange,
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
