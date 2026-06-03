use extractor_rules::{extract_strong_fields, FieldType};

#[test]
fn extracts_candidate_name_from_labeled_line_and_heading_with_evidence() {
    let labeled_text = "\
Name: Synthetic Candidate
Email: candidate@example.test
Experience
Senior Backend Engineer";

    let labeled_matches = extract_strong_fields(labeled_text);
    let labeled_name = labeled_matches
        .iter()
        .find(|field| field.field_type == FieldType::Name)
        .unwrap();
    assert_eq!(labeled_name.raw_value, "Synthetic Candidate");
    assert_eq!(
        labeled_name.normalized_value.as_deref(),
        Some("synthetic candidate")
    );
    assert_eq!(
        &labeled_text[labeled_name.span_start..labeled_name.span_end],
        labeled_name.raw_value
    );
    assert!(labeled_name.confidence >= 0.9);
    assert!(!format!("{labeled_name:?}").contains("Synthetic Candidate"));

    let heading_text = "\
Synthetic Heading Candidate
Senior Backend Engineer
Skills: Rust, Java";
    let heading_matches = extract_strong_fields(heading_text);
    let heading_name = heading_matches
        .iter()
        .find(|field| field.field_type == FieldType::Name)
        .unwrap();
    assert_eq!(heading_name.raw_value, "Synthetic Heading Candidate");
    assert_eq!(
        heading_name.normalized_value.as_deref(),
        Some("synthetic heading candidate")
    );
    assert!(heading_name.confidence >= 0.8);
}

#[test]
fn avoids_section_headers_and_contact_lines_as_candidate_names() {
    let text = "\
Education
Synthetic University
Email: candidate@example.test
Skills: Rust, Java";

    let matches = extract_strong_fields(text);

    assert!(!matches
        .iter()
        .any(|field| field.field_type == FieldType::Name));
}

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

#[test]
fn extracts_company_title_and_certificate_with_evidence() {
    let text = "\
Experience
Synthetic Payments Inc.
Senior Backend Engineer
Certificate
AWS Certified Solutions Architect
2021.05 - 2024.05";

    let matches = extract_strong_fields(text);

    let company = matches
        .iter()
        .find(|field| field.field_type == FieldType::Company)
        .unwrap();
    assert_eq!(
        company.normalized_value.as_deref(),
        Some("synthetic payments")
    );
    assert_eq!(
        &text[company.span_start..company.span_end],
        company.raw_value
    );
    assert!(company.confidence >= 0.75);

    let title = matches
        .iter()
        .find(|field| field.field_type == FieldType::Title)
        .unwrap();
    assert_eq!(title.normalized_value.as_deref(), Some("backend_engineer"));
    assert_eq!(&text[title.span_start..title.span_end], title.raw_value);
    assert!(title.confidence >= 0.75);

    let certificate = matches
        .iter()
        .find(|field| field.field_type == FieldType::Certificate)
        .unwrap();
    assert_eq!(
        certificate.normalized_value.as_deref(),
        Some("aws certified solutions architect")
    );
    assert_eq!(
        &text[certificate.span_start..certificate.span_end],
        certificate.raw_value
    );
    assert!(certificate.confidence >= 0.8);
    assert!(!format!("{certificate:?}").contains("AWS Certified"));
}
