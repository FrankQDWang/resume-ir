//! Strong rule-based entity extraction for resume text.

use core_domain::EntityType;
use regex::Regex;
use std::fmt;
use std::sync::OnceLock;

/// Strong rule-based entity mention.
#[derive(Clone, PartialEq)]
pub struct StrongEntity {
    entity_type: EntityType,
    raw_value: String,
    normalized_value: Option<String>,
    span_start: u32,
    span_end: u32,
    confidence: f32,
    extractor: &'static str,
}

impl StrongEntity {
    /// Returns the extracted entity type.
    #[must_use]
    pub fn entity_type(&self) -> EntityType {
        self.entity_type.clone()
    }

    /// Returns the local raw value.
    #[must_use]
    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }

    /// Returns a normalized local value when available.
    #[must_use]
    pub fn normalized_value(&self) -> Option<&str> {
        self.normalized_value.as_deref()
    }

    /// Returns the start character offset.
    #[must_use]
    pub fn span_start(&self) -> u32 {
        self.span_start
    }

    /// Returns the end character offset.
    #[must_use]
    pub fn span_end(&self) -> u32 {
        self.span_end
    }

    /// Returns extraction confidence.
    #[must_use]
    pub fn confidence(&self) -> f32 {
        self.confidence
    }

    /// Returns the extractor identifier.
    #[must_use]
    pub fn extractor(&self) -> &'static str {
        self.extractor
    }
}

impl fmt::Debug for StrongEntity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StrongEntity")
            .field("entity_type", &self.entity_type)
            .field("raw_value", &"[redacted entity value]")
            .field(
                "normalized_value",
                &self
                    .normalized_value
                    .as_ref()
                    .map(|_| "[redacted normalized entity value]"),
            )
            .field("span_start", &self.span_start)
            .field("span_end", &self.span_end)
            .field("confidence", &self.confidence)
            .field("extractor", &self.extractor)
            .finish()
    }
}

/// Extracts deterministic strong-rule entities for the S10 field MVP.
#[must_use]
pub fn extract_strong_entities(text: &str) -> Vec<StrongEntity> {
    let mut entities = Vec::new();

    if let Some(regex) = email_regex() {
        for matched in regex.find_iter(text) {
            entities.push(entity(
                text,
                EntityType::Email,
                matched.start(),
                matched.end(),
                Some(matched.as_str().to_ascii_lowercase()),
                0.99,
                "strong-email",
            ));
        }
    }

    for regex in [date_range_regex(), chinese_date_range_regex()]
        .into_iter()
        .flatten()
    {
        for matched in regex.find_iter(text) {
            if !overlaps_existing(&entities, text, matched.start(), matched.end()) {
                entities.push(entity(
                    text,
                    EntityType::Date,
                    matched.start(),
                    matched.end(),
                    Some(normalize_spaces(matched.as_str())),
                    0.95,
                    "strong-date-range",
                ));
            }
        }
    }

    if let Some(regex) = phone_regex() {
        for matched in regex.find_iter(text) {
            if overlaps_existing(&entities, text, matched.start(), matched.end()) {
                continue;
            }

            let raw = matched.as_str();
            let digits = raw
                .chars()
                .filter(|character| character.is_ascii_digit())
                .collect::<String>();
            if !(10..=15).contains(&digits.len()) {
                continue;
            }

            let normalized = if raw.trim_start().starts_with('+') {
                format!("+{digits}")
            } else {
                digits
            };

            entities.push(entity(
                text,
                EntityType::Phone,
                matched.start(),
                matched.end(),
                Some(normalized),
                0.95,
                "strong-phone",
            ));
        }
    }

    push_dictionary_entities(
        &mut entities,
        text,
        EntityType::School,
        SCHOOL_RULES,
        0.88,
        "strong-school-dictionary",
    );
    push_dictionary_entities(
        &mut entities,
        text,
        EntityType::Other("degree".to_string()),
        DEGREE_RULES,
        0.96,
        "strong-degree-dictionary",
    );
    push_dictionary_entities(
        &mut entities,
        text,
        EntityType::Skill,
        SKILL_RULES,
        0.90,
        "strong-skill-dictionary",
    );

    entities.sort_by_key(|entity| entity.span_start);
    entities
}

