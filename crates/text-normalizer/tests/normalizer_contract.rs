//! Text normalizer contract tests.

use text_normalizer::normalize_text;

#[test]
fn cleans_mixed_chinese_english_whitespace_and_repeated_page_lines() {
    let raw = "Synthetic Header\n技能\tRust   Python\n经历  Build 工具\nPage 1\n\u{000c}Synthetic Header\n教育  计算机 Science\nPage 2";

    let normalized = normalize_text(raw);

    assert_eq!(
        normalized.text(),
        "技能 Rust Python\n经历 Build 工具\n教育 计算机 Science"
    );
    assert!(!normalized.text().contains("Synthetic Header"));
    assert!(!normalized.text().contains("Page 1"));
    assert!(!normalized.text().contains("Page 2"));
}

#[test]
fn preserves_table_linearized_content_while_collapsing_spacing() {
    let raw =
        "字段 | 值\nEmail\t|\tunit@example.test\nPhone | +99 000 0000 0000\nSkills | Rust  |  中文";

    let normalized = normalize_text(raw);

    assert_eq!(
        normalized.text(),
        "字段 | 值\nEmail | unit@example.test\nPhone | +99 000 0000 0000\nSkills | Rust | 中文"
    );
}

#[test]
fn repeated_page_cleanup_does_not_remove_matching_body_lines() {
    let raw = "Synthetic Header\nRepeated Body Token\n正文 A\n\u{000c}Synthetic Header\n正文 B\nRepeated Body Token";

    let normalized = normalize_text(raw);

    assert_eq!(
        normalized.text(),
        "Repeated Body Token\n正文 A\n正文 B\nRepeated Body Token"
    );
    assert!(!normalized.text().contains("Synthetic Header"));
}

#[test]
fn maps_normalized_offsets_back_to_original_offsets() {
    let raw = "技能\tRust\n经验  中文";

    let normalized = normalize_text(raw);
    let rust_start = normalized.text().find("Rust");
    let original_rust_start = raw.find("Rust");
    let chinese_start = normalized.text().find("中文");
    let original_chinese_start = raw.find("中文");

    assert_eq!(
        rust_start.and_then(|offset| normalized.map().original_offset_for(offset)),
        original_rust_start
    );
    assert_eq!(
        chinese_start.and_then(|offset| normalized.map().original_offset_for(offset)),
        original_chinese_start
    );
    assert_eq!(
        rust_start.and_then(|offset| {
            normalized
                .map()
                .normalized_span_to_original(offset, offset + "Rust".len())
        }),
        original_rust_start.map(|offset| (offset, offset + "Rust".len()))
    );
    assert_eq!(
        chinese_start.and_then(|offset| {
            normalized
                .map()
                .normalized_span_to_original(offset, offset + "中文".len())
        }),
        original_chinese_start.map(|offset| (offset, offset + "中文".len()))
    );
}

#[test]
fn debug_output_redacts_clean_text() {
    let normalized = normalize_text("Email unit@example.test");

    let debug = format!("{normalized:?}");

    assert!(debug.contains("text_len"));
    assert!(!debug.contains("unit@example.test"));
}
