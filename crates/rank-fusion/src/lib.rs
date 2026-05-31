//! Field filter and candidate dedupe skeleton for local ranking.

use core_domain::EntityType;
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;

const STRONG_FIELD_CONFIDENCE: f32 = 0.75;
const S10_AS_OF_YEAR: i32 = 2026;
const S10_AS_OF_MONTH: i32 = 5;

/// Ordered degree levels used by field filters.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DegreeLevel {
    /// High-school level education.
    HighSchool,
    /// Associate degree or equivalent.
    Associate,
    /// Bachelor degree or equivalent.
    Bachelor,
    /// Master degree or equivalent.
    Master,
    /// Doctoral degree or equivalent.
    Doctor,
}

impl FromStr for DegreeLevel {
    type Err = DegreeParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match canonicalize_value(value).as_str() {
            "high_school" | "high school" | "高中" => Ok(Self::HighSchool),
            "associate" | "associate degree" | "大专" => Ok(Self::Associate),
            "bachelor" | "bachelors" | "bachelor degree" | "bachelor of science" | "本科" => {
                Ok(Self::Bachelor)
            }
            "master" | "masters" | "master degree" | "master of science" | "硕士" => {
                Ok(Self::Master)
            }
            "doctor" | "doctorate" | "phd" | "ph.d." | "doctor of philosophy" | "博士" => {
                Ok(Self::Doctor)
            }
            _ => Err(DegreeParseError),
        }
    }
}

/// Error returned when a degree filter value is not recognized.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DegreeParseError;

impl fmt::Display for DegreeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unknown degree level")
    }
}

impl std::error::Error for DegreeParseError {}

/// Caller-provided field filters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldFilters {
    /// Minimum accepted degree level.
    pub degree_min: Option<DegreeLevel>,
    /// Any accepted skill, matched case-insensitively against normalized skills.
    pub skills_any: Vec<String>,
    /// Minimum years of experience inferred from strong date ranges.
    pub years_experience_min: Option<f32>,
}

impl FieldFilters {
    /// Returns whether any field constraint is active.
    #[must_use]
    pub fn has_constraints(&self) -> bool {
        self.degree_min.is_some()
            || !self.skills_any.is_empty()
            || self.years_experience_min.is_some()
    }
}

/// Evidence-preserving field value from extractors.
#[derive(Clone, PartialEq)]
pub struct FieldEvidence {
    entity_type: EntityType,
    raw_evidence: String,
    normalized_value: Option<String>,
    confidence: f32,
}

impl FieldEvidence {
    /// Creates field evidence while keeping the original raw evidence local.
    #[must_use]
    pub fn new(
        entity_type: EntityType,
        raw_evidence: impl Into<String>,
        normalized_value: Option<&str>,
        confidence: f32,
    ) -> Self {
        Self {
            entity_type,
            raw_evidence: raw_evidence.into(),
            normalized_value: normalized_value.map(canonicalize_value),
            confidence,
        }
    }

    /// Returns the entity type.
    #[must_use]
    pub fn entity_type(&self) -> &EntityType {
        &self.entity_type
    }

    /// Returns the normalized field value when one exists.
    #[must_use]
    pub fn normalized_value(&self) -> Option<&str> {
        self.normalized_value.as_deref()
    }

    /// Returns the extraction confidence.
    #[must_use]
    pub fn confidence(&self) -> f32 {
        self.confidence
    }
}

impl fmt::Debug for FieldEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FieldEvidence")
            .field("entity_type", &self.entity_type)
            .field("raw_evidence", &"[redacted field evidence]")
            .field(
                "normalized_value",
                &self
                    .normalized_value
                    .as_ref()
                    .map(|_| "[redacted field value]"),
            )
            .field("confidence", &self.confidence)
            .finish()
    }
}

/// Normalized fields used for filtering and dedupe.
#[derive(Clone, PartialEq)]
pub struct FieldSummary {
    degree: Option<DegreeLevel>,
    skills: Vec<String>,
    years_experience: Option<f32>,
    email_key: Option<String>,
    phone_key: Option<String>,
}

