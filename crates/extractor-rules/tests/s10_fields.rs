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
fn extracts_labeled_school_and_degree_values_with_alias_normalization() {
    let text = "\
Education
School: Synthetic Institute of Technology
Degree: MSc Computer Science
教育经历
学校：合成大学
学历：博士研究生";

    let matches = extract_strong_fields(text);

    let schools = matches
        .iter()
        .filter(|field| field.field_type == FieldType::School)
        .collect::<Vec<_>>();
    let school_normalized = schools
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        school_normalized,
        vec!["synthetic institute of technology", "合成大学"]
    );
    assert_eq!(
        &text[schools[0].span_start..schools[0].span_end],
        "Synthetic Institute of Technology"
    );
    assert_eq!(
        &text[schools[1].span_start..schools[1].span_end],
        "合成大学"
    );
    assert!(schools.iter().all(|field| !field.raw_value.contains(':')));
    assert!(schools.iter().all(|field| !field.raw_value.contains('：')));
    assert!(!format!("{:?}", schools[0]).contains("Synthetic Institute"));

    let degrees = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Degree)
        .collect::<Vec<_>>();
    let degree_normalized = degrees
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(degree_normalized, vec!["master", "doctor"]);
    assert_eq!(&text[degrees[0].span_start..degrees[0].span_end], "MSc");
    assert_eq!(
        &text[degrees[1].span_start..degrees[1].span_end],
        "博士研究生"
    );
    assert!(degrees.iter().all(|field| !field.raw_value.contains(':')));
    assert!(degrees.iter().all(|field| !field.raw_value.contains('：')));
    assert!(!format!("{:?}", degrees[1]).contains("博士"));
}

#[test]
fn extracts_labeled_major_values_with_alias_normalization() {
    let text = "\
Education
Major: Computer Science
Field of Study: Software Engineering
教育经历
专业：数据科学
";

    let matches = extract_strong_fields(text);

    let majors = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Major)
        .collect::<Vec<_>>();
    let normalized = majors
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(
        normalized,
        vec!["computer_science", "software_engineering", "data_science"]
    );
    assert_eq!(
        &text[majors[0].span_start..majors[0].span_end],
        "Computer Science"
    );
    assert_eq!(
        &text[majors[1].span_start..majors[1].span_end],
        "Software Engineering"
    );
    assert_eq!(&text[majors[2].span_start..majors[2].span_end], "数据科学");
    assert!(majors.iter().all(|field| field.confidence >= 0.86));
    assert!(majors.iter().all(|field| !field.raw_value.contains(':')));
    assert!(majors.iter().all(|field| !field.raw_value.contains('：')));
    assert!(!format!("{:?}", majors[0]).contains("Computer Science"));
}

#[test]
fn extracts_broader_major_aliases_inside_education_context() {
    let text = "\
Education
Artificial Intelligence
Computer Engineering
Cybersecurity
教育经历
网络工程
通信工程
机械工程
自动化
会计学
市场营销
人力资源管理
Skills
Built artificial intelligence dashboards";

    let matches = extract_strong_fields(text);

    let majors = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Major)
        .collect::<Vec<_>>();
    let normalized = majors
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(
        normalized,
        vec![
            "artificial_intelligence",
            "computer_engineering",
            "cybersecurity",
            "network_engineering",
            "communication_engineering",
            "mechanical_engineering",
            "automation",
            "accounting",
            "marketing",
            "human_resources"
        ]
    );
    assert_eq!(
        &text[majors[0].span_start..majors[0].span_end],
        "Artificial Intelligence"
    );
    assert_eq!(&text[majors[3].span_start..majors[3].span_end], "网络工程");
    assert!(majors.iter().all(|field| field.confidence >= 0.86));
    assert!(!majors
        .iter()
        .any(|field| field.span_start > text.find("Skills").unwrap()));
    assert!(!format!("{:?}", majors[0]).contains("Artificial Intelligence"));
}

#[test]
fn extracts_school_tier_values_inside_education_context() {
    let text = "\
Education
School: Synthetic 985 University (985/211/双一流)
Degree: Bachelor of Engineering
教育经历
学校层次：海外高校
Skills
Built 985 telemetry dashboards";

    let matches = extract_strong_fields(text);

    let school_tiers = matches
        .iter()
        .filter(|field| field.field_type == FieldType::SchoolTier)
        .collect::<Vec<_>>();
    let normalized = school_tiers
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(
        normalized,
        vec!["985", "211", "double_first_class", "overseas"]
    );
    assert_eq!(
        &text[school_tiers[0].span_start..school_tiers[0].span_end],
        "985"
    );
    assert_eq!(
        &text[school_tiers[2].span_start..school_tiers[2].span_end],
        "双一流"
    );
    assert!(school_tiers.iter().all(|field| field.confidence >= 0.82));
    assert!(!school_tiers
        .iter()
        .any(|field| field.span_start > text.find("Skills").unwrap()));
    assert!(!format!("{:?}", school_tiers[0]).contains("985"));
}

