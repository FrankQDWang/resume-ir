#[test]
fn exposes_text_normalizer_crate_identity() {
    assert_eq!(text_normalizer::crate_name(), "text-normalizer");
}

#[test]
fn normalizes_mixed_chinese_english_whitespace_and_preserves_offset_mapping() {
    use text_normalizer::TextNormalizer;

    let source = " 张三  Zhang\tSan\r\n技能：Rust  \t Python\r\n\r\n\r\n项目  | 角色  | 时间\r\n A | Owner | 2020 - 2022 ";
    let normalized = TextNormalizer::normalize(source);

    assert_eq!(
        normalized.text(),
        "张三 Zhang San\n技能：Rust Python\n\n项目 | 角色 | 时间\nA | Owner | 2020 - 2022"
    );

    let clean_start = normalized.text().find("Rust").unwrap();
    let clean_end = clean_start + "Rust".len();
    let original = normalized
        .original_span_for_clean_range(clean_start..clean_end)
        .unwrap();
    assert_eq!(&source[original], "Rust");
    assert!(!format!("{normalized:?}").contains("Zhang"));
}

#[test]
fn maps_clean_ranges_across_inserted_newlines_without_falling_back_to_document_start() {
    use text_normalizer::TextNormalizer;

    let source = "  First line\r\n  第二行";
    let normalized = TextNormalizer::normalize(source);
    let original = normalized
        .original_span_for_clean_range(0..normalized.text().len())
        .unwrap();

    assert_eq!(normalized.text(), "First line\n第二行");
    assert!(original.start > 0);
    assert!(source[original].starts_with("First line"));
}

#[test]
fn removes_repeated_page_headers_and_footers_without_dropping_unique_lines() {
    use text_normalizer::TextNormalizer;

    let source = "Confidential Resume\nAlice Synthetic\nPage 1 of 2\n\u{000c}Confidential Resume\nExperience\nPage 2 of 2";
    let normalized = TextNormalizer::normalize(source);

    assert!(!normalized.text().contains("Confidential Resume"));
    assert!(!normalized.text().contains("Page 1 of 2"));
    assert!(!normalized.text().contains("Page 2 of 2"));
    assert!(normalized.text().contains("Alice Synthetic"));
    assert!(normalized.text().contains("Experience"));
}

#[test]
fn repairs_ocr_spacing_noise_but_preserves_bullets_and_date_ranges() {
    use text_normalizer::TextNormalizer;

    let source = "E x p e r i e n c e\n• 2020 - 2022  Rust 平台";
    let normalized = TextNormalizer::normalize(source);

    assert!(normalized.text().contains("Experience"));
    assert!(normalized.text().contains("• 2020 - 2022 Rust 平台"));
}

#[test]
fn text_only_normalization_matches_origin_preserving_output() {
    use text_normalizer::TextNormalizer;

    let sources = [
        " 张三  Zhang\tSan\r\n技能：Rust  \t Python\r\n\r\n\r\n项目  | 角色  | 时间\r\n A | Owner | 2020 - 2022 ",
        "Confidential Resume\nAlice Synthetic\nPage 1 of 2\n\u{000c}Confidential Resume\nExperience\nPage 2 of 2",
        "E x p e r i e n c e\n• 2020 - 2022  Rust 平台",
        "",
        "  Single\tline  ",
    ];

    for source in sources {
        assert_eq!(
            TextNormalizer::normalize_text_only(source),
            TextNormalizer::normalize(source).text()
        );
    }
}