impl FieldSummary {
    /// Builds a summary from strong extractor evidence.
    #[must_use]
    pub fn from_evidence(evidence: &[FieldEvidence]) -> Self {
        let mut summary = Self {
            degree: None,
            skills: Vec::new(),
            years_experience: None,
            email_key: None,
            phone_key: None,
        };

        for item in evidence {
            if item.confidence < STRONG_FIELD_CONFIDENCE {
                continue;
            }

            match item.entity_type() {
                EntityType::Other(kind) if kind == "degree" => {
                    let value = item
                        .normalized_value()
                        .unwrap_or(item.raw_evidence.as_str());
                    if let Ok(degree) = DegreeLevel::from_str(value) {
                        summary.degree = Some(summary.degree.map_or(degree, |current| {
                            if degree > current {
                                degree
                            } else {
                                current
                            }
                        }));
                    }
                }
                EntityType::Skill => {
                    let skill = item
                        .normalized_value()
                        .map_or_else(|| canonicalize_value(&item.raw_evidence), ToOwned::to_owned);
                    if !summary.skills.iter().any(|existing| existing == &skill) {
                        summary.skills.push(skill);
                    }
                }
                EntityType::Date => {
                    if let Some(years) = years_from_date_range(
                        item.normalized_value()
                            .unwrap_or(item.raw_evidence.as_str()),
                    ) {
                        summary.years_experience = Some(
                            summary
                                .years_experience
                                .map_or(years, |current| current + years),
                        );
                    }
                }
                EntityType::Email if summary.email_key.is_none() => {
                    let value = item
                        .normalized_value()
                        .map_or_else(|| canonicalize_value(&item.raw_evidence), ToOwned::to_owned);
                    summary.email_key = Some(hash_key("email", &value));
                }
                EntityType::Phone if summary.phone_key.is_none() => {
                    let value = item
                        .normalized_value()
                        .map_or_else(|| canonicalize_value(&item.raw_evidence), ToOwned::to_owned);
                    summary.phone_key = Some(hash_key("phone", &value));
                }
                _ => {}
            }
        }

        summary
    }

    /// Returns the highest normalized degree.
    #[must_use]
    pub fn degree(&self) -> Option<DegreeLevel> {
        self.degree
    }

    /// Returns normalized skills.
    #[must_use]
    pub fn skills(&self) -> &[String] {
        &self.skills
    }

    /// Returns inferred years of experience.
    #[must_use]
    pub fn years_experience(&self) -> Option<f32> {
        self.years_experience
    }

    /// Returns whether this summary satisfies all active field filters.
    #[must_use]
    pub fn matches(&self, filters: &FieldFilters) -> bool {
        if let Some(minimum) = filters.degree_min {
            if self.degree.is_none_or(|degree| degree < minimum) {
                return false;
            }
        }

        if !filters.skills_any.is_empty() {
            let requested = filters
                .skills_any
                .iter()
                .map(|skill| canonicalize_value(skill))
                .collect::<Vec<_>>();
            if !self
                .skills
                .iter()
                .any(|skill| requested.iter().any(|requested| requested == skill))
            {
                return false;
            }
        }

        if let Some(minimum) = filters.years_experience_min {
            if self
                .years_experience
                .is_none_or(|years| years + f32::EPSILON < minimum)
            {
                return false;
            }
        }

        true
    }

    fn dedupe_key(&self) -> Option<DedupeKey> {
        self.email_key
            .as_ref()
            .map(|key| DedupeKey {
                basis: DedupeBasis::Email,
                value: key.clone(),
            })
            .or_else(|| {
                self.phone_key.as_ref().map(|key| DedupeKey {
                    basis: DedupeBasis::Phone,
                    value: key.clone(),
                })
            })
    }
}

impl fmt::Debug for FieldSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FieldSummary")
            .field("degree", &self.degree)
            .field("skill_count", &self.skills.len())
            .field("years_experience", &self.years_experience)
            .field(
                "email_key",
                &self.email_key.as_ref().map(|_| "[redacted key]"),
            )
            .field(
                "phone_key",
                &self.phone_key.as_ref().map(|_| "[redacted key]"),
            )
            .finish()
    }
}

/// One searchable version entering soft dedupe grouping.
#[derive(Clone, PartialEq)]
pub struct CandidateRecord {
    doc_id: String,
    summary: FieldSummary,
}

