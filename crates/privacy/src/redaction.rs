use std::borrow::Cow;
use std::sync::OnceLock;

use regex::Regex;

const REDACTED_EMAIL: &str = "<redacted-email>";
const REDACTED_PHONE: &str = "<redacted-phone>";
const REDACTED_WECHAT: &str = "<redacted-wechat>";
const REDACTED_PATH: &str = "<redacted-path>";

/// Removes contact values and local paths before text crosses a privacy boundary.
///
/// Text without a supported signal is returned as a borrowed value, allowing callers
/// on hot paths to avoid both regular-expression work and allocation.
pub fn redact_contact_values(text: &str) -> Cow<'_, str> {
    static EMAIL_REGEX: OnceLock<Regex> = OnceLock::new();
    static SEPARATED_PHONE_REGEX: OnceLock<Regex> = OnceLock::new();
    static COMPACT_PHONE_REGEX: OnceLock<Regex> = OnceLock::new();
    static WECHAT_REGEX: OnceLock<Regex> = OnceLock::new();
    static LOCAL_PATH_REGEX: OnceLock<Regex> = OnceLock::new();

    let mut redacted = None;
    if text.contains('@') {
        replace_redaction(
            &mut redacted,
            text,
            EMAIL_REGEX.get_or_init(|| {
                Regex::new(r"(?-u:\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b)")
                    .expect("email redaction regex must compile")
            }),
            REDACTED_EMAIL,
        );
    }
    if contains_separated_phone_signal(redacted_text(text, &redacted)) {
        replace_redaction(
            &mut redacted,
            text,
            SEPARATED_PHONE_REGEX.get_or_init(|| {
                Regex::new(
                    r"(?x)
                    (?:\+\d{1,3}[\s.-]*)?
                    (?:
                        \(\d{3}\)[\s.-]*
                        |
                        \d{3,4}[\s.-]+
                    )
                    \d{3,4}[\s.-]*\d{4}
                    ",
                )
                .expect("separated-phone redaction regex must compile")
            }),
            REDACTED_PHONE,
        );
    }
    if contains_compact_phone_signal(redacted_text(text, &redacted)) {
        replace_redaction(
            &mut redacted,
            text,
            COMPACT_PHONE_REGEX.get_or_init(|| {
                Regex::new(r"\+?(?:1)?\d{10}\b")
                    .expect("compact-phone redaction regex must compile")
            }),
            REDACTED_PHONE,
        );
    }
    if contains_wechat_signal(redacted_text(text, &redacted)) {
        replace_redaction(
            &mut redacted,
            text,
            WECHAT_REGEX.get_or_init(|| {
                Regex::new(
                    r"(?ix)\b(?:wechat|weixin|wx|微信|微信号)\s*[:：]\s*[A-Za-z][A-Za-z0-9_.-]{5,31}\b",
                )
                .expect("WeChat redaction regex must compile")
            }),
            REDACTED_WECHAT,
        );
    }
    if contains_local_path_signal(redacted_text(text, &redacted)) {
        replace_redaction(
            &mut redacted,
            text,
            LOCAL_PATH_REGEX.get_or_init(|| {
                Regex::new(
                    r"(?ix)
                    (?:
                        file://\S+
                        |
                        (?:~|/Users|/home|/private|/var|/tmp|[A-Z]:[\\/])\S*
                        |
                        \b[A-Z]:\\\S+
                        |
                        \S*(?:/Users/|/home/|/private|\\Users\\)\S*
                    )
                    ",
                )
                .expect("local-path redaction regex must compile")
            }),
            REDACTED_PATH,
        );
    }

    match redacted {
        Some(value) => Cow::Owned(value),
        None => Cow::Borrowed(text),
    }
}

fn replace_redaction(
    current: &mut Option<String>,
    original: &str,
    regex: &Regex,
    replacement: &str,
) {
    if let Cow::Owned(value) = regex.replace_all(redacted_text(original, current), replacement) {
        *current = Some(value);
    }
}

fn redacted_text<'a>(original: &'a str, redacted: &'a Option<String>) -> &'a str {
    redacted.as_deref().unwrap_or(original)
}

fn contains_separated_phone_signal(text: &str) -> bool {
    let mut digits = 0_usize;
    let mut candidate_len = 0_usize;
    let mut separator_seen = false;

    for byte in text.bytes() {
        match byte {
            b'0'..=b'9' => {
                digits += 1;
                candidate_len += 1;
                if digits >= 10 && separator_seen {
                    return true;
                }
            }
            b'+' | b'(' | b')' => {
                candidate_len += 1;
                separator_seen = true;
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'.' | b'-' if candidate_len > 0 => {
                candidate_len += 1;
                separator_seen = true;
            }
            _ => {
                digits = 0;
                candidate_len = 0;
                separator_seen = false;
            }
        }

        if candidate_len > 32 {
            digits = 0;
            candidate_len = 0;
            separator_seen = false;
        }
    }

    false
}

fn contains_compact_phone_signal(text: &str) -> bool {
    let mut consecutive_digits = 0_usize;
    for byte in text.bytes() {
        if byte.is_ascii_digit() {
            consecutive_digits += 1;
            if consecutive_digits >= 10 {
                return true;
            }
        } else {
            consecutive_digits = 0;
        }
    }

    false
}

fn contains_wechat_signal(text: &str) -> bool {
    text.contains("微信")
        || contains_ascii_case_insensitive(text, b"wechat")
        || contains_ascii_case_insensitive(text, b"weixin")
        || contains_ascii_case_insensitive(text, b"wx")
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &[u8]) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn contains_local_path_signal(text: &str) -> bool {
    text.as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'/' | b'\\' | b'~'))
}
