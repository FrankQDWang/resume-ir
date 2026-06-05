use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{EntityType, MetaStore};

#[test]
fn filtered_search_uses_persisted_field_mentions_without_reextracting_clean_text() {
    let data_dir = temp_dir("persisted-fields-data");
    import_fixtures(&data_dir);

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let versions = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .filter(|document| document.file_name != "synthetic-scanned-resume.pdf")
        .flat_map(|document| store.resume_versions_for_document(&document.id).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(versions.len(), 2);
    for version in &versions {
        let mentions = store.entity_mentions_for_version(&version.id).unwrap();
        assert!(mentions
            .iter()
            .any(|mention| mention.entity_type == EntityType::Degree));
        assert!(mentions
            .iter()
            .any(|mention| mention.entity_type == EntityType::Skill));
        assert!(mentions
            .iter()
            .any(|mention| { mention.entity_type == EntityType::YearsExperience }));
    }

    for mut version in versions {
        version.raw_text = None;
        version.clean_text = None;
        store.upsert_resume_version(&version).unwrap();
    }

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--degree",
            "bachelor",
            "--skills-any",
            "java",
            "--years-experience-min",
            "4",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run persisted-field filtered search");

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("results: 2"));
    assert!(stdout.contains("synthetic-java-engineer.docx"));
    assert!(stdout.contains("synthetic-java-platform.pdf"));

    remove_dir(&data_dir);
}