impl CandidateRecord {
    /// Creates a candidate record for grouping.
    #[must_use]
    pub fn new(doc_id: impl Into<String>, summary: FieldSummary) -> Self {
        Self {
            doc_id: doc_id.into(),
            summary,
        }
    }
}

impl fmt::Debug for CandidateRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateRecord")
            .field("doc_id", &self.doc_id)
            .field("summary", &self.summary)
            .finish()
    }
}

/// Soft dedupe output group.
#[derive(Clone, PartialEq)]
pub struct CandidateGroup {
    key: Option<DedupeKey>,
    doc_ids: Vec<String>,
}

impl CandidateGroup {
    /// Returns grouped document identifiers.
    #[must_use]
    pub fn doc_ids(&self) -> Vec<&str> {
        self.doc_ids.iter().map(String::as_str).collect()
    }

    /// Returns the number of grouped resume versions.
    #[must_use]
    pub fn version_count(&self) -> usize {
        self.doc_ids.len()
    }
}

impl fmt::Debug for CandidateGroup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CandidateGroup")
            .field("key", &self.key)
            .field("doc_ids", &self.doc_ids)
            .field("version_count", &self.version_count())
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
struct DedupeKey {
    basis: DedupeBasis,
    value: String,
}

impl fmt::Debug for DedupeKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DedupeKey")
            .field("basis", &self.basis)
            .field("value", &"[redacted dedupe key]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DedupeBasis {
    Email,
    Phone,
}

/// Groups candidate versions by evidence-safe soft dedupe keys.
#[must_use]
pub fn group_soft_duplicates(records: Vec<CandidateRecord>) -> Vec<CandidateGroup> {
    let mut groups: Vec<CandidateGroup> = Vec::new();

    for record in records {
        let key = record.summary.dedupe_key();
        if let Some(existing) = groups
            .iter_mut()
            .find(|group| key.is_some() && group.key == key)
        {
            existing.doc_ids.push(record.doc_id);
        } else {
            groups.push(CandidateGroup {
                key,
                doc_ids: vec![record.doc_id],
            });
        }
    }

    groups
}

/// Returns the crate name for smoke tests and workspace metadata.
#[must_use]
pub fn crate_name() -> &'static str {
    "rank-fusion"
}

fn canonicalize_value(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_ascii_lowercase()
}

fn hash_key(namespace: &str, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(namespace.as_bytes());
    hasher.update(b":");
    hasher.update(value.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[derive(Clone, Copy)]
struct YearMonth {
    year: i32,
    month: i32,
}

fn years_from_date_range(value: &str) -> Option<f32> {
    let dates = year_months(value, has_open_ended_marker(value));
    let start = *dates.first()?;
    let end = *dates.get(1)?;
    let months = (end.year - start.year) * 12 + (end.month - start.month);
    if months <= 0 {
        return None;
    }
    Some(months as f32 / 12.0)
}

fn year_months(value: &str, open_ended: bool) -> Vec<YearMonth> {
    let normalized = value
        .replace('年', "-")
        .replace('月', "")
        .replace(['/', '.'], "-");
    let tokens = normalized
        .split(|character: char| !character.is_ascii_digit())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut dates = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        let token = tokens[index];
        if token.len() == 4 {
            if let Ok(year) = token.parse::<i32>() {
                if (1900..=2099).contains(&year) {
                    let mut month = 1;
                    if let Some(next) = tokens.get(index + 1) {
                        if next.len() <= 2 {
                            if let Ok(parsed_month) = next.parse::<i32>() {
                                if (1..=12).contains(&parsed_month) {
                                    month = parsed_month;
                                    index += 1;
                                }
                            }
                        }
                    }
                    dates.push(YearMonth { year, month });
                }
            }
        }
        index += 1;
    }

    if open_ended && dates.len() == 1 {
        dates.push(YearMonth {
            year: S10_AS_OF_YEAR,
            month: S10_AS_OF_MONTH,
        });
    }

    dates
}

fn has_open_ended_marker(value: &str) -> bool {
    let normalized = canonicalize_value(value);
    normalized.contains("present")
        || normalized.contains("current")
        || normalized.contains("now")
        || value.contains("至今")
}
