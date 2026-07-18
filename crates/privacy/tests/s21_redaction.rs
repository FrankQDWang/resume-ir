use std::borrow::Cow;

use privacy::redact_contact_values;

#[test]
fn redaction_covers_email_separated_and_compact_phone_wechat_and_local_paths() {
    let redacted = redact_contact_values(
        "Email: person@example.test Phone: +1 650-555-1234 Compact: 16505551234 \
         WeChat: wx_candidate01 Paths: /Users/person/private/resume.pdf \
         file:///private/tmp/resume.pdf C:\\Users\\person\\resume.pdf",
    );

    assert!(redacted.contains("<redacted-email>"));
    assert_eq!(redacted.matches("<redacted-phone>").count(), 2);
    assert!(redacted.contains("<redacted-wechat>"));
    assert_eq!(redacted.matches("<redacted-path>").count(), 3);
    for private_value in [
        "person@example.test",
        "650-555-1234",
        "16505551234",
        "wx_candidate01",
        "/Users/person",
        "file:///private",
        "C:\\Users\\person",
    ] {
        assert!(!redacted.contains(private_value));
    }
}

#[test]
fn redaction_covers_supported_phone_separator_shapes() {
    for phone in [
        "(415) 555-0132",
        "(415)555-0132",
        "+1(415)555-0132",
        "+14155550132",
    ] {
        let redacted = redact_contact_values(phone);
        assert_eq!(redacted, "<redacted-phone>", "phone shape: {phone}");
    }
}

#[test]
fn redaction_covers_ascii_email_adjacent_to_non_ascii_text() {
    for text in ["中文person@example.test中文", "éperson@example.testé"] {
        let redacted = redact_contact_values(text);
        assert!(redacted.contains("<redacted-email>"));
        assert!(!redacted.contains("person@example.test"));
    }
}

#[test]
fn no_signal_and_date_only_text_use_the_borrowed_fast_path() {
    for text in [
        "Synthetic candidate summary without contact markers",
        "Experience 2020-01 to 2024-12; led 3 projects and 2 teams",
    ] {
        let redacted = redact_contact_values(text);
        assert!(matches!(redacted, Cow::Borrowed(value) if value == text));
    }
}
