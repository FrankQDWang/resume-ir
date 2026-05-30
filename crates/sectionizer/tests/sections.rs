use core_domain::SectionType;
use sectionizer::{sectionize, sectionize_with_max_len};

#[test]
fn recognizes_common_resume_headings() {
    let sections = sectionize("Education\nZhejiang University\nExperience\nPayment platform");

    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].section_type, SectionType::Education);
    assert_eq!(sections[0].text, "Zhejiang University");
    assert_eq!(sections[1].section_type, SectionType::Experience);
}

#[test]
fn falls_back_to_paragraph_and_length_chunks() {
    let text = "alpha beta gamma\n\ndelta epsilon zeta";
    let sections = sectionize_with_max_len(text, 12);

    assert!(sections.len() >= 2);
    assert!(
        sections
            .iter()
            .all(|section| section.section_type == SectionType::Other)
    );
    assert!(
        sections
            .iter()
            .all(|section| section.text.chars().count() <= 12)
    );
}
