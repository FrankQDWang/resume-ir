//! Sectionizer contract tests.

use core_domain::SectionType;
use sectionizer::{sectionize, sectionize_with_options, SectionizeOptions};

#[test]
fn detects_basic_mixed_chinese_english_headings() {
    let text = "Contact\nEmail unit@example.test\n技能\nRust\n项目经历\n内部工具";

    let sections = sectionize(text);

    assert_eq!(sections.len(), 3);
    assert_eq!(sections[0].section_type(), SectionType::Contact);
    assert_eq!(sections[1].section_type(), SectionType::Skill);
    assert_eq!(sections[2].section_type(), SectionType::Project);
    assert!(sections.iter().all(|section| section.confidence() >= 0.80));
}

#[test]
fn falls_back_to_paragraph_chunks_when_headings_are_absent() {
    let text = "第一段 mixed English content without headings.\n\n第二段 contains more synthetic table-like text | A | B.\n\nThird paragraph adds enough text to require another chunk.";
    let options = SectionizeOptions::new(72);

    let sections = sectionize_with_options(text, options);

    assert!(sections.len() >= 2);
    assert!(sections
        .iter()
        .all(|section| section.section_type() == SectionType::Other));
    assert!(sections.iter().all(|section| section.confidence() < 0.50));
    assert!(sections
        .windows(2)
        .all(|pair| pair[0].char_end() <= pair[1].char_start()));
}

#[test]
fn keeps_table_linearized_text_in_fallback_chunks() {
    let text = "字段 | 值\nEmail | unit@example.test\nPhone | +99 000 0000 0000";

    let sections = sectionize(text);

    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].text(), text);
    assert_eq!(sections[0].section_type(), SectionType::Other);
}

#[test]
fn debug_output_redacts_section_text() {
    let text = "Contact\nEmail unit@example.test";
    let sections = sectionize(text);

    let debug = format!("{:?}", sections[0]);

    assert!(debug.contains("[redacted section text]"));
    assert!(!debug.contains("unit@example.test"));
}
