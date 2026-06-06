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
        let lowered = value.trim().to_ascii_lowercase();
        let dotted_bachelor_engineering = lowered.split_whitespace().collect::<String>() == "b.e.";
        let compact = lowered
            .chars()
            .filter(|character| !matches!(character, ' ' | '.' | '-' | '_'))
            .collect::<String>();
        match compact.as_str() {
            "highschool" | "高中" | "高中学历" => Some(Self::HighSchool),
            "associate" | "associatedegree" | "college" | "大专" | "大专学历" | "专科"
            | "专科学历" => Some(Self::Associate),
            "bachelor" | "undergraduate" | "bs" | "ba" | "btech" | "beng" | "本科" | "大学本科"
            | "本科生" | "学士" | "学士学位" => Some(Self::Bachelor),
            "be" if dotted_bachelor_engineering => Some(Self::Bachelor),
            "master" | "ms" | "ma" | "mba" | "msc" | "meng" | "mtech" | "mphil" | "硕士"
            | "硕士研究生" | "硕士学位" | "研究生" => Some(Self::Master),
            "doctor" | "doctorate" | "phd" | "博士" | "博士研究生" | "博士学位" | "博士生" => {
                Some(Self::Doctor)
            }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SchoolTier {
    Tier985,
    Tier211,
    DoubleFirstClass,
    Overseas,
    Regular,
    Unknown,
}

impl SchoolTier {
    pub fn parse(value: &str) -> Option<Self> {
        let lowered = value.trim().to_ascii_lowercase();
        let compact = lowered
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect::<String>();
        match compact.as_str() {
            "985" | "project985" | "985project" | "985工程" | "c9league" => Some(Self::Tier985),
            "211" | "project211" | "211project" | "211工程" => Some(Self::Tier211),
            "doublefirstclass"
            | "doublefirstclassuniversity"
            | "双一流"
            | "双一流建设高校"
            | "双一流建设大学"
            | "双一流高校"
            | "双一流院校" => Some(Self::DoubleFirstClass),
            "overseas"
            | "oversea"
            | "foreign"
            | "foreignuniversity"
            | "international"
            | "internationaluniversity"
            | "ivyleague"
            | "russellgroup"
            | "海外"
            | "国外"
            | "海外高校"
            | "海外院校" => Some(Self::Overseas),
            "regular" | "regularuniversity" | "ordinary" | "ordinaryuniversity"
            | "ordinarycollege" | "normal" | "普通" | "普通高校" | "普通院校" | "普通本科" => {
                Some(Self::Regular)
            }
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }

    pub fn canonical(self) -> &'static str {
        match self {
            Self::Tier985 => "985",
            Self::Tier211 => "211",
            Self::DoubleFirstClass => "double_first_class",
            Self::Overseas => "overseas",
            Self::Regular => "regular",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateRange {
    start_month: i32,
    end_month: Option<i32>,
}

impl DateRange {
    pub fn parse(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        let (start, end) = trimmed
            .split_once('/')
            .or_else(|| trimmed.split_once(".."))?;
        let start_month = parse_year_month(start.trim())?;
        let end = end.trim();
        let end_month = if matches!(
            end.to_ascii_lowercase().as_str(),
            "present" | "current" | "now" | "ongoing"
        ) {
            None
        } else {
            Some(parse_year_month(end)?)
        };
        Self::from_month_bounds(start_month, end_month)
    }

    pub fn from_month_bounds(start_month: i32, end_month: Option<i32>) -> Option<Self> {
        if !is_supported_month_index(start_month) {
            return None;
        }
        if let Some(end_month) = end_month {
            if !is_supported_month_index(end_month) || end_month < start_month {
                return None;
            }
        }
        Some(Self {
            start_month,
            end_month,
        })
    }

    pub fn start_month(self) -> i32 {
        self.start_month
    }

    pub fn end_month(self) -> Option<i32> {
        self.end_month
    }

    pub fn overlaps(self, other: Self) -> bool {
        let self_end = self.end_month.unwrap_or(i32::MAX);
        let other_end = other.end_month.unwrap_or(i32::MAX);
        self.start_month <= other_end && other.start_month <= self_end
    }

    pub fn canonical(self) -> String {
        let end = self
            .end_month
            .map(format_year_month)
            .unwrap_or_else(|| "PRESENT".to_string());
        format!("{}/{}", format_year_month(self.start_month), end)
    }
}

#[derive(Clone, PartialEq)]
pub struct ResumeProfile {
    doc_id: String,
    names: Vec<String>,
    degree: Option<DegreeLevel>,
    schools: Vec<String>,
    school_tiers: Vec<SchoolTier>,
    majors: Vec<String>,
    certificates: Vec<String>,
    date_ranges: Vec<DateRange>,
    companies: Vec<String>,
    titles: Vec<String>,
    locations: Vec<String>,
    skills: Vec<String>,
    years_experience: Option<f32>,
}

impl ResumeProfile {
    pub fn new(doc_id: impl Into<String>) -> Self {
        Self {
            doc_id: doc_id.into(),
            names: Vec::new(),
            degree: None,
            schools: Vec::new(),
            school_tiers: Vec::new(),
            majors: Vec::new(),
            certificates: Vec::new(),
            date_ranges: Vec::new(),
            companies: Vec::new(),
            titles: Vec::new(),
            locations: Vec::new(),
            skills: Vec::new(),
            years_experience: None,
        }
    }

    pub fn with_degree(mut self, degree: DegreeLevel) -> Self {
        self.degree = Some(degree);
        self
    }

    pub fn with_names<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.names = names
            .into_iter()
            .map(|name| normalize_name(name.as_ref()))
            .filter(|name| !name.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_school_tiers<I>(mut self, school_tiers: I) -> Self
    where
        I: IntoIterator<Item = SchoolTier>,
    {
        self.school_tiers = school_tiers
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_schools<I, S>(mut self, schools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.schools = schools
            .into_iter()
            .map(|school| normalize_school(school.as_ref()))
            .filter(|school| !school.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_majors<I, S>(mut self, majors: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.majors = majors
            .into_iter()
            .map(|major| normalize_major(major.as_ref()))
            .filter(|major| !major.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_certificates<I, S>(mut self, certificates: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.certificates = certificates
            .into_iter()
            .map(|certificate| normalize_certificate(certificate.as_ref()))
            .filter(|certificate| !certificate.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_date_ranges<I, S>(mut self, date_ranges: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.date_ranges = date_ranges
            .into_iter()
            .filter_map(|date_range| DateRange::parse(date_range.as_ref()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_companies<I, S>(mut self, companies: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.companies = companies
            .into_iter()
            .map(|company| normalize_company(company.as_ref()))
            .filter(|company| !company.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_titles<I, S>(mut self, titles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.titles = titles
            .into_iter()
            .map(|title| normalize_title(title.as_ref()))
            .filter(|title| !title.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_locations<I, S>(mut self, locations: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.locations = locations
            .into_iter()
            .map(|location| normalize_location(location.as_ref()))
            .filter(|location| !location.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
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

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn school_tiers(&self) -> &[SchoolTier] {
        &self.school_tiers
    }

    pub fn schools(&self) -> &[String] {
        &self.schools
    }

    pub fn majors(&self) -> &[String] {
        &self.majors
    }

    pub fn certificates(&self) -> &[String] {
        &self.certificates
    }

    pub fn date_ranges(&self) -> &[DateRange] {
        &self.date_ranges
    }

    pub fn companies(&self) -> &[String] {
        &self.companies
    }

    pub fn titles(&self) -> &[String] {
        &self.titles
    }

    pub fn locations(&self) -> &[String] {
        &self.locations
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
            .field("name_count", &self.names.len())
            .field("degree", &self.degree)
            .field("school_count", &self.schools.len())
            .field("school_tier_count", &self.school_tiers.len())
            .field("major_count", &self.majors.len())
            .field("certificate_count", &self.certificates.len())
            .field("date_range_count", &self.date_ranges.len())
            .field("company_count", &self.companies.len())
            .field("title_count", &self.titles.len())
            .field("location_count", &self.locations.len())
            .field("skill_count", &self.skills.len())
            .field("years_experience", &self.years_experience)
            .finish()
    }
}

#[derive(Clone, Default, PartialEq)]
pub struct SearchFilters {
    degree_min: Option<DegreeLevel>,
    names_any: Vec<String>,
    schools_any: Vec<String>,
    school_tiers_any: Vec<SchoolTier>,
    majors_any: Vec<String>,
    certificates_any: Vec<String>,
    date_range_overlaps: Option<DateRange>,
    companies_any: Vec<String>,
    titles_any: Vec<String>,
    locations_any: Vec<String>,
    skills_any: Vec<String>,
    contact_hashes_any: Vec<String>,
    years_experience_min: Option<f32>,
}

impl SearchFilters {
    pub fn with_degree_min(mut self, degree: DegreeLevel) -> Self {
        self.degree_min = Some(degree);
        self
    }

    pub fn with_names_any<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.names_any = names
            .into_iter()
            .map(|name| normalize_name(name.as_ref()))
            .filter(|name| !name.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_school_tiers_any<I>(mut self, school_tiers: I) -> Self
    where
        I: IntoIterator<Item = SchoolTier>,
    {
        self.school_tiers_any = school_tiers
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_schools_any<I, S>(mut self, schools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.schools_any = schools
            .into_iter()
            .map(|school| normalize_school(school.as_ref()))
            .filter(|school| !school.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_majors_any<I, S>(mut self, majors: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.majors_any = majors
            .into_iter()
            .map(|major| normalize_major(major.as_ref()))
            .filter(|major| !major.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_certificates_any<I, S>(mut self, certificates: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.certificates_any = certificates
            .into_iter()
            .map(|certificate| normalize_certificate(certificate.as_ref()))
            .filter(|certificate| !certificate.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_date_range_overlaps(mut self, date_range: &str) -> Self {
        self.date_range_overlaps = DateRange::parse(date_range);
        self
    }

    pub fn with_companies_any<I, S>(mut self, companies: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.companies_any = companies
            .into_iter()
            .map(|company| normalize_company(company.as_ref()))
            .filter(|company| !company.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_titles_any<I, S>(mut self, titles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.titles_any = titles
            .into_iter()
            .map(|title| normalize_title(title.as_ref()))
            .filter(|title| !title.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }

    pub fn with_locations_any<I, S>(mut self, locations: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.locations_any = locations
            .into_iter()
            .map(|location| normalize_location(location.as_ref()))
            .filter(|location| !location.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
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

    pub fn with_contact_hashes_any<I, S>(mut self, contact_hashes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.contact_hashes_any = contact_hashes
            .into_iter()
            .filter_map(|contact_hash| normalize_contact_hash(contact_hash.as_ref()))
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
            && self.names_any.is_empty()
            && self.schools_any.is_empty()
            && self.school_tiers_any.is_empty()
            && self.majors_any.is_empty()
            && self.certificates_any.is_empty()
            && self.date_range_overlaps.is_none()
            && self.companies_any.is_empty()
            && self.titles_any.is_empty()
            && self.locations_any.is_empty()
            && self.skills_any.is_empty()
            && self.contact_hashes_any.is_empty()
            && self.years_experience_min.is_none()
    }

    pub fn degree_min(&self) -> Option<DegreeLevel> {
        self.degree_min
    }

    pub fn names_any(&self) -> &[String] {
        &self.names_any
    }

    pub fn skills_any(&self) -> &[String] {
        &self.skills_any
    }

    pub fn school_tiers_any(&self) -> &[SchoolTier] {
        &self.school_tiers_any
    }

    pub fn schools_any(&self) -> &[String] {
        &self.schools_any
    }

    pub fn majors_any(&self) -> &[String] {
        &self.majors_any
    }

    pub fn certificates_any(&self) -> &[String] {
        &self.certificates_any
    }

    pub fn date_range_overlaps(&self) -> Option<DateRange> {
        self.date_range_overlaps
    }

    pub fn companies_any(&self) -> &[String] {
        &self.companies_any
    }

    pub fn titles_any(&self) -> &[String] {
        &self.titles_any
    }

    pub fn locations_any(&self) -> &[String] {
        &self.locations_any
    }

    pub fn contact_hashes_any(&self) -> &[String] {
        &self.contact_hashes_any
    }

    pub fn years_experience_min(&self) -> Option<f32> {
        self.years_experience_min
    }

    pub fn matches(&self, profile: &ResumeProfile) -> bool {
        if let Some(min_degree) = self.degree_min {
            if profile.degree().is_none_or(|degree| degree < min_degree) {
                return false;
            }
        }

        if !self.names_any.is_empty() {
            let profile_names = profile.names().iter().collect::<BTreeSet<_>>();
            if !self
                .names_any
                .iter()
                .any(|name| profile_names.contains(name))
            {
                return false;
            }
        }

        if !self.school_tiers_any.is_empty()
            && !school_tiers_match_any(&self.school_tiers_any, profile.school_tiers())
        {
            return false;
        }

        if !self.schools_any.is_empty() {
            let profile_schools = profile.schools().iter().collect::<BTreeSet<_>>();
            if !self
                .schools_any
                .iter()
                .any(|school| profile_schools.contains(school))
            {
                return false;
            }
        }

        if !self.majors_any.is_empty() {
            let profile_majors = profile.majors().iter().collect::<BTreeSet<_>>();
            if !self
                .majors_any
                .iter()
                .any(|major| profile_majors.contains(major))
            {
                return false;
            }
        }

        if !self.certificates_any.is_empty() {
            let profile_certificates = profile.certificates().iter().collect::<BTreeSet<_>>();
            if !self
                .certificates_any
                .iter()
                .any(|certificate| profile_certificates.contains(certificate))
            {
                return false;
            }
        }

        if let Some(filter_range) = self.date_range_overlaps {
            if !profile
                .date_ranges()
                .iter()
                .any(|profile_range| profile_range.overlaps(filter_range))
            {
                return false;
            }
        }

        if !self.companies_any.is_empty() {
            let profile_companies = profile.companies().iter().collect::<BTreeSet<_>>();
            if !self
                .companies_any
                .iter()
                .any(|company| profile_companies.contains(company))
            {
                return false;
            }
        }

        if !self.titles_any.is_empty() {
            let profile_titles = profile.titles().iter().collect::<BTreeSet<_>>();
            if !self
                .titles_any
                .iter()
                .any(|title| profile_titles.contains(title))
            {
                return false;
            }
        }

        if !self.locations_any.is_empty() {
            let profile_locations = profile.locations().iter().collect::<BTreeSet<_>>();
            if !self
                .locations_any
                .iter()
                .any(|location| profile_locations.contains(location))
            {
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

fn school_tiers_match_any(filters: &[SchoolTier], profile_tiers: &[SchoolTier]) -> bool {
    let requested_unknown = filters.contains(&SchoolTier::Unknown);
    if requested_unknown
        && (profile_tiers.is_empty() || profile_tiers.contains(&SchoolTier::Unknown))
    {
        return true;
    }

    let profile_tiers = profile_tiers.iter().collect::<BTreeSet<_>>();
    filters
        .iter()
        .filter(|school_tier| **school_tier != SchoolTier::Unknown)
        .any(|school_tier| profile_tiers.contains(school_tier))
}

impl fmt::Debug for SearchFilters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SearchFilters")
            .field("degree_min", &self.degree_min)
            .field("schools_any_count", &self.schools_any.len())
            .field("school_tiers_any_count", &self.school_tiers_any.len())
            .field("majors_any_count", &self.majors_any.len())
            .field("certificates_any_count", &self.certificates_any.len())
            .field(
                "date_range_overlaps",
                &self.date_range_overlaps.map(DateRange::canonical),
            )
            .field("companies_any_count", &self.companies_any.len())
            .field("titles_any_count", &self.titles_any.len())
            .field("locations_any_count", &self.locations_any.len())
            .field("skills_any_count", &self.skills_any.len())
            .field("contact_hashes_any_count", &self.contact_hashes_any.len())
            .field("years_experience_min", &self.years_experience_min)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct DedupeProfile {
    doc_id: String,
    name: Option<String>,
    schools: BTreeSet<String>,
    companies: BTreeSet<String>,
    skills: BTreeSet<String>,
}

impl DedupeProfile {
    pub fn new(doc_id: impl Into<String>) -> Self {
        Self {
            doc_id: doc_id.into(),
            name: None,
            schools: BTreeSet::new(),
            companies: BTreeSet::new(),
            skills: BTreeSet::new(),
        }
    }

    pub fn with_name(mut self, name: &str) -> Self {
        let name = normalize_dedupe_value(name);
        if !name.is_empty() {
            self.name = Some(name);
        }
        self
    }

    pub fn with_schools<I, S>(mut self, schools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.schools = normalize_dedupe_values(schools);
        self
    }

    pub fn with_companies<I, S>(mut self, companies: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.companies = normalize_dedupe_values(companies);
        self
    }

    pub fn with_skills<I, S>(mut self, skills: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.skills = normalize_dedupe_values(skills);
        self
    }

    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl fmt::Debug for DedupeProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DedupeProfile")
            .field("doc_id", &self.doc_id)
            .field("has_name", &self.name.is_some())
            .field("school_count", &self.schools.len())
            .field("company_count", &self.companies.len())
            .field("skill_count", &self.skills.len())
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct SoftDedupeScore {
    left_doc_id: String,
    right_doc_id: String,
    confidence: f32,
    shared_school_count: usize,
    shared_company_count: usize,
    shared_skill_count: usize,
}

impl SoftDedupeScore {
    pub fn left_doc_id(&self) -> &str {
        &self.left_doc_id
    }

    pub fn right_doc_id(&self) -> &str {
        &self.right_doc_id
    }

    pub fn confidence(&self) -> f32 {
        self.confidence
    }

    pub fn shared_school_count(&self) -> usize {
        self.shared_school_count
    }

    pub fn shared_skill_count(&self) -> usize {
        self.shared_skill_count
    }
}

impl fmt::Debug for SoftDedupeScore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SoftDedupeScore")
            .field("left_doc_id", &self.left_doc_id)
            .field("right_doc_id", &self.right_doc_id)
            .field("confidence", &self.confidence)
            .field("shared_school_count", &self.shared_school_count)
            .field("shared_company_count", &self.shared_company_count)
            .field("shared_skill_count", &self.shared_skill_count)
            .finish()
    }
}

pub fn soft_dedupe_score(left: &DedupeProfile, right: &DedupeProfile) -> Option<SoftDedupeScore> {
    if left.doc_id == right.doc_id {
        return None;
    }
    if left.name.as_deref()? != right.name.as_deref()? {
        return None;
    }

    let shared_school_count = intersection_count(&left.schools, &right.schools);
    let shared_company_count = intersection_count(&left.companies, &right.companies);
    let shared_skill_count = intersection_count(&left.skills, &right.skills);
    let skill_score = shared_skill_count.min(3) as f32 / 3.0;
    let confidence = (0.45
        + (shared_school_count > 0) as u8 as f32 * 0.25
        + (shared_company_count > 0) as u8 as f32 * 0.20
        + skill_score * 0.10)
        .min(0.95);

    (confidence > 0.70).then(|| SoftDedupeScore {
        left_doc_id: left.doc_id.clone(),
        right_doc_id: right.doc_id.clone(),
        confidence,
        shared_school_count,
        shared_company_count,
        shared_skill_count,
    })
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

pub fn reciprocal_rank_fusion_hits<I>(channels: I, k: f32) -> Vec<RankedHit>
where
    I: IntoIterator<Item = Vec<RankedHit>>,
{
    let k = k.max(1.0);
    let mut scores = BTreeMap::<String, f32>::new();
    let mut candidate_keys = BTreeMap::<String, Option<String>>::new();

    for channel in channels {
        for (index, hit) in channel.into_iter().enumerate() {
            let rank = index + 1;
            *scores.entry(hit.doc_id.clone()).or_insert(0.0) += 1.0 / (k + rank as f32);
            candidate_keys
                .entry(hit.doc_id)
                .or_insert(hit.candidate_key);
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
        .map(|(index, (doc_id, score))| {
            let mut hit = RankedHit::new(doc_id.clone(), index + 1, score);
            if let Some(Some(candidate_key)) = candidate_keys.remove(&doc_id) {
                hit = hit.with_candidate_key(candidate_key);
            }
            hit
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetrievalChannel {
    FullText,
    Vector,
}

#[derive(Clone, PartialEq)]
pub struct RankedChannel {
    channel: RetrievalChannel,
    doc_ids: Vec<String>,
}

impl RankedChannel {
    pub fn new<I, S>(channel: RetrievalChannel, doc_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            channel,
            doc_ids: doc_ids
                .into_iter()
                .map(|doc_id| doc_id.as_ref().to_string())
                .collect(),
        }
    }

    pub fn channel(&self) -> RetrievalChannel {
        self.channel
    }

    pub fn len(&self) -> usize {
        self.doc_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.doc_ids.is_empty()
    }
}

impl fmt::Debug for RankedChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RankedChannel")
            .field("channel", &self.channel)
            .field("doc_count", &self.doc_ids.len())
            .finish()
    }
}

pub fn fuse_ranked_channels<I>(channels: I, k: f32) -> Vec<RankedHit>
where
    I: IntoIterator<Item = RankedChannel>,
{
    reciprocal_rank_fusion(channels.into_iter().map(|channel| channel.doc_ids), k)
}

#[derive(Clone, Default, PartialEq)]
pub struct HybridRecall {
    fulltext: Vec<RankedHit>,
    vector: Vec<RankedHit>,
}

impl HybridRecall {
    pub fn new(fulltext: Vec<RankedHit>, vector: Vec<RankedHit>) -> Self {
        Self { fulltext, vector }
    }
}

impl fmt::Debug for HybridRecall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HybridRecall")
            .field("fulltext_count", &self.fulltext.len())
            .field("vector_count", &self.vector.len())
            .finish()
    }
}

pub fn fuse_hybrid_rrf(recall: HybridRecall, k: f32, limit: usize) -> Vec<RankedHit> {
    let mut fused = reciprocal_rank_fusion_hits([recall.fulltext, recall.vector], k);
    fused.truncate(limit);
    fused
}

fn normalize_skill(skill: &str) -> String {
    let normalized = normalize_dedupe_value(skill);
    let compact = normalized
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect::<String>();
    match compact.as_str() {
        "k8s" | "kubernetes" => "kubernetes".to_string(),
        "golang" | "go" => "go".to_string(),
        "postgres" | "postgresql" => "postgresql".to_string(),
        "nodejs" | "node" => "node.js".to_string(),
        "reactjs" | "react" => "react".to_string(),
        "vuejs" | "vue" => "vue.js".to_string(),
        "ts" | "typescript" => "typescript".to_string(),
        "js" | "javascript" => "javascript".to_string(),
        "sklearn" | "scikitlearn" => "scikit-learn".to_string(),
        "pytorch" => "pytorch".to_string(),
        "tensorflow" => "tensorflow".to_string(),
        "graphql" => "graphql".to_string(),
        "amazonwebservices" | "aws" => "aws".to_string(),
        "microsoftazure" | "azure" => "azure".to_string(),
        "googlecloudplatform" | "googlecloud" | "gcp" => "gcp".to_string(),
        "terraform" => "terraform".to_string(),
        "ansible" => "ansible".to_string(),
        "jenkins" => "jenkins".to_string(),
        "gitlabci" | "gitlabcicd" => "gitlab ci".to_string(),
        "apachekafka" | "kafka" => "kafka".to_string(),
        "apacheflink" | "flink" => "flink".to_string(),
        "elasticsearch" | "elastic" => "elasticsearch".to_string(),
        "mongodb" => "mongodb".to_string(),
        "snowflake" => "snowflake".to_string(),
        _ => normalized,
    }
}

fn normalize_contact_hash(contact_hash: &str) -> Option<String> {
    let value = contact_hash.trim();
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn normalize_name(name: &str) -> String {
    normalize_dedupe_value(name)
}

fn parse_year_month(value: &str) -> Option<i32> {
    let (year, month) = value.trim().split_once('-')?;
    let year = year.parse::<i32>().ok()?;
    let month = month.parse::<i32>().ok()?;
    if !(1900..=2100).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    Some(year * 12 + month)
}

fn format_year_month(month_index: i32) -> String {
    let zero_based = month_index - 1;
    let year = zero_based.div_euclid(12);
    let month = zero_based.rem_euclid(12) + 1;
    format!("{year:04}-{month:02}")
}

fn is_supported_month_index(month_index: i32) -> bool {
    (1900 * 12 + 1..=2100 * 12 + 12).contains(&month_index)
}

fn normalize_school(school: &str) -> String {
    normalize_dedupe_value(school)
}

fn normalize_major(major: &str) -> String {
    let value = major.trim().to_lowercase();
    let compact = value.replace([' ', '-', '_'], "");
    match compact.as_str() {
        "computerscience" | "计算机科学" | "计算机科学与技术" => {
            "computer_science".to_string()
        }
        "softwareengineering" | "软件工程" => "software_engineering".to_string(),
        "datascience" | "数据科学" => "data_science".to_string(),
        "informationsystem" | "informationsystems" | "信息系统" => {
            "information_systems".to_string()
        }
        "electronicengineering" | "electronicsengineering" | "电子工程" | "电子信息工程" => {
            "electronic_engineering".to_string()
        }
        "mathematics" | "math" | "数学" => "mathematics".to_string(),
        "statistics" | "statistic" | "统计" | "统计学" => "statistics".to_string(),
        "finance" | "金融" | "金融学" => "finance".to_string(),
        "economics" | "economic" | "经济" | "经济学" => "economics".to_string(),
        "businessadministration" | "工商管理" => "business_administration".to_string(),
        "artificialintelligence" | "人工智能" => "artificial_intelligence".to_string(),
        "computerengineering" | "计算机工程" => "computer_engineering".to_string(),
        "cybersecurity" | "网络安全" | "信息安全" => "cybersecurity".to_string(),
        "networkengineering" | "网络工程" => "network_engineering".to_string(),
        "communicationengineering" | "communicationsengineering" | "通信工程" => {
            "communication_engineering".to_string()
        }
        "mechanicalengineering" | "机械工程" => "mechanical_engineering".to_string(),
        "automation" | "自动化" => "automation".to_string(),
        "accounting" | "accountancy" | "会计" | "会计学" => "accounting".to_string(),
        "marketing" | "市场营销" => "marketing".to_string(),
        "humanresources" | "humanresourcemanagement" | "人力资源管理" => {
            "human_resources".to_string()
        }
        _ => value
            .split(|character: char| !character.is_alphanumeric())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("_"),
    }
}

fn normalize_company(company: &str) -> String {
    let mut value = normalize_dedupe_value(company);
    for suffix in [
        " co., ltd.",
        " co., ltd",
        " co. ltd.",
        " co. ltd",
        " co ltd",
        " company limited",
        " pte. ltd.",
        " pte. ltd",
        " pte ltd",
        " private limited",
        " incorporated",
        " corporation",
        " limited",
        " company",
        " gmbh",
        " s.a.",
        " s.a",
        " sa",
        " inc.",
        " inc",
        " corp.",
        " corp",
        " ltd.",
        " ltd",
        " llc",
        " co.",
        " co",
    ] {
        if value.ends_with(suffix) {
            value.truncate(value.len() - suffix.len());
            value = value.trim().to_string();
            break;
        }
    }
    for suffix in [
        "有限责任公司",
        "股份有限公司",
        "有限合伙",
        "合伙企业",
        "有限公司",
        "公司",
    ] {
        if value.ends_with(suffix) {
            value.truncate(value.len() - suffix.len());
            value = value.trim().to_string();
            break;
        }
    }
    value
}

fn normalize_title(title: &str) -> String {
    let normalized = title
        .trim()
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    match normalized.as_str() {
        "senior_backend_engineer"
        | "staff_backend_engineer"
        | "backend_developer"
        | "backend_engineer"
        | "后端工程师"
        | "高级后端工程师" => "backend_engineer".to_string(),
        "product_manager" | "产品经理" => "product_manager".to_string(),
        "frontend_engineer" | "front_end_engineer" | "staff_frontend_engineer" | "前端工程师" => {
            "frontend_engineer".to_string()
        }
        "data_scientist" | "数据科学家" => "data_scientist".to_string(),
        "devops_engineer" | "dev_ops_engineer" => "devops_engineer".to_string(),
        "engineering_manager" | "工程经理" => "engineering_manager".to_string(),
        _ => normalized,
    }
}

fn normalize_location(location: &str) -> String {
    let primary = location
        .trim()
        .split([',', '，', ';', '；', '|', '/', '\\', '、'])
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let compact = primary.replace([' ', '-', '_'], "");
    match compact.as_str() {
        "shanghai" | "上海" | "上海市" => "shanghai".to_string(),
        "hangzhou" | "杭州" | "杭州市" => "hangzhou".to_string(),
        "shenzhen" | "深圳" | "深圳市" => "shenzhen".to_string(),
        "beijing" | "北京" | "北京市" => "beijing".to_string(),
        "guangzhou" | "广州" | "广州市" => "guangzhou".to_string(),
        "suzhou" | "苏州" | "苏州市" => "suzhou".to_string(),
        "nanjing" | "南京" | "南京市" => "nanjing".to_string(),
        "chengdu" | "成都" | "成都市" => "chengdu".to_string(),
        "wuhan" | "武汉" | "武汉市" => "wuhan".to_string(),
        "chongqing" | "重庆" | "重庆市" => "chongqing".to_string(),
        "tianjin" | "天津" | "天津市" => "tianjin".to_string(),
        "xian" | "西安" | "西安市" => "xian".to_string(),
        "changsha" | "长沙" | "长沙市" => "changsha".to_string(),
        "hefei" | "合肥" | "合肥市" => "hefei".to_string(),
        "qingdao" | "青岛" | "青岛市" => "qingdao".to_string(),
        "hongkong" | "hongkongsar" | "香港" | "香港特别行政区" => "hong_kong".to_string(),
        "taipei" | "台北" | "台北市" => "taipei".to_string(),
        "singapore" | "新加坡" => "singapore".to_string(),
        "sanfrancisco" | "sanfranciscobayarea" | "sfbayarea" | "bayarea" => {
            "san_francisco".to_string()
        }
        "sanjose" => "san_jose".to_string(),
        "newyork" | "newyorkcity" | "nyc" | "纽约" | "纽约市" => "new_york".to_string(),
        "losangeles" | "la" => "los_angeles".to_string(),
        "seattle" => "seattle".to_string(),
        "boston" => "boston".to_string(),
        "austin" => "austin".to_string(),
        "toronto" => "toronto".to_string(),
        "vancouver" => "vancouver".to_string(),
        "tokyo" | "东京" => "tokyo".to_string(),
        "london" | "伦敦" => "london".to_string(),
        "berlin" => "berlin".to_string(),
        "paris" | "巴黎" => "paris".to_string(),
        "remote" | "远程" => "remote".to_string(),
        _ => primary,
    }
}

fn normalize_certificate(certificate: &str) -> String {
    let normalized = certificate
        .trim()
        .to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    match normalized.as_str() {
        "aws_certified_solutions_architect"
        | "aws_solutions_architect_associate"
        | "aws_solutions_architect_professional"
        | "saa_c02"
        | "saa_c03" => "aws_solutions_architect".to_string(),
        "aws_certified_developer" | "aws_developer_associate" | "dva_c01" | "dva_c02" => {
            "aws_developer".to_string()
        }
        "az_104" => "azure_administrator".to_string(),
        "azure_developer" | "azure_developer_associate" | "az_204" => "azure_developer".to_string(),
        "certified_kubernetes_administrator" => "cka".to_string(),
        "certified_kubernetes_application_developer" => "ckad".to_string(),
        "certified_kubernetes_security_specialist" => "cks".to_string(),
        "hashicorp_certified_terraform_associate" | "terraform_associate" => {
            "hashicorp_terraform_associate".to_string()
        }
        "google_associate_cloud_engineer"
        | "google_cloud_associate_cloud_engineer"
        | "gcp_associate_cloud_engineer" => "gcp_associate_cloud_engineer".to_string(),
        "cfa_level_i" | "cfa_level_1" => "cfa_level_1".to_string(),
        "red_hat_certified_system_administrator" => "rhcsa".to_string(),
        "注册会计师" => "cpa".to_string(),
        _ => normalized,
    }
}

fn normalize_dedupe_values<I, S>(values: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    values
        .into_iter()
        .map(|value| normalize_dedupe_value(value.as_ref()))
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalize_dedupe_value(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn intersection_count(left: &BTreeSet<String>, right: &BTreeSet<String>) -> usize {
    left.intersection(right).count()
}
