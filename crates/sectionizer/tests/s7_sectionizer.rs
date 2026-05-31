use core_domain::SectionType;

#[test]
fn exposes_sectionizer_crate_identity() {
    assert_eq!(sectionizer::crate_name(), "sectionizer");
}

#[test]
fn recognizes_resume_sections_in_chinese_and_english_order() {
    use sectionizer::Sectionizer;

    let text = "联系方式\nsynthetic@example.test\n\n教育经历\nExample University\n\nExperience\nSynthetic Labs\n\n技能\nRust Python";
    let sections = Sectionizer::default().sectionize(text);

    let section_types = sections
        .iter()
        .map(|section| section.section_type.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        section_types,
        vec![
            SectionType::Contact,
            SectionType::Education,
            SectionType::Experience,
            SectionType::Skill,
        ]
    );
    assert_eq!(
        slice_by_char_range(text, sections[1].char_start..sections[1].char_end),
        sections[1].text
    );
    assert!(sections.iter().all(|section| section.confidence >= 0.8));
    assert!(!format!("{:?}", sections[0]).contains("synthetic@example.test"));
}

#[test]
fn falls_back_to_length_and_paragraph_chunks_when_no_headings_match() {
    use sectionizer::Sectionizer;

    let text = "Built local search tooling with deterministic tests.\n\nImproved parsing quality for mixed Chinese English resumes.\n\nMaintained synthetic fixtures only for privacy boundaries.";
    let sections = Sectionizer::with_max_chars(70).sectionize(text);

    assert!(sections.len() >= 2);
    assert!(sections
        .iter()
        .all(|section| section.section_type == SectionType::Other("chunk".to_string())));
    assert!(sections
        .iter()
        .all(|section| section.text.chars().count() <= 70));
    assert!(sections
        .windows(2)
        .all(|pair| pair[0].char_end <= pair[1].char_start));
}

#[test]
fn fallback_splits_single_overlong_paragraph_by_length() {
    use sectionizer::Sectionizer;

    let text = "中英MixedResumeContent".repeat(8);
    let sections = Sectionizer::with_max_chars(20).sectionize(&text);

    assert!(sections.len() > 1);
    assert!(sections
        .iter()
        .all(|section| section.text.chars().count() <= 20));
    assert!(sections.iter().all(|section| slice_by_char_range(
        &text,
        section.char_start..section.char_end
    ) == section.text));
}

#[test]
fn table_linearized_text_is_kept_inside_nearest_section() {
    use sectionizer::Sectionizer;

    let text = "项目经历\n项目 | 角色 | 时间\nIR | Owner | 2020-2021\n职责：本地检索";
    let sections = Sectionizer::default().sectionize(text);

    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].section_type, SectionType::Project);
    assert!(sections[0].text.contains("项目 | 角色 | 时间"));
    assert!(sections[0].text.contains("IR | Owner | 2020-2021"));
}

fn slice_by_char_range(text: &str, range: std::ops::Range<usize>) -> String {
    text.chars()
        .skip(range.start)
        .take(range.end - range.start)
        .collect()
}