#[test]
fn import_persists_sectioned_certificate_alias_mentions_without_output_leaks() {
    let data_dir = temp_dir("persisted-certificate-alias-data");
    let resume_root = temp_dir("persisted-certificate-alias-resumes");
    fs::write(
        resume_root.join("synthetic-cert-candidate.txt"),
        "\
Synthetic Cert Candidate
Email: cert-candidate@example.test
Certifications
PMP, CKA, CISSP
认证
CFA Level I
Skills: Java
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import certificate aliases");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("PMP"));
    assert!(!stdout.contains("cert-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-cert-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mut normalized = store
        .entity_mentions_for_version(&version.id)
        .unwrap()
        .into_iter()
        .filter(|mention| mention.entity_type == EntityType::Certificate)
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert!(!format!("{mention:?}").contains("PMP"));
            mention.normalized_value.unwrap()
        })
        .collect::<Vec<_>>();
    normalized.sort();

    assert_eq!(normalized, vec!["cfa_level_1", "cissp", "cka", "pmp"]);

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn import_persists_sectioned_skill_alias_mentions_without_output_leaks() {
    let data_dir = temp_dir("persisted-skill-alias-data");
    let resume_root = temp_dir("persisted-skill-alias-resumes");
    fs::write(
        resume_root.join("synthetic-skill-candidate.txt"),
        "\
Synthetic Skill Candidate
Email: skill-candidate@example.test
Skills
Python / TypeScript / PostgreSQL
技术栈
K8s, Golang, Redis
Experience
Java island migration
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import skill aliases");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("Python"));
    assert!(!stdout.contains("skill-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-skill-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mut normalized = store
        .entity_mentions_for_version(&version.id)
        .unwrap()
        .into_iter()
        .filter(|mention| mention.entity_type == EntityType::Skill)
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert!(!format!("{mention:?}").contains("Python"));
            mention.normalized_value.unwrap()
        })
        .collect::<Vec<_>>();
    normalized.sort();

    assert_eq!(
        normalized,
        vec![
            "Go",
            "Kubernetes",
            "PostgreSQL",
            "Python",
            "Redis",
            "TypeScript"
        ]
    );

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn import_persists_labeled_school_and_degree_mentions_without_output_leaks() {
    let data_dir = temp_dir("persisted-labeled-education-data");
    let resume_root = temp_dir("persisted-labeled-education-resumes");
    fs::write(
        resume_root.join("synthetic-labeled-education-candidate.txt"),
        "\
Synthetic Education Candidate
Email: education-candidate@example.test
Education
School: Synthetic Institute of Technology
Degree: MSc Computer Science
教育经历
学校：合成大学
学历：博士研究生
Skills: Java
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import labeled education fields");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("Synthetic Institute"));
    assert!(!stdout.contains("合成大学"));
    assert!(!stdout.contains("博士研究生"));
    assert!(!stdout.contains("education-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-labeled-education-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mentions = store.entity_mentions_for_version(&version.id).unwrap();

    let school_normalized = mentions
        .iter()
        .filter(|mention| mention.entity_type == EntityType::School)
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert!(!mention.raw_value.contains(':'));
            assert!(!mention.raw_value.contains('：'));
            assert!(!format!("{mention:?}").contains("Synthetic Institute"));
            mention.normalized_value.as_deref().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        school_normalized,
        vec!["synthetic institute of technology", "合成大学"]
    );

    let degree_normalized = mentions
        .iter()
        .filter(|mention| mention.entity_type == EntityType::Degree)
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert!(!mention.raw_value.contains(':'));
            assert!(!mention.raw_value.contains('：'));
            assert!(!format!("{mention:?}").contains("博士"));
            mention.normalized_value.as_deref().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(degree_normalized, vec!["master", "doctor"]);

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn import_persists_school_tier_mentions_and_filters_search_without_output_leaks() {
    let data_dir = temp_dir("persisted-school-tier-data");
    let resume_root = temp_dir("persisted-school-tier-resumes");
    fs::write(
        resume_root.join("synthetic-school-tier-candidate.txt"),
        "\
Synthetic School Tier Candidate
Email: school-tier-candidate@example.test
Education
School: Synthetic 985 University (985/211/双一流)
Degree: Bachelor of Engineering
Skills: Java
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import school tier fields");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("Synthetic 985 University"));
    assert!(!stdout.contains("school-tier-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-school-tier-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mentions = store.entity_mentions_for_version(&version.id).unwrap();
    let tiers = mentions
        .iter()
        .filter(|mention| mention.entity_type == EntityType::SchoolTier)
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert!(!format!("{mention:?}").contains("985"));
            mention.normalized_value.as_deref().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(tiers, vec!["985", "211", "double_first_class"]);

    let tier_filtered = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--school-tier",
            "985",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run school-tier filtered search");
    assert!(
        tier_filtered.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&tier_filtered.stdout),
        String::from_utf8_lossy(&tier_filtered.stderr)
    );
    assert!(tier_filtered.stderr.is_empty());
    let tier_stdout = String::from_utf8_lossy(&tier_filtered.stdout);
    assert!(tier_stdout.contains("results: 1"));
    assert!(tier_stdout.contains("synthetic-school-tier-candidate.txt"));
    assert!(!tier_stdout.contains(path_str(&data_dir)));
    assert!(!tier_stdout.contains(path_str(&resume_root)));
    assert!(!tier_stdout.contains("school-tier-candidate@example.test"));

    let overseas_filtered = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "search",
            "Java",
            "--school-tier",
            "overseas",
            "--top-k",
            "20",
        ])
        .output()
        .expect("run non-matching school-tier filtered search");
    assert!(overseas_filtered.status.success());
    let overseas_stdout = String::from_utf8_lossy(&overseas_filtered.stdout);
    assert!(overseas_stdout.contains("results: 0"));

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn import_does_not_persist_degree_aliases_from_skill_lines() {
    let data_dir = temp_dir("persisted-degree-context-data");
    let resume_root = temp_dir("persisted-degree-context-resumes");
    fs::write(
        resume_root.join("synthetic-degree-context-candidate.txt"),
        "\
Synthetic Degree Context Candidate
Email: degree-context-candidate@example.test
Skills
MS SQL, Java
Experience
Built reporting systems
Education
Synthetic University
Bachelor of Science in Computer Science
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import degree context candidate");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("MS SQL"));
    assert!(!stdout.contains("Bachelor"));
    assert!(!stdout.contains("degree-context-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-degree-context-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mentions = store.entity_mentions_for_version(&version.id).unwrap();

    let degrees = mentions
        .iter()
        .filter(|mention| mention.entity_type == EntityType::Degree)
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert_ne!(mention.raw_value, "MS");
            assert!(!format!("{mention:?}").contains("Bachelor"));
            mention.normalized_value.as_deref().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(degrees, vec!["bachelor"]);

    let skills = mentions
        .iter()
        .filter(|mention| mention.entity_type == EntityType::Skill)
        .filter_map(|mention| mention.normalized_value.as_deref())
        .collect::<Vec<_>>();
    assert!(skills.contains(&"SQL"));
    assert!(skills.contains(&"Java"));

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn import_persists_labeled_company_and_title_mentions_without_output_leaks() {
    let data_dir = temp_dir("persisted-labeled-role-data");
    let resume_root = temp_dir("persisted-labeled-role-resumes");
    fs::write(
        resume_root.join("synthetic-labeled-role-candidate.txt"),
        "\
Synthetic Labeled Role Candidate
Email: labeled-role-candidate@example.test
Experience
Company: Synthetic Commerce Inc.
Title: Product Manager
工作经历
公司：合成科技有限公司
职位：高级后端工程师
Skills: Java
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import labeled company and title");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("Synthetic Commerce"));
    assert!(!stdout.contains("合成科技"));
    assert!(!stdout.contains("高级后端"));
    assert!(!stdout.contains("labeled-role-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-labeled-role-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mentions = store.entity_mentions_for_version(&version.id).unwrap();
    let company_normalized = mentions
        .iter()
        .filter(|mention| mention.entity_type == EntityType::Company)
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert!(!mention.raw_value.contains(':'));
            assert!(!mention.raw_value.contains('：'));
            assert!(!format!("{mention:?}").contains("Synthetic Commerce"));
            mention.normalized_value.as_deref().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(company_normalized, vec!["synthetic commerce", "合成科技"]);

    let title_normalized = mentions
        .iter()
        .filter(|mention| mention.entity_type == EntityType::Title)
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert!(!mention.raw_value.contains(':'));
            assert!(!mention.raw_value.contains('：'));
            assert!(!format!("{mention:?}").contains("高级后端"));
            mention.normalized_value.as_deref().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        title_normalized,
        vec!["product_manager", "backend_engineer"]
    );

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn import_persists_broader_title_alias_mentions_without_output_leaks() {
    let data_dir = temp_dir("persisted-title-alias-data");
    let resume_root = temp_dir("persisted-title-alias-resumes");
    fs::write(
        resume_root.join("synthetic-title-alias-candidate.txt"),
        "\
Synthetic Title Alias Candidate
Email: title-alias-candidate@example.test
Experience
Role: Staff Frontend Engineer
职位：数据科学家
Position: DevOps Engineer
Title: Engineering Manager
Certificate
AWS Certified Solutions Architect
Skills: Java
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import broader title aliases");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("Staff Frontend"));
    assert!(!stdout.contains("数据科学家"));
    assert!(!stdout.contains("DevOps Engineer"));
    assert!(!stdout.contains("title-alias-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-title-alias-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let title_mentions = store
        .entity_mentions_for_version(&version.id)
        .unwrap()
        .into_iter()
        .filter(|mention| mention.entity_type == EntityType::Title)
        .collect::<Vec<_>>();
    let normalized = title_mentions
        .iter()
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert!(!mention.raw_value.contains(':'));
            assert!(!mention.raw_value.contains('：'));
            assert!(!mention.raw_value.contains("AWS Certified"));
            assert!(!format!("{mention:?}").contains("Staff Frontend"));
            mention.normalized_value.as_deref().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        normalized,
        vec![
            "frontend_engineer",
            "data_scientist",
            "devops_engineer",
            "engineering_manager"
        ]
    );

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn import_persists_chinese_date_range_and_years_mentions_without_output_leaks() {
    let data_dir = temp_dir("persisted-chinese-date-data");
    let resume_root = temp_dir("persisted-chinese-date-resumes");
    fs::write(
        resume_root.join("synthetic-chinese-date-candidate.txt"),
        "\
Synthetic Date Candidate
Email: date-candidate@example.test
Experience
2020年1月 - 2024年3月
Synthetic Payments Inc.
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import Chinese date range");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("2020年1月"));
    assert!(!stdout.contains("date-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-chinese-date-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mentions = store.entity_mentions_for_version(&version.id).unwrap();
    let date_range = mentions
        .iter()
        .find(|mention| mention.entity_type == EntityType::DateRange)
        .unwrap();
    assert_eq!(
        date_range.normalized_value.as_deref(),
        Some("2020-01/2024-03")
    );
    assert!(date_range.span_start.is_some());
    assert!(date_range.span_end.is_some());
    assert!(!format!("{date_range:?}").contains("2020年1月"));

    let years = mentions
        .iter()
        .find(|mention| mention.entity_type == EntityType::YearsExperience)
        .unwrap();
    assert_eq!(years.normalized_value.as_deref(), Some("4.2"));
    assert!(years.span_start.is_some());
    assert!(years.span_end.is_some());
    assert!(!format!("{years:?}").contains("2020年1月"));

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn import_persists_present_date_range_and_years_mentions_without_output_leaks() {
    let data_dir = temp_dir("persisted-present-date-data");
    let resume_root = temp_dir("persisted-present-date-resumes");
    fs::write(
        resume_root.join("synthetic-present-date-candidate.txt"),
        "\
Synthetic Present Date Candidate
Email: present-date-candidate@example.test
Experience
2020年1月 - 至今
Project
Jan 2021 - Present
Contract
2022.03 - Current
Skills: Java
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import present date ranges");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("2020年1月"));
    assert!(!stdout.contains("Present"));
    assert!(!stdout.contains("present-date-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-present-date-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mentions = store.entity_mentions_for_version(&version.id).unwrap();
    let normalized = mentions
        .iter()
        .filter(|mention| mention.entity_type == EntityType::DateRange)
        .map(|mention| {
            assert!(mention.span_start.is_some());
            assert!(mention.span_end.is_some());
            assert!(!format!("{mention:?}").contains("Present"));
            mention.normalized_value.as_deref().unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        normalized,
        vec!["2020-01/PRESENT", "2021-01/PRESENT", "2022-03/PRESENT"]
    );

    let years = mentions
        .iter()
        .find(|mention| mention.entity_type == EntityType::YearsExperience)
        .unwrap();
    let years_value = years.normalized_value.as_deref().unwrap();
    let years_value = years_value.parse::<f32>().unwrap();
    assert!(years_value >= 10.0, "{years_value}");
    assert!(years.span_start.is_some());
    assert!(years.span_end.is_some());
    assert!(!format!("{years:?}").contains("Present"));

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

#[test]
fn import_persists_chinese_mobile_mentions_without_output_leaks() {
    let data_dir = temp_dir("persisted-chinese-mobile-data");
    let resume_root = temp_dir("persisted-chinese-mobile-resumes");
    fs::write(
        resume_root.join("synthetic-mobile-candidate.txt"),
        "\
Synthetic Mobile Candidate
Email: mobile-candidate@example.test
手机: 13800138000
备用电话: 139 0013 8001
Skills: Java
",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&resume_root),
        ])
        .output()
        .expect("import Chinese mobile numbers");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&resume_root)));
    assert!(!stdout.contains("13800138000"));
    assert!(!stdout.contains("139 0013 8001"));
    assert!(!stdout.contains("mobile-candidate@example.test"));

    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    let document = store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == "synthetic-mobile-candidate.txt")
        .unwrap();
    let version = store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let phones = store
        .entity_mentions_for_version(&version.id)
        .unwrap()
        .into_iter()
        .filter(|mention| mention.entity_type == EntityType::Phone)
        .collect::<Vec<_>>();

    assert_eq!(phones.len(), 2);
    for phone in phones {
        assert_eq!(phone.raw_value, "<redacted:phone>");
        assert_eq!(phone.normalized_value, None);
        assert!(phone.span_start.is_some());
        assert!(phone.span_end.is_some());
        assert!(!format!("{phone:?}").contains("13800138000"));
        assert!(!format!("{phone:?}").contains("139 0013 8001"));
    }

    remove_dir(&data_dir);
    remove_dir(&resume_root);
}

fn import_fixtures(data_dir: &Path) {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures/resumes");
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(data_dir),
            "import",
            "--root",
            path_str(&fixture_root),
        ])
        .output()
        .expect("import fixtures");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s16-cli-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}