struct DictionaryRule {
    evidence: &'static str,
    normalized: &'static str,
}

const SCHOOL_RULES: &[DictionaryRule] = &[
    DictionaryRule {
        evidence: "Massachusetts Institute of Technology",
        normalized: "massachusetts institute of technology",
    },
    DictionaryRule {
        evidence: "Synthetic State University",
        normalized: "synthetic state university",
    },
    DictionaryRule {
        evidence: "Stanford University",
        normalized: "stanford university",
    },
    DictionaryRule {
        evidence: "Tsinghua University",
        normalized: "tsinghua university",
    },
    DictionaryRule {
        evidence: "Peking University",
        normalized: "peking university",
    },
    DictionaryRule {
        evidence: "清华大学",
        normalized: "清华大学",
    },
    DictionaryRule {
        evidence: "北京大学",
        normalized: "北京大学",
    },
    DictionaryRule {
        evidence: "MIT",
        normalized: "massachusetts institute of technology",
    },
];

const DEGREE_RULES: &[DictionaryRule] = &[
    DictionaryRule {
        evidence: "Doctor of Philosophy",
        normalized: "doctor",
    },
    DictionaryRule {
        evidence: "Ph.D.",
        normalized: "doctor",
    },
    DictionaryRule {
        evidence: "PhD",
        normalized: "doctor",
    },
    DictionaryRule {
        evidence: "博士",
        normalized: "doctor",
    },
    DictionaryRule {
        evidence: "Master of Science",
        normalized: "master",
    },
    DictionaryRule {
        evidence: "Master's Degree",
        normalized: "master",
    },
    DictionaryRule {
        evidence: "Master",
        normalized: "master",
    },
    DictionaryRule {
        evidence: "硕士",
        normalized: "master",
    },
    DictionaryRule {
        evidence: "Bachelor of Science",
        normalized: "bachelor",
    },
    DictionaryRule {
        evidence: "Bachelor's Degree",
        normalized: "bachelor",
    },
    DictionaryRule {
        evidence: "Bachelor",
        normalized: "bachelor",
    },
    DictionaryRule {
        evidence: "本科",
        normalized: "bachelor",
    },
    DictionaryRule {
        evidence: "Associate Degree",
        normalized: "associate",
    },
    DictionaryRule {
        evidence: "Associate",
        normalized: "associate",
    },
    DictionaryRule {
        evidence: "大专",
        normalized: "associate",
    },
    DictionaryRule {
        evidence: "High School",
        normalized: "high_school",
    },
    DictionaryRule {
        evidence: "高中",
        normalized: "high_school",
    },
];

const SKILL_RULES: &[DictionaryRule] = &[
    DictionaryRule {
        evidence: "Spring Cloud",
        normalized: "spring cloud",
    },
    DictionaryRule {
        evidence: "TypeScript",
        normalized: "typescript",
    },
    DictionaryRule {
        evidence: "JavaScript",
        normalized: "javascript",
    },
    DictionaryRule {
        evidence: "Kubernetes",
        normalized: "kubernetes",
    },
    DictionaryRule {
        evidence: "TensorFlow",
        normalized: "tensorflow",
    },
    DictionaryRule {
        evidence: "PostgreSQL",
        normalized: "postgresql",
    },
    DictionaryRule {
        evidence: "Python",
        normalized: "python",
    },
    DictionaryRule {
        evidence: "Docker",
        normalized: "docker",
    },
    DictionaryRule {
        evidence: "Spark",
        normalized: "spark",
    },
    DictionaryRule {
        evidence: "MySQL",
        normalized: "mysql",
    },
    DictionaryRule {
        evidence: "Rust",
        normalized: "rust",
    },
    DictionaryRule {
        evidence: "Java",
        normalized: "java",
    },
    DictionaryRule {
        evidence: "SQL",
        normalized: "sql",
    },
    DictionaryRule {
        evidence: "AWS",
        normalized: "aws",
    },
    DictionaryRule {
        evidence: "Go",
        normalized: "go",
    },
];

