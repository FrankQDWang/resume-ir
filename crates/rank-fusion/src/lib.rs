use core_domain::EntityType;
use extractor_rules::ExtractedField;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DegreeLevel {
    Bachelor = 1,
    Master = 2,
    Doctorate = 3,
}

impl DegreeLevel {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "bachelor" | "本科" | "学士" => Some(Self::Bachelor),
            "master" | "硕士" => Some(Self::Master),
            "doctorate" | "phd" | "博士" => Some(Self::Doctorate),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bachelor => "bachelor",
            Self::Master => "master",
            Self::Doctorate => "doctorate",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FieldFilter {
    pub degree_min: Option<DegreeLevel>,
    pub skills_any: Vec<String>,
    pub years_experience_min: Option<f32>,
}

impl FieldFilter {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.degree_min.is_none()
            && self.skills_any.is_empty()
            && self.years_experience_min.is_none()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateProfile {
    pub doc_id: String,
    pub fields: Vec<ExtractedField>,
}

#[must_use]
pub fn filter_candidates(candidates: &[CandidateProfile], filters: &FieldFilter) -> Vec<String> {
    candidates
        .iter()
        .filter(|candidate| passes_field_filters(candidate, filters))
        .map(|candidate| candidate.doc_id.clone())
        .collect()
}

#[must_use]
pub fn passes_field_filters(candidate: &CandidateProfile, filters: &FieldFilter) -> bool {
    degree_matches(&candidate.fields, filters.degree_min)
        && skills_match(&candidate.fields, &filters.skills_any)
        && experience_matches(&candidate.fields, filters.years_experience_min)
}

#[must_use]
pub fn soft_dedupe_key(file_name: &str, fields: &[ExtractedField]) -> String {
    let school = first_normalized(fields, EntityType::School);
    let degree = first_normalized(fields, EntityType::Degree);
    match (school, degree) {
        (Some(school), Some(degree)) => format!("profile:{school}:{degree}"),
        _ => format!("file:{}", file_name.to_ascii_lowercase()),
    }
}

fn degree_matches(fields: &[ExtractedField], required: Option<DegreeLevel>) -> bool {
    let Some(required) = required else {
        return true;
    };
    fields
        .iter()
        .filter(|field| field.entity_type == EntityType::Degree)
        .filter_map(|field| field.normalized_value.as_deref())
        .filter_map(DegreeLevel::parse)
        .any(|level| level >= required)
}

fn skills_match(fields: &[ExtractedField], required: &[String]) -> bool {
    if required.is_empty() {
        return true;
    }
    fields
        .iter()
        .filter(|field| field.entity_type == EntityType::Skill)
        .filter_map(|field| field.normalized_value.as_deref())
        .any(|skill| {
            required
                .iter()
                .any(|required_skill| skill == required_skill.to_ascii_lowercase())
        })
}

fn experience_matches(fields: &[ExtractedField], required_years: Option<f32>) -> bool {
    let Some(required_years) = required_years else {
        return true;
    };
    max_years_from_dates(fields) >= required_years
}

fn max_years_from_dates(fields: &[ExtractedField]) -> f32 {
    fields
        .iter()
        .filter(|field| field.entity_type == EntityType::Date)
        .filter_map(|field| field.normalized_value.as_deref())
        .filter_map(date_range_months)
        .max()
        .map(|months| months as f32 / 12.0)
        .unwrap_or(0.0)
}

fn date_range_months(value: &str) -> Option<i32> {
    let (start, end) = value.split_once("..")?;
    let (start_year, start_month) = parse_year_month(start)?;
    let (end_year, end_month) = parse_year_month(end)?;
    let start_index = start_year * 12 + start_month;
    let end_index = end_year * 12 + end_month;
    (end_index >= start_index).then_some(end_index - start_index)
}

fn parse_year_month(value: &str) -> Option<(i32, i32)> {
    let (year, month) = value.split_once('-')?;
    Some((year.parse().ok()?, month.parse().ok()?))
}

fn first_normalized(fields: &[ExtractedField], entity_type: EntityType) -> Option<&str> {
    fields
        .iter()
        .find(|field| field.entity_type == entity_type)
        .and_then(|field| field.normalized_value.as_deref())
}

#[must_use]
pub fn crate_name() -> &'static str {
    "rank-fusion"
}
