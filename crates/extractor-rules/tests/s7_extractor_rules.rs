#[test]
fn exposes_extractor_rules_crate_identity() {
    assert_eq!(extractor_rules::crate_name(), "extractor-rules");
}

#[test]
fn extracts_email_phone_and_date_ranges_with_offsets_from_mixed_text() {
    use extractor_rules::{extract_strong_fields, FieldType};

    let text = "联系: Synthetic.Candidate@Example.Test\n电话: +86 138-0013-8000\nExperience: 2020.01 - 2022.03\nProject: Jan 2023 - Mar 2024";
    let matches = extract_strong_fields(text);

    let email = matches
        .iter()
        .find(|field| field.field_type == FieldType::Email)
        .unwrap();
    assert_eq!(
        email.normalized_value.as_deref(),
        Some("synthetic.candidate@example.test")
    );
    assert_eq!(&text[email.span_start..email.span_end], email.raw_value);
    assert!(!format!("{email:?}").contains("Synthetic.Candidate"));
    assert!(!format!("{email:?}").contains("synthetic.candidate@example.test"));

    let phone = matches
        .iter()
        .find(|field| field.field_type == FieldType::Phone)
        .unwrap();
    assert_eq!(phone.normalized_value.as_deref(), Some("+8613800138000"));

    let date_ranges = matches
        .iter()
        .filter(|field| field.field_type == FieldType::DateRange)
        .collect::<Vec<_>>();
    assert_eq!(date_ranges.len(), 2);
    assert!(date_ranges
        .iter()
        .any(|field| field.normalized_value.as_deref() == Some("2020-01/2022-03")));
    assert!(date_ranges.iter().all(|field| field.confidence >= 0.9));
}

#[test]
fn does_not_emit_low_confidence_field_candidates() {
    use extractor_rules::extract_strong_fields;

    let text = "Email: synthetic(at)example dot test\nPhone: 12345\nTimeline: 2020 and Java 8";
    let matches = extract_strong_fields(text);

    assert!(matches.is_empty());
}

#[test]
fn table_linearized_text_keeps_rule_offsets() {
    use extractor_rules::{extract_strong_fields, FieldType};

    let text = "Field | Value\nEmail | research@example.test\nPhone | (415) 555-0132";
    let matches = extract_strong_fields(text);
    let phone = matches
        .iter()
        .find(|field| field.field_type == FieldType::Phone)
        .unwrap();

    assert_eq!(&text[phone.span_start..phone.span_end], "(415) 555-0132");
    assert_eq!(phone.normalized_value.as_deref(), Some("+14155550132"));
}