fn push_dictionary_entities(
    entities: &mut Vec<StrongEntity>,
    text: &str,
    entity_type: EntityType,
    rules: &[DictionaryRule],
    confidence: f32,
    extractor: &'static str,
) {
    let lower_text = text.to_ascii_lowercase();

    for rule in rules {
        let needle = rule.evidence.to_ascii_lowercase();
        let mut search_start = 0;
        while let Some(relative_start) = lower_text[search_start..].find(&needle) {
            let start = search_start + relative_start;
            let end = start + needle.len();
            search_start = end;

            if !has_token_boundaries(text, start, end)
                || overlaps_existing(entities, text, start, end)
            {
                continue;
            }

            entities.push(entity(
                text,
                entity_type.clone(),
                start,
                end,
                Some(rule.normalized.to_string()),
                confidence,
                extractor,
            ));
        }
    }
}

fn entity(
    text: &str,
    entity_type: EntityType,
    byte_start: usize,
    byte_end: usize,
    normalized_value: Option<String>,
    confidence: f32,
    extractor: &'static str,
) -> StrongEntity {
    StrongEntity {
        entity_type,
        raw_value: text[byte_start..byte_end].to_owned(),
        normalized_value,
        span_start: saturating_usize_to_u32(byte_to_char_index(text, byte_start)),
        span_end: saturating_usize_to_u32(byte_to_char_index(text, byte_end)),
        confidence,
        extractor,
    }
}

fn overlaps_existing(
    entities: &[StrongEntity],
    text: &str,
    byte_start: usize,
    byte_end: usize,
) -> bool {
    let start = saturating_usize_to_u32(byte_to_char_index(text, byte_start));
    let end = saturating_usize_to_u32(byte_to_char_index(text, byte_end));

    entities
        .iter()
        .any(|entity| start < entity.span_end && end > entity.span_start)
}

fn normalize_spaces(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has_token_boundaries(text: &str, byte_start: usize, byte_end: usize) -> bool {
    is_token_boundary_before(text, byte_start) && is_token_boundary_after(text, byte_end)
}

fn is_token_boundary_before(text: &str, byte_start: usize) -> bool {
    if byte_start == 0 {
        return true;
    }
    text[..byte_start]
        .chars()
        .next_back()
        .is_none_or(is_boundary_character)
}

fn is_token_boundary_after(text: &str, byte_end: usize) -> bool {
    if byte_end >= text.len() {
        return true;
    }
    text[byte_end..]
        .chars()
        .next()
        .is_none_or(is_boundary_character)
}

fn is_boundary_character(character: char) -> bool {
    !character.is_ascii_alphanumeric()
}

fn byte_to_char_index(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().count()
}

fn saturating_usize_to_u32(value: usize) -> u32 {
    if value > u32::MAX as usize {
        u32::MAX
    } else {
        value as u32
    }
}

fn email_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"(?i)[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}").ok())
        .as_ref()
}

fn phone_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| Regex::new(r"(?:\+\d{1,3}[\s-]*)?(?:\d{2,4}[\s-]+){2,4}\d{2,4}").ok())
        .as_ref()
}

fn date_range_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
            r"(?ix)\b(?:19|20)\d{2}(?:[./-](?:0?[1-9]|1[0-2]))?\s*(?:-|to|~)\s*(?:(?:19|20)\d{2}(?:[./-](?:0?[1-9]|1[0-2]))?|present|current|now)\b",
        )
        .ok()
        })
        .as_ref()
}

fn chinese_date_range_regex() -> Option<&'static Regex> {
    static REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    REGEX
        .get_or_init(|| {
            Regex::new(
            r"(?x)(?:19|20)\d{2}年(?:0?[1-9]|1[0-2])?月?\s*(?:-|至|到|~)\s*(?:(?:19|20)\d{2}年(?:0?[1-9]|1[0-2])?月?|至今)",
        )
        .ok()
        })
        .as_ref()
}
