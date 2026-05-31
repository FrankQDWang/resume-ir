use extractor_rules::{extract_strong_fields, FieldType};

#[test]
fn extracts_degree_school_skills_and_experience_with_evidence() {
    let text = "\
Education
Synthetic University
Bachelor of Science in Computer Science
Skills: Java, Spring Cloud, Rust, SQLite
Experience
2020.01 - 2024.03";

    let matches = extract_strong_fields(text);

    let school = matches
        .iter()
        .find(|field| field.field_type == FieldType::School)
        .unwrap();
    assert_eq!(
        school.normalized_value.as_deref(),
        Some("synthetic university")
    );
    assert_eq!(&text[school.span_start..school.span_end], school.raw_value);
    assert!(school.confidence >= 0.8);

    let degree = matches
        .iter()
        .find(|field| field.field_type == FieldType::Degree)
        .unwrap();
    assert_eq!(degree.normalized_value.as_deref(), Some("bachelor"));
    assert!(degree.raw_value.contains("Bachelor"));
    assert!(degree.confidence >= 0.9);

    let skills = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Skill)
        .map(|field| field.normalized_value.as_deref().unwrap())
        .collect::<Vec<_>>();
    assert!(skills.contains(&"Java"));
    assert!(skills.contains(&"Spring Cloud"));
    assert!(skills.contains(&"Rust"));
    assert!(skills.contains(&"SQLite"));

    let years = matches
        .iter()
        .find(|field| field.field_type == FieldType::YearsExperience)
        .unwrap();
    assert_eq!(years.normalized_value.as_deref(), Some("4.2"));
    assert_eq!(&text[years.span_start..years.span_end], years.raw_value);
    assert!(!format!("{years:?}").contains("2020.01"));
}

#[test]
fn avoids_obvious_low_confidence_degree_and_skill_noise() {
    let text = "Mastercard project in Java island research. Timeline: 2020 and Java 8.";
    let matches = extract_strong_fields(text);

    assert!(!matches
        .iter()
        .any(|field| field.field_type == FieldType::Degree));
    assert!(!matches
        .iter()
        .any(|field| field.field_type == FieldType::Skill));
}
