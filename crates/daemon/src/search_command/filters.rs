use meta_store::{
    ContactHash, EntityType, SearchFilterCase, SearchProjectionFilter, SearchProjectionPredicate,
};
use rank_fusion::{DateRange, DegreeLevel, SchoolTier, SearchFilters};

use crate::command_failure::CommandFailure;

pub(super) fn parse_search_filters(
    filters: Option<&serde_json::Value>,
) -> Result<SearchProjectionFilter, CommandFailure> {
    let Some(filters) = filters else {
        return Ok(SearchProjectionFilter::default());
    };
    if filters.is_null() {
        return Ok(SearchProjectionFilter::default());
    }
    let Some(object) = filters.as_object() else {
        return Err(CommandFailure::BadRequest("filters must be an object"));
    };

    const ALLOWED_FIELDS: &[&str] = &[
        "degree_min",
        "skills_any",
        "contact_hashes_any",
        "school_tiers_any",
        "names_any",
        "schools_any",
        "majors_any",
        "certificates_any",
        "date_range_overlaps",
        "companies_any",
        "titles_any",
        "locations_any",
        "years_experience_min",
    ];
    if object
        .keys()
        .any(|field| !ALLOWED_FIELDS.contains(&field.as_str()))
    {
        return Err(CommandFailure::BadRequest(
            "filters contain an unknown field",
        ));
    }

    let mut parsed = SearchFilters::default();
    if let Some(value) = object.get("degree_min") {
        if !value.is_null() {
            let degree = value
                .as_str()
                .and_then(DegreeLevel::parse)
                .ok_or(CommandFailure::BadRequest("degree_min is invalid"))?;
            parsed = parsed.with_degree_min(degree);
        }
    }
    if let Some(value) = object.get("skills_any") {
        if !value.is_null() {
            let skills = value
                .as_array()
                .ok_or(CommandFailure::BadRequest("skills_any must be an array"))?;
            if skills.len() > 64 {
                return Err(CommandFailure::BadRequest("too many skills"));
            }
            let skills = skills
                .iter()
                .map(|skill| {
                    skill
                        .as_str()
                        .filter(|skill| !skill.trim().is_empty())
                        .ok_or(CommandFailure::BadRequest("skills_any must be strings"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            parsed = parsed.with_skills_any(skills);
        }
    }
    if let Some(value) = object.get("contact_hashes_any") {
        if !value.is_null() {
            let contact_hashes = value.as_array().ok_or(CommandFailure::BadRequest(
                "contact_hashes_any must be an array",
            ))?;
            if contact_hashes.len() > 64 {
                return Err(CommandFailure::BadRequest("too many contact hashes"));
            }
            let contact_hashes = contact_hashes
                .iter()
                .map(|contact_hash| {
                    let contact_hash = contact_hash.as_str().ok_or(CommandFailure::BadRequest(
                        "contact_hashes_any values must be strings",
                    ))?;
                    ContactHash::from_keyed_digest(contact_hash.to_string())
                        .map(|hash| hash.as_str().to_string())
                        .map_err(|_| {
                            CommandFailure::BadRequest(
                                "contact_hashes_any values must be contact hashes",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            parsed = parsed.with_contact_hashes_any(contact_hashes);
        }
    }
    if let Some(value) = object.get("school_tiers_any") {
        if !value.is_null() {
            let tiers = value.as_array().ok_or(CommandFailure::BadRequest(
                "school_tiers_any must be an array",
            ))?;
            if tiers.len() > 16 {
                return Err(CommandFailure::BadRequest("too many school tiers"));
            }
            let tiers = tiers
                .iter()
                .map(|tier| {
                    tier.as_str()
                        .and_then(SchoolTier::parse)
                        .ok_or(CommandFailure::BadRequest(
                            "school_tiers_any values are invalid",
                        ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            parsed = parsed.with_school_tiers_any(tiers);
        }
    }
    if let Some(value) = object.get("names_any") {
        if !value.is_null() {
            let names = string_array(
                value,
                "names_any must be an array",
                "too many names",
                64,
                "names_any values must be strings",
            )?;
            parsed = parsed.with_names_any(names);
        }
    }
    if let Some(value) = object.get("schools_any") {
        if !value.is_null() {
            let schools = string_array(
                value,
                "schools_any must be an array",
                "too many schools",
                64,
                "schools_any values must be strings",
            )?;
            parsed = parsed.with_schools_any(schools);
        }
    }
    if let Some(value) = object.get("majors_any") {
        if !value.is_null() {
            let majors = string_array(
                value,
                "majors_any must be an array",
                "too many majors",
                64,
                "majors_any values must be strings",
            )?;
            parsed = parsed.with_majors_any(majors);
        }
    }
    if let Some(value) = object.get("certificates_any") {
        if !value.is_null() {
            let certificates = string_array(
                value,
                "certificates_any must be an array",
                "too many certificates",
                32,
                "certificates_any values must be strings",
            )?;
            parsed = parsed.with_certificates_any(certificates);
        }
    }
    if let Some(value) = object.get("date_range_overlaps") {
        if !value.is_null() {
            let range = value
                .as_str()
                .and_then(DateRange::parse)
                .ok_or(CommandFailure::BadRequest("date_range_overlaps is invalid"))?;
            parsed = parsed.with_date_range_overlaps(&range.canonical());
        }
    }
    if let Some(value) = object.get("companies_any") {
        if !value.is_null() {
            let companies = string_array(
                value,
                "companies_any must be an array",
                "too many companies",
                64,
                "companies_any values must be strings",
            )?;
            parsed = parsed.with_companies_any(companies);
        }
    }
    if let Some(value) = object.get("titles_any") {
        if !value.is_null() {
            let titles = string_array(
                value,
                "titles_any must be an array",
                "too many titles",
                64,
                "titles_any values must be strings",
            )?;
            parsed = parsed.with_titles_any(titles);
        }
    }
    if let Some(value) = object.get("locations_any") {
        if !value.is_null() {
            let locations = string_array(
                value,
                "locations_any must be an array",
                "too many locations",
                64,
                "locations_any values must be strings",
            )?;
            parsed = parsed.with_locations_any(locations);
        }
    }
    if let Some(value) = object.get("years_experience_min") {
        if !value.is_null() {
            let years = value
                .as_f64()
                .filter(|years| years.is_finite() && *years >= 0.0)
                .ok_or(CommandFailure::BadRequest(
                    "years_experience_min is invalid",
                ))? as f32;
            if !years.is_finite() {
                return Err(CommandFailure::BadRequest(
                    "years_experience_min is invalid",
                ));
            }
            parsed = parsed.with_years_experience_min(years);
        }
    }
    search_projection_filter(&parsed)
}

fn string_array<'a>(
    value: &'a serde_json::Value,
    array_error: &'static str,
    count_error: &'static str,
    limit: usize,
    value_error: &'static str,
) -> Result<Vec<&'a str>, CommandFailure> {
    let values = value
        .as_array()
        .ok_or(CommandFailure::BadRequest(array_error))?;
    if values.len() > limit {
        return Err(CommandFailure::BadRequest(count_error));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or(CommandFailure::BadRequest(value_error))
        })
        .collect()
}

fn search_projection_filter(
    filters: &SearchFilters,
) -> Result<SearchProjectionFilter, CommandFailure> {
    let mut predicates = Vec::new();
    if let Some(degree) = filters.degree_min() {
        predicates.push(SearchProjectionPredicate::EntityValuesAny {
            entity_type: EntityType::Degree,
            normalized_values: degree_filter_values(degree),
            min_confidence: crate::FIELD_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::Exact,
        });
    }
    push_text_filter(&mut predicates, EntityType::Name, filters.names_any());
    push_school_tier_filter(&mut predicates, filters.school_tiers_any());
    push_text_filter(&mut predicates, EntityType::School, filters.schools_any());
    push_text_filter(&mut predicates, EntityType::Major, filters.majors_any());
    push_text_filter(
        &mut predicates,
        EntityType::Certificate,
        filters.certificates_any(),
    );
    if let Some(range) = filters.date_range_overlaps() {
        predicates.push(SearchProjectionPredicate::DateRangeOverlap {
            start_month: range.start_month(),
            end_month: range.end_month(),
            min_confidence: crate::FIELD_CONFIDENCE_THRESHOLD,
        });
    }
    push_text_filter(
        &mut predicates,
        EntityType::Company,
        filters.companies_any(),
    );
    push_text_filter(&mut predicates, EntityType::Title, filters.titles_any());
    push_text_filter(
        &mut predicates,
        EntityType::Location,
        filters.locations_any(),
    );
    push_text_filter(&mut predicates, EntityType::Skill, filters.skills_any());
    if !filters.contact_hashes_any().is_empty() {
        let hashes = filters
            .contact_hashes_any()
            .iter()
            .map(|value| {
                ContactHash::from_keyed_digest(value.clone()).map_err(|_| {
                    CommandFailure::BadRequest("contact_hashes_any values must be contact hashes")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        predicates.push(SearchProjectionPredicate::ContactHashesAny(hashes));
    }
    if let Some(minimum) = filters.years_experience_min() {
        predicates.push(SearchProjectionPredicate::NumericEntityMinimum {
            entity_type: EntityType::YearsExperience,
            minimum,
            min_confidence: crate::FIELD_CONFIDENCE_THRESHOLD,
        });
    }
    SearchProjectionFilter::new(predicates)
        .map_err(|_| CommandFailure::BadRequest("filters are invalid"))
}

fn push_text_filter(
    predicates: &mut Vec<SearchProjectionPredicate>,
    entity_type: EntityType,
    values: &[String],
) {
    if values.is_empty() {
        return;
    }
    predicates.push(SearchProjectionPredicate::EntityValuesAny {
        entity_type,
        normalized_values: values.to_vec(),
        min_confidence: crate::FIELD_CONFIDENCE_THRESHOLD,
        case: SearchFilterCase::AsciiInsensitive,
    });
}

fn push_school_tier_filter(predicates: &mut Vec<SearchProjectionPredicate>, tiers: &[SchoolTier]) {
    if tiers.is_empty() {
        return;
    }
    let include_missing = tiers.contains(&SchoolTier::Unknown);
    let values = tiers
        .iter()
        .filter(|tier| **tier != SchoolTier::Unknown)
        .map(|tier| tier.canonical().to_string())
        .collect::<Vec<_>>();
    let predicate = match (values.is_empty(), include_missing) {
        (true, true) => SearchProjectionPredicate::MissingEntityType {
            entity_type: EntityType::SchoolTier,
            min_confidence: crate::FIELD_CONFIDENCE_THRESHOLD,
        },
        (false, true) => SearchProjectionPredicate::EntityValuesAnyOrMissing {
            entity_type: EntityType::SchoolTier,
            normalized_values: values,
            min_confidence: crate::FIELD_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::Exact,
        },
        (false, false) => SearchProjectionPredicate::EntityValuesAny {
            entity_type: EntityType::SchoolTier,
            normalized_values: values,
            min_confidence: crate::FIELD_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::Exact,
        },
        (true, false) => return,
    };
    predicates.push(predicate);
}

fn degree_filter_values(minimum: DegreeLevel) -> Vec<String> {
    [
        DegreeLevel::HighSchool,
        DegreeLevel::Associate,
        DegreeLevel::Bachelor,
        DegreeLevel::Master,
        DegreeLevel::Doctor,
    ]
    .into_iter()
    .filter(|degree| *degree >= minimum)
    .map(|degree| degree.canonical().to_string())
    .collect()
}