#[test]
fn avoids_degree_aliases_outside_education_context() {
    let text = "\
Skills
MS SQL, Java
Experience
Built reporting systems
Education
Synthetic University
Bachelor of Science in Computer Science";

    let matches = extract_strong_fields(text);

    let degrees = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Degree)
        .collect::<Vec<_>>();
    let degree_normalized = degrees
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(degree_normalized, vec!["bachelor"]);
    assert!(degrees[0].raw_value.contains("Bachelor of Science"));
    assert_eq!(
        &text[degrees[0].span_start..degrees[0].span_end],
        degrees[0].raw_value
    );
    assert!(!degrees.iter().any(|field| field.raw_value == "MS"));

    let skills = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Skill)
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert!(skills.contains(&"SQL"));
    assert!(skills.contains(&"Java"));
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
fn extracts_labeled_company_and_title_values_with_exact_spans() {
    let text = "\
Experience
Company: Synthetic Commerce Inc.
Title: Product Manager
工作经历
公司：合成科技有限公司
职位：高级后端工程师";

    let matches = extract_strong_fields(text);
    let companies = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Company)
        .collect::<Vec<_>>();
    let company_normalized = companies
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(company_normalized, vec!["synthetic commerce", "合成科技"]);
    assert_eq!(
        &text[companies[0].span_start..companies[0].span_end],
        "Synthetic Commerce Inc."
    );
    assert_eq!(
        &text[companies[1].span_start..companies[1].span_end],
        "合成科技有限公司"
    );
    assert!(companies
        .iter()
        .all(|field| !field.raw_value.contains('：')));
    assert!(companies.iter().all(|field| !field.raw_value.contains(':')));

    let titles = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Title)
        .collect::<Vec<_>>();
    let title_normalized = titles
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        title_normalized,
        vec!["product_manager", "backend_engineer"]
    );
    assert_eq!(
        &text[titles[0].span_start..titles[0].span_end],
        "Product Manager"
    );
    assert_eq!(
        &text[titles[1].span_start..titles[1].span_end],
        "高级后端工程师"
    );
    assert!(titles.iter().all(|field| !field.raw_value.contains('：')));
    assert!(titles.iter().all(|field| !field.raw_value.contains(':')));
    assert!(!format!("{:?}", companies[0]).contains("Synthetic Commerce"));
    assert!(!format!("{:?}", titles[1]).contains("高级后端"));
}

#[test]
fn extracts_labeled_location_values_with_exact_spans() {
    let text = "\
Candidate Location Target
Location: Shanghai, China
所在地：杭州
Base: Shenzhen";

    let matches = extract_strong_fields(text);
    let locations = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Location)
        .collect::<Vec<_>>();
    let normalized = locations
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(normalized, vec!["shanghai", "hangzhou", "shenzhen"]);
    assert_eq!(
        &text[locations[0].span_start..locations[0].span_end],
        "Shanghai, China"
    );
    assert_eq!(
        &text[locations[1].span_start..locations[1].span_end],
        "杭州"
    );
    assert_eq!(
        &text[locations[2].span_start..locations[2].span_end],
        "Shenzhen"
    );
    assert!(locations.iter().all(|field| field.confidence >= 0.82));
    assert!(locations.iter().all(|field| !field.raw_value.contains(':')));
    assert!(locations
        .iter()
        .all(|field| !field.raw_value.contains('：')));
    assert!(!format!("{:?}", locations[0]).contains("Shanghai"));
}

#[test]
fn extracts_broader_labeled_location_aliases_with_exact_spans() {
    let text = "\
Candidate Location Alias Target
Current Location: San Francisco Bay Area
Preferred City: New York City
工作地点：香港
Base City: Singapore
地点：重庆市
Experience
Supported Bay Area customers without declaring a location label";

    let matches = extract_strong_fields(text);
    let locations = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Location)
        .collect::<Vec<_>>();
    let normalized = locations
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(
        normalized,
        vec![
            "san_francisco",
            "new_york",
            "hong_kong",
            "singapore",
            "chongqing"
        ]
    );
    assert_eq!(
        &text[locations[0].span_start..locations[0].span_end],
        "San Francisco Bay Area"
    );
    assert_eq!(
        &text[locations[1].span_start..locations[1].span_end],
        "New York City"
    );
    assert_eq!(
        &text[locations[2].span_start..locations[2].span_end],
        "香港"
    );
    assert_eq!(
        &text[locations[3].span_start..locations[3].span_end],
        "Singapore"
    );
    assert_eq!(
        &text[locations[4].span_start..locations[4].span_end],
        "重庆市"
    );
    assert!(locations.iter().all(|field| field.confidence >= 0.82));
    assert!(!format!("{:?}", locations[0]).contains("San Francisco"));
    assert!(!locations
        .iter()
        .any(|field| field.raw_value.contains("customers")));
}

