//! Field filter and candidate dedupe contract tests.

use core_domain::EntityType;
use rank_fusion::{
    group_soft_duplicates, CandidateRecord, DegreeLevel, FieldEvidence, FieldFilters, FieldSummary,
};

#[test]
fn degree_skill_and_year_filters_use_normalized_field_summaries() {
    let evidence = vec![
        FieldEvidence::new(
            EntityType::Other("degree".to_string()),
            "Bachelor of Science",
            Some("bachelor"),
            0.96,
        ),
        FieldEvidence::new(EntityType::Skill, "Java", Some("java"), 0.92),
        FieldEvidence::new(
            EntityType::Skill,
            "Spring Cloud",
            Some("spring cloud"),
            0.90,
        ),
        FieldEvidence::new(EntityType::Date, "2019-01 to 2023-01", None, 0.95),
    ];

    let summary = FieldSummary::from_evidence(&evidence);

    assert_eq!(summary.degree(), Some(DegreeLevel::Bachelor));
    assert_eq!(summary.skills(), ["java", "spring cloud"]);
    assert_eq!(summary.years_experience(), Some(4.0));
    assert!(summary.matches(&FieldFilters {
        degree_min: Some(DegreeLevel::Bachelor),
        skills_any: vec!["JAVA".to_string()],
        years_experience_min: Some(3.5),
    }));
    assert!(!summary.matches(&FieldFilters {
        degree_min: Some(DegreeLevel::Master),
        skills_any: vec!["java".to_string()],
        years_experience_min: Some(3.5),
    }));
}

#[test]
fn open_ended_date_ranges_count_to_deterministic_s10_as_of_date() {
    let evidence = vec![
        FieldEvidence::new(EntityType::Date, "2020-01 to present", None, 0.95),
        FieldEvidence::new(EntityType::Date, "2022年09月至今", None, 0.95),
    ];

    let summary = FieldSummary::from_evidence(&evidence);

    assert_eq!(summary.years_experience(), Some(10.0));
    assert!(summary.matches(&FieldFilters {
        degree_min: None,
        skills_any: Vec::new(),
        years_experience_min: Some(9.0),
    }));
}

#[test]
fn soft_dedupe_groups_by_hashed_contact_keys_without_debug_evidence_leaks() {
    let first = FieldSummary::from_evidence(&[FieldEvidence::new(
        EntityType::Email,
        "team@example.test",
        Some("team@example.test"),
        0.99,
    )]);
    let second = FieldSummary::from_evidence(&[FieldEvidence::new(
        EntityType::Email,
        "TEAM@example.test",
        Some("team@example.test"),
        0.99,
    )]);
    let third = FieldSummary::from_evidence(&[FieldEvidence::new(
        EntityType::Skill,
        "Java",
        Some("java"),
        0.92,
    )]);

    let groups = group_soft_duplicates(vec![
        CandidateRecord::new("doc-a", first),
        CandidateRecord::new("doc-b", second),
        CandidateRecord::new("doc-c", third),
    ]);

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].doc_ids(), ["doc-a", "doc-b"]);
    assert_eq!(groups[0].version_count(), 2);
    let debug = format!("{groups:?}");
    assert!(debug.contains("[redacted dedupe key]"));
    assert!(!debug.contains("team@example.test"));
    assert!(!debug.contains("Java"));
}
