use text_normalizer::normalize_text;

#[test]
fn cleans_whitespace_headers_footers_and_preserves_offsets() {
    let input = "Name\r\n\r\nPage 1\nJava\t\tBackend\n页码 1";

    let clean = normalize_text(input);

    assert_eq!(clean.text, "Name\nJava Backend");
    let java_clean_index = clean
        .text
        .chars()
        .position(|char| char == 'J')
        .expect("Java");
    assert_eq!(
        clean.original_byte_offset(java_clean_index),
        input.find("Java")
    );
}

#[test]
fn preserves_chinese_english_mixed_text() {
    let input = "技能：Rust  \t 后端\n项目：支付 网关";

    let clean = normalize_text(input);

    assert_eq!(clean.text, "技能：Rust 后端\n项目：支付 网关");
}