#[test]
fn extracts_city_evidence_from_labeled_address_values_without_street_spans() {
    let text = "\
Candidate Address Location Target
Address: 123 Market St, San Francisco, CA 94105
地址：北京市海淀区中关村大街1号
Current Address: 88 Queen's Road, Hong Kong
Experience
Delivered projects at 123 Market St without a location label";

    let matches = extract_strong_fields(text);
    let locations = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Location)
        .collect::<Vec<_>>();
    let normalized = locations
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(normalized, vec!["san_francisco", "beijing", "hong_kong"]);
    assert_eq!(
        &text[locations[0].span_start..locations[0].span_end],
        "San Francisco"
    );
    assert_eq!(
        &text[locations[1].span_start..locations[1].span_end],
        "北京市"
    );
    assert_eq!(
        &text[locations[2].span_start..locations[2].span_end],
        "Hong Kong"
    );
    assert!(locations
        .iter()
        .all(|field| !field.raw_value.contains("123")));
    assert!(locations
        .iter()
        .all(|field| !field.raw_value.contains("Queen")));
    assert!(!format!("{:?}", locations[0]).contains("Market St"));
    assert!(!locations
        .iter()
        .any(|field| field.raw_value.contains("projects")));
}

#[test]
fn extracts_broader_title_aliases_without_certificate_title_noise() {
    let text = "\
Experience
Staff Frontend Engineer
全栈开发工程师
Machine Learning Engineer
数据科学家
DevOps Engineer
QA Engineer
Engineering Manager
Solutions Architect
Certificate
AWS Certified Solutions Architect";

    let matches = extract_strong_fields(text);
    let titles = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Title)
        .collect::<Vec<_>>();
    let normalized = titles
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();

    assert_eq!(
        normalized,
        vec![
            "frontend_engineer",
            "fullstack_engineer",
            "machine_learning_engineer",
            "data_scientist",
            "devops_engineer",
            "qa_engineer",
            "engineering_manager",
            "solutions_architect"
        ]
    );
    assert!(titles
        .iter()
        .all(|field| text[field.span_start..field.span_end] == field.raw_value));
    assert!(!titles
        .iter()
        .any(|field| field.raw_value.contains("AWS Certified")));
    assert!(!format!("{:?}", titles[0]).contains("Staff Frontend"));
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

#[test]
fn extracts_expanded_production_skill_certificate_and_title_aliases() {
    let text = "\
Technical Skills
Apache Spark / Hadoop / Airflow
TensorFlow, PyTorch, scikit-learn
Vue.js, Angular, GraphQL
Certifications
AWS Certified Security - Specialty
Google Professional Data Engineer
CCNA
Experience
Platform Engineer
信息安全工程师
Mobile Engineer
Business Analyst";

    let matches = extract_strong_fields(text);

    let skills = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Skill)
        .collect::<Vec<_>>();
    let skill_normalized = skills
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        skill_normalized,
        vec![
            "Spark",
            "Hadoop",
            "Airflow",
            "TensorFlow",
            "PyTorch",
            "scikit-learn",
            "Vue.js",
            "Angular",
            "GraphQL"
        ]
    );
    assert!(skills
        .iter()
        .all(|field| text[field.span_start..field.span_end] == field.raw_value));
    assert!(!skills
        .iter()
        .any(|field| field.raw_value == "Technical Skills"));
    assert!(!format!("{:?}", skills[0]).contains("Apache Spark"));

    let certificates = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Certificate)
        .collect::<Vec<_>>();
    let certificate_normalized = certificates
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        certificate_normalized,
        vec![
            "aws_security_specialty",
            "gcp_professional_data_engineer",
            "ccna"
        ]
    );
    assert!(certificates
        .iter()
        .all(|field| text[field.span_start..field.span_end] == field.raw_value));
    assert!(!format!("{:?}", certificates[0]).contains("AWS Certified"));

    let titles = matches
        .iter()
        .filter(|field| field.field_type == FieldType::Title)
        .collect::<Vec<_>>();
    let title_normalized = titles
        .iter()
        .filter_map(|field| field.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        title_normalized,
        vec![
            "platform_engineer",
            "security_engineer",
            "mobile_engineer",
            "business_analyst"
        ]
    );
    assert!(titles
        .iter()
        .all(|field| text[field.span_start..field.span_end] == field.raw_value));
    assert!(!titles
        .iter()
        .any(|field| field.raw_value.contains("AWS Certified")));
    assert!(!format!("{:?}", titles[0]).contains("Platform Engineer"));
}
