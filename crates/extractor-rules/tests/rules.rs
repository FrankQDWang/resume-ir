use core_domain::EntityType;
use extractor_rules::{ExtractedField, extract_strong_fields};

#[test]
fn extracts_email_phone_and_date_range_with_evidence() {
    let text = "Email: candidate@example.com Phone: 138-0013-8000 2019.09-2023.06 浙江大学";

    let fields = extract_strong_fields(text);

    assert!(fields.iter().any(|field| {
        field.entity_type == EntityType::Email
            && field.raw_value == "candidate@example.com"
            && field.normalized_value.as_deref() == Some("candidate@example.com")
            && field.confidence >= 0.95
            && field.span_start < field.span_end
    }));
    assert!(fields.iter().any(|field| {
        field.entity_type == EntityType::Phone
            && field.normalized_value.as_deref() == Some("13800138000")
            && field.confidence >= 0.95
    }));
    assert!(fields.iter().any(|field| {
        field.entity_type == EntityType::Date
            && field.normalized_value.as_deref() == Some("2019-09..2023-06")
            && field.confidence >= 0.95
    }));
}

#[test]
fn low_confidence_fields_do_not_enter_strong_filter() {
    let weak = ExtractedField {
        entity_type: EntityType::Skill,
        raw_value: "maybe-java".to_owned(),
        normalized_value: Some("java".to_owned()),
        span_start: 0,
        span_end: 10,
        confidence: 0.7,
    };

    assert!(!weak.is_strong_filterable());
}
