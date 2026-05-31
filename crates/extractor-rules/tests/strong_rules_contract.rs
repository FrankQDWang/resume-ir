//! Strong extractor rule contract tests.

use core_domain::EntityType;
use extractor_rules::extract_strong_entities;

#[test]
fn extracts_email_phone_and_date_ranges_from_mixed_text() {
    let text = "联系方式: team@example.test\n电话: +99 000 0000 0000\n经历: 2020-01 to 2022-03\n教育: 2022年09月-2024年06月";

    let entities = extract_strong_entities(text);

    assert!(entities.iter().any(|entity| {
        entity.entity_type() == EntityType::Email
            && entity.normalized_value() == Some("team@example.test")
    }));
    assert!(entities.iter().any(|entity| {
        entity.entity_type() == EntityType::Phone
            && entity.normalized_value() == Some("+9900000000000")
    }));
    assert_eq!(
        entities
            .iter()
            .filter(|entity| entity.entity_type() == EntityType::Date)
            .count(),
        2
    );
    assert!(entities.iter().all(|entity| entity.confidence() >= 0.90));
}

#[test]
fn extracts_school_degree_and_skills_with_evidence_and_confidence() {
    let text = "Education: Synthetic State University, Bachelor of Science\nSkills: Java, Spring Cloud, MySQL";

    let entities = extract_strong_entities(text);

    assert!(entities.iter().any(|entity| {
        entity.entity_type() == EntityType::School
            && entity.raw_value() == "Synthetic State University"
            && entity.normalized_value() == Some("synthetic state university")
            && entity.confidence() >= 0.85
    }));
    assert!(entities.iter().any(|entity| {
        entity.entity_type() == EntityType::Other("degree".to_string())
            && entity.raw_value() == "Bachelor of Science"
            && entity.normalized_value() == Some("bachelor")
            && entity.confidence() >= 0.90
    }));
    assert!(entities.iter().any(|entity| {
        entity.entity_type() == EntityType::Skill
            && entity.raw_value() == "Spring Cloud"
            && entity.normalized_value() == Some("spring cloud")
            && entity.confidence() >= 0.85
    }));
}

#[test]
fn skill_rules_do_not_split_javascript_into_java() {
    let text = "Skills: JavaScript, TypeScript";

    let entities = extract_strong_entities(text);
    let skills = entities
        .iter()
        .filter(|entity| entity.entity_type() == EntityType::Skill)
        .filter_map(|entity| entity.normalized_value())
        .collect::<Vec<_>>();

    assert_eq!(skills, vec!["javascript", "typescript"]);
}

#[test]
fn extracts_from_table_linearized_text_with_offsets() {
    let text = "Field | Value\nEmail | ops@example.test\nPhone | +99 000 1111 2222\nRange | 2019/06 - present";

    let entities = extract_strong_entities(text);
    let email_start = entities
        .iter()
        .find(|entity| entity.entity_type() == EntityType::Email)
        .map(|entity| entity.span_start() as usize);
    let phone_start = entities
        .iter()
        .find(|entity| entity.entity_type() == EntityType::Phone)
        .map(|entity| entity.span_start() as usize);

    assert_eq!(email_start, text.find("ops@example.test"));
    assert_eq!(phone_start, text.find("+99 000 1111 2222"));
    assert!(entities
        .iter()
        .any(|entity| entity.entity_type() == EntityType::Date));
}

#[test]
fn low_confidence_field_like_text_does_not_enter_strong_filters() {
    let text = "email maybe ops at example dot test\nphone maybe 12345\ndates around spring 2024";

    let entities = extract_strong_entities(text);

    assert!(entities.is_empty());
}

#[test]
fn debug_output_redacts_entity_values() {
    let entities = extract_strong_entities("Email unit@example.test");

    let debug = match entities.first() {
        Some(entity) => format!("{entity:?}"),
        None => String::new(),
    };

    assert!(debug.contains("[redacted entity value]"));
    assert!(!debug.contains("unit@example.test"));
}
