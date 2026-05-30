use core_domain::EntityType;
use regex::Regex;

#[derive(Clone, Debug, PartialEq)]
pub struct ExtractedField {
    pub entity_type: EntityType,
    pub raw_value: String,
    pub evidence: String,
    pub normalized_value: Option<String>,
    pub span_start: usize,
    pub span_end: usize,
    pub confidence: f32,
}

impl ExtractedField {
    #[must_use]
    pub fn is_strong_filterable(&self) -> bool {
        self.confidence >= 0.95
    }
}

pub fn extract_strong_fields(text: &str) -> Vec<ExtractedField> {
    extract_resume_fields(text)
        .into_iter()
        .filter(ExtractedField::is_strong_filterable)
        .collect()
}

pub fn extract_resume_fields(text: &str) -> Vec<ExtractedField> {
    let mut fields = Vec::new();
    fields.extend(extract_emails(text));
    fields.extend(extract_phones(text));
    fields.extend(extract_date_ranges(text));
    fields.extend(extract_schools(text));
    fields.extend(extract_degrees(text));
    fields.extend(extract_skills(text));
    fields
}

fn extract_emails(text: &str) -> Vec<ExtractedField> {
    let regex =
        Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").expect("valid email regex");
    regex
        .find_iter(text)
        .map(|matched| ExtractedField {
            entity_type: EntityType::Email,
            raw_value: matched.as_str().to_owned(),
            evidence: matched.as_str().to_owned(),
            normalized_value: Some(matched.as_str().to_ascii_lowercase()),
            span_start: matched.start(),
            span_end: matched.end(),
            confidence: 0.99,
        })
        .collect()
}

fn extract_phones(text: &str) -> Vec<ExtractedField> {
    let regex =
        Regex::new(r"(?:\+?86[- ]?)?1[3-9]\d[- ]?\d{4}[- ]?\d{4}").expect("valid phone regex");
    regex
        .find_iter(text)
        .filter_map(|matched| {
            let digits: String = matched
                .as_str()
                .chars()
                .filter(char::is_ascii_digit)
                .collect();
            let normalized = digits.strip_prefix("86").unwrap_or(&digits).to_owned();
            (normalized.len() == 11).then(|| ExtractedField {
                entity_type: EntityType::Phone,
                raw_value: matched.as_str().to_owned(),
                evidence: matched.as_str().to_owned(),
                normalized_value: Some(normalized),
                span_start: matched.start(),
                span_end: matched.end(),
                confidence: 0.98,
            })
        })
        .collect()
}

fn extract_date_ranges(text: &str) -> Vec<ExtractedField> {
    let regex = Regex::new(
        r"(?P<sy>\d{4})[./-](?P<sm>\d{1,2})\s*[-–至到]\s*(?P<ey>\d{4})[./-](?P<em>\d{1,2})",
    )
    .expect("valid date range regex");
    regex
        .captures_iter(text)
        .filter_map(|captures| {
            let matched = captures.get(0)?;
            let start_year = captures.name("sy")?.as_str();
            let start_month = format_month(captures.name("sm")?.as_str());
            let end_year = captures.name("ey")?.as_str();
            let end_month = format_month(captures.name("em")?.as_str());
            Some(ExtractedField {
                entity_type: EntityType::Date,
                raw_value: matched.as_str().to_owned(),
                evidence: matched.as_str().to_owned(),
                normalized_value: Some(format!(
                    "{start_year}-{start_month}..{end_year}-{end_month}"
                )),
                span_start: matched.start(),
                span_end: matched.end(),
                confidence: 0.96,
            })
        })
        .collect()
}

fn extract_schools(text: &str) -> Vec<ExtractedField> {
    [
        ("Zhejiang University", "zhejiang university"),
        (
            "Shanghai Jiao Tong University",
            "shanghai jiao tong university",
        ),
        ("Tsinghua University", "tsinghua university"),
        ("Peking University", "peking university"),
        ("浙江大学", "浙江大学"),
        ("清华大学", "清华大学"),
        ("北京大学", "北京大学"),
    ]
    .into_iter()
    .filter_map(|(needle, normalized)| {
        find_case_insensitive(text, needle).map(|(start, end)| ExtractedField {
            entity_type: EntityType::School,
            raw_value: text[start..end].to_owned(),
            evidence: text[start..end].to_owned(),
            normalized_value: Some(normalized.to_owned()),
            span_start: start,
            span_end: end,
            confidence: 0.93,
        })
    })
    .collect()
}

fn extract_degrees(text: &str) -> Vec<ExtractedField> {
    let regex = Regex::new(r"(?i)\b(bachelor|master|phd|doctorate)\b|本科|学士|硕士|博士")
        .expect("valid degree regex");
    regex
        .find_iter(text)
        .filter_map(|matched| {
            let normalized = normalize_degree(matched.as_str())?;
            Some(ExtractedField {
                entity_type: EntityType::Degree,
                raw_value: matched.as_str().to_owned(),
                evidence: matched.as_str().to_owned(),
                normalized_value: Some(normalized.to_owned()),
                span_start: matched.start(),
                span_end: matched.end(),
                confidence: 0.94,
            })
        })
        .collect()
}

fn normalize_degree(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "bachelor" => Some("bachelor"),
        "master" => Some("master"),
        "phd" | "doctorate" => Some("doctorate"),
        _ if value == "本科" || value == "学士" => Some("bachelor"),
        _ if value == "硕士" => Some("master"),
        _ if value == "博士" => Some("doctorate"),
        _ => None,
    }
}

fn extract_skills(text: &str) -> Vec<ExtractedField> {
    [
        "Java",
        "Rust",
        "Python",
        "Go",
        "Spring",
        "Redis",
        "SQL",
        "Kubernetes",
        "Docker",
    ]
    .into_iter()
    .flat_map(|skill| {
        let pattern = format!(r"(?i)\b{}\b", regex::escape(skill));
        let regex = Regex::new(&pattern).expect("valid skill regex");
        regex
            .find_iter(text)
            .map(|matched| ExtractedField {
                entity_type: EntityType::Skill,
                raw_value: matched.as_str().to_owned(),
                evidence: matched.as_str().to_owned(),
                normalized_value: Some(skill.to_ascii_lowercase()),
                span_start: matched.start(),
                span_end: matched.end(),
                confidence: 0.92,
            })
            .collect::<Vec<_>>()
    })
    .collect()
}

fn find_case_insensitive(text: &str, needle: &str) -> Option<(usize, usize)> {
    text.to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
        .map(|start| (start, start + needle.len()))
}

fn format_month(month: &str) -> String {
    format!("{:02}", month.parse::<u8>().unwrap_or(0))
}

#[must_use]
pub fn crate_name() -> &'static str {
    "extractor-rules"
}
