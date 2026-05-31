use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub fn crate_name() -> &'static str {
    "rank-fusion"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DegreeLevel {
    HighSchool,
    Associate,
    Bachelor,
    Master,
    Doctor,
}

impl DegreeLevel {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "high_school" | "high-school" | "highschool" | "high school" => Some(Self::HighSchool),
            "associate" | "associate_degree" | "associate degree" | "college" => {
                Some(Self::Associate)
            }
            "bachelor" | "undergraduate" | "bs" | "ba" => Some(Self::Bachelor),
            "master" | "ms" | "ma" | "mba" => Some(Self::Master),
            "doctor" | "doctorate" | "phd" | "ph.d." => Some(Self::Doctor),
            _ => None,
        }
    }

    pub fn canonical(self) -> &'static str {
        match self {
            Self::HighSchool => "high_school",
            Self::Associate => "associate",
            Self::Bachelor => "bachelor",
            Self::Master => "master",
            Self::Doctor => "doctor",
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct ResumeProfile {
    doc_id: String,
    degree: Option<DegreeLevel>,
    skills: Vec<String>,
    years_experience: Option<f32>,
}

impl ResumeProfile {
    pub fn new(doc_id: impl Into<String>) -> Self {
        Self {
            doc_id: doc_id.into(),
            degree: None,
            skills: Vec::new(),
            years_experience: None,
        }
    }

    pub fn with_degree(mut self, degree: DegreeLevel) -> Self {
        self.degree = Some(degree);
        self
    }

    pub fn with_skills<I, S>(mut self, skills: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.skills = skills
            .into_iter()
            .map(|skill| normalize_skill(skill.as_ref()))
            .filter(|skill| !skill.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_years_experience(mut self, years_experience: f32) -> Self {
        self.years_experience = Some(years_experience.max(0.0));
        self
    }

    pub fn degree(&self) -> Option<DegreeLevel> {
        self.degree
    }

    pub fn skills(&self) -> &[String] {
        &self.skills
    }

    pub fn years_experience(&self) -> Option<f32> {
        self.years_experience
    }
}

impl fmt::Debug for ResumeProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResumeProfile")
            .field("doc_id", &self.doc_id)
            .field("degree", &self.degree)
            .field("skill_count", &self.skills.len())
            .field("years_experience", &self.years_experience)
            .finish()
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct SearchFilters {
    degree_min: Option<DegreeLevel>,
    skills_any: Vec<String>,
    years_experience_min: Option<f32>,
}

impl SearchFilters {
    pub fn with_degree_min(mut self, degree: DegreeLevel) -> Self {
        self.degree_min = Some(degree);
        self
    }

    pub fn with_skills_any<I, S>(mut self, skills: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.skills_any = skills
            .into_iter()
            .map(|skill| normalize_skill(skill.as_ref()))
            .filter(|skill| !skill.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_years_experience_min(mut self, years: f32) -> Self {
        self.years_experience_min = Some(years.max(0.0));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.degree_min.is_none()
            && self.skills_any.is_empty()
            && self.years_experience_min.is_none()
    }

    pub fn matches(&self, profile: &ResumeProfile) -> bool {
        if let Some(min_degree) = self.degree_min {
            if profile.degree().is_none_or(|degree| degree < min_degree) {
                return false;
            }
        }

        if !self.skills_any.is_empty() {
            let profile_skills = profile.skills().iter().collect::<BTreeSet<_>>();
            if !self
                .skills_any
                .iter()
                .any(|skill| profile_skills.contains(skill))
            {
                return false;
            }
        }

        if let Some(min_years) = self.years_experience_min {
            if profile
                .years_experience()
                .is_none_or(|years| years < min_years)
            {
                return false;
            }
        }

        true
    }
}

impl fmt::Debug for SearchFilters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchFilters")
            .field("degree_min", &self.degree_min)
            .field("skills_any_count", &self.skills_any.len())
            .field("years_experience_min", &self.years_experience_min)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct RankedHit {
    doc_id: String,
    rank: usize,
    score: f32,
    candidate_key: Option<String>,
}

impl RankedHit {
    pub fn new(doc_id: impl Into<String>, rank: usize, score: f32) -> Self {
        Self {
            doc_id: doc_id.into(),
            rank,
            score,
            candidate_key: None,
        }
    }

    pub fn with_candidate_key(mut self, candidate_key: impl Into<String>) -> Self {
        self.candidate_key = Some(candidate_key.into());
        self
    }

    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    pub fn score(&self) -> f32 {
        self.score
    }
}

impl fmt::Debug for RankedHit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RankedHit")
            .field("doc_id", &self.doc_id)
            .field("rank", &self.rank)
            .field("score", &self.score)
            .field(
                "candidate_key",
                &self.candidate_key.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

pub fn fold_by_candidate(hits: Vec<RankedHit>) -> Vec<RankedHit> {
    let mut seen = BTreeSet::new();
    let mut folded = Vec::new();

    for hit in hits {
        let key = hit
            .candidate_key
            .clone()
            .unwrap_or_else(|| format!("doc:{}", hit.doc_id));
        if seen.insert(key) {
            folded.push(hit);
        }
    }

    folded
}

pub fn reciprocal_rank_fusion<I>(channels: I, k: f32) -> Vec<RankedHit>
where
    I: IntoIterator<Item = Vec<String>>,
{
    let k = k.max(1.0);
    let mut scores = BTreeMap::<String, f32>::new();

    for channel in channels {
        for (index, doc_id) in channel.into_iter().enumerate() {
            let rank = index + 1;
            *scores.entry(doc_id).or_insert(0.0) += 1.0 / (k + rank as f32);
        }
    }

    let mut fused = scores.into_iter().collect::<Vec<_>>();
    fused.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });

    fused
        .into_iter()
        .enumerate()
        .map(|(index, (doc_id, score))| RankedHit::new(doc_id, index + 1, score))
        .collect()
}

fn normalize_skill(skill: &str) -> String {
    skill
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}
