use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use privacy::{contact_hash_key_path, ContactHasher, ContactKind};

#[test]
fn contact_hasher_outputs_stable_lowercase_redacted_hashes() {
    let hasher = ContactHasher::from_key_bytes([7_u8; 32]);

    let email_hash = hasher
        .hash_contact(ContactKind::Email, "shared.candidate@example.test")
        .unwrap();
    let same_email_hash = hasher
        .hash_contact(ContactKind::Email, "shared.candidate@example.test")
        .unwrap();
    let phone_hash = hasher
        .hash_contact(ContactKind::Phone, "+14155550132")
        .unwrap();

    assert_eq!(email_hash, same_email_hash);
    assert_ne!(email_hash.as_str(), phone_hash.as_str());
    assert_eq!(email_hash.as_str().len(), 64);
    assert!(email_hash
        .as_str()
        .bytes()
        .all(|byte| { byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) }));
    assert!(!format!("{email_hash:?}").contains(email_hash.as_str()));
    assert!(!format!("{hasher:?}").contains("07"));
}

#[test]
fn contact_hash_key_is_created_locally_without_contact_material() {
    let data_dir = temp_dir("privacy-key");
    let hasher = ContactHasher::load_or_create(&data_dir).unwrap();
    let key_path = contact_hash_key_path(&data_dir);
    let key_material = fs::read_to_string(&key_path).unwrap();

    assert_eq!(key_material.trim().len(), 64);
    assert!(!key_material.contains("shared.candidate@example.test"));
    assert!(!key_material.contains("+14155550132"));
    #[cfg(unix)]
    assert_eq!(key_mode(&key_path) & 0o777, 0o600);

    let reloaded = ContactHasher::load_or_create(&data_dir).unwrap();
    assert_eq!(
        hasher
            .hash_contact(ContactKind::Email, "shared.candidate@example.test")
            .unwrap(),
        reloaded
            .hash_contact(ContactKind::Email, "shared.candidate@example.test")
            .unwrap()
    );

    remove_dir(&data_dir);
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s21-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

#[cfg(unix)]
fn key_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path).unwrap().permissions().mode()
}
