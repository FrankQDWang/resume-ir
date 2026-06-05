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
fn extracts_chinese_year_month_date_ranges_with_years_evidence() {
    let text = "\
Experience
2020年1月 - 2024年3月
Synthetic Payments Inc.";

    let matches = extract_strong_fields(text);

    let date_range = matches
        .iter()
        .find(|field| field.field_type == FieldType::DateRange)
        .unwrap();
    assert_eq!(
        date_range.normalized_value.as_deref(),
        Some("2020-01/2024-03")
    );
    assert_eq!(
        &text[date_range.span_start..date_range.span_end],
        "2020年1月 - 2024年3月"
    );
    assert!(date_range.confidence >= 0.9);
    assert!(!format!("{date_range:?}").contains("2020年1月"));

    let years = matches
        .iter()
        .find(|field| field.field_type == FieldType::YearsExperience)
        .unwrap();
    assert_eq!(years.normalized_value.as_deref(), Some("4.2"));
    assert_eq!(
        &text[years.span_start..years.span_end],
        date_range.raw_value
    );
}

#[test]
fn extracts_open_ended_present_date_ranges_with_years_evidence() {
    let text = "\
Experience
2020年1月 - 至今
Project
Jan 2021 - Present
Contract
2022.03 - Current";

    let matches = extract_strong_fields(text);
    let date_ranges = matches
        .iter()
        .filter(|field| field.field_type == FieldType::DateRange)
        .collect::<Vec<_>>();
    let normalized = date_ranges
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(
        normalized,
        vec!["2020-01/PRESENT", "2021-01/PRESENT", "2022-03/PRESENT"]
    );
    assert_eq!(
        &text[date_ranges[0].span_start..date_ranges[0].span_end],
        "2020年1月 - 至今"
    );
    assert_eq!(
        &text[date_ranges[1].span_start..date_ranges[1].span_end],
        "Jan 2021 - Present"
    );
    assert_eq!(
        &text[date_ranges[2].span_start..date_ranges[2].span_end],
        "2022.03 - Current"
    );
    assert!(date_ranges.iter().all(|field| field.confidence >= 0.9));
    assert!(!format!("{:?}", date_ranges[0]).contains("至今"));

    let years = matches
        .iter()
        .find(|field| field.field_type == FieldType::YearsExperience)
        .unwrap();
    let years_value = years.normalized_value.as_deref().unwrap();
    let years_value = years_value.parse::<f32>().unwrap();
    assert!(years_value >= 10.0, "{years_value}");
    assert!(!format!("{years:?}").contains("Present"));
}

#[test]
fn extracts_sectioned_skill_aliases_without_header_or_context_noise() {
    let text = "\
Skills
Python / TypeScript / PostgreSQL
技术栈
K8s, Golang, Redis
Experience
Java island migration";

    let matches = extract_strong_fields(text);
    let skills = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Skill)
        .collect::<Vec<_>>();

    let normalized = skills
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        normalized,
        vec![
            "Python",
            "TypeScript",
            "PostgreSQL",
            "Kubernetes",
            "Go",
            "Redis"
        ]
    );
    assert!(skills
        .iter()
        .all(|field| text[field.span_start..field.span_end] == field.raw_value));
    assert!(!skills
        .iter()
        .any(|field| field.raw_value == "Skills" || field.raw_value == "技术栈"));
    assert!(!skills.iter().any(|field| field.raw_value == "Java"));
    assert!(!format!("{:?}", skills[0]).contains("Python"));
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
        Some("aws_solutions_architect")
    );
    assert_eq!(
        &text[certificate.span_start..certificate.span_end],
        certificate.raw_value
    );
    assert!(certificate.confidence >= 0.8);
    assert!(!format!("{certificate:?}").contains("AWS Certified"));
}

#[test]
fn extracts_sectioned_certificate_aliases_without_header_noise() {
    let text = "\
Certifications
PMP, CKA, CISSP
认证
CFA Level I
Experience
Senior Backend Engineer";

    let matches = extract_strong_fields(text);
    let certificates = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Certificate)
        .collect::<Vec<_>>();

    let normalized = certificates
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(normalized, vec!["pmp", "cka", "cissp", "cfa_level_1"]);
    assert!(certificates
        .iter()
        .all(|field| text[field.span_start..field.span_end] == field.raw_value));
    assert!(certificates.iter().all(|field| field.confidence >= 0.84));
    assert!(!certificates
        .iter()
        .any(|field| field.raw_value == "Certifications" || field.raw_value == "认证"));
    assert!(!format!("{:?}", certificates[0]).contains("PMP"));
}

#[test]
fn extracts_fullwidth_labeled_certificate_alias_with_exact_span() {
    let text = "认证：PMP";

    let matches = extract_strong_fields(text);
    let certificate = matches
        .iter()
        .find(|field| field.field_type == FieldType::Certificate)
        .unwrap();

    assert_eq!(certificate.raw_value, "PMP");
    assert_eq!(certificate.normalized_value.as_deref(), Some("pmp"));
    assert_eq!(&text[certificate.span_start..certificate.span_end], "PMP");
    assert!(!format!("{certificate:?}").contains("PMP"));
}
