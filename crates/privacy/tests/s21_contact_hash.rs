use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use privacy::{
    contact_hash_key_path, inspect_contact_hash_key, ContactHashKeyState, ContactHasher,
    ContactKind,
};

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

#[test]
fn contact_hash_key_inspection_is_read_only_and_redacted() {
    let data_dir = temp_dir("privacy-key-inspect");
    let key_path = contact_hash_key_path(&data_dir);

    let missing = inspect_contact_hash_key(&data_dir).unwrap();
    assert_eq!(missing.state(), ContactHashKeyState::Missing);
    assert!(!key_path.exists());
    assert!(!format!("{missing:?}").contains(path_str(&data_dir)));

    fs::create_dir_all(key_path.parent().unwrap()).unwrap();
    fs::write(&key_path, "not-a-hex-key\n").unwrap();
    let invalid = inspect_contact_hash_key(&data_dir).unwrap();
    assert_eq!(invalid.state(), ContactHashKeyState::Invalid);
    assert!(!format!("{invalid:?}").contains("not-a-hex-key"));
    assert!(!format!("{invalid:?}").contains(path_str(&data_dir)));

    fs::write(&key_path, format!("{}\n", "a".repeat(64))).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();
        let weak = inspect_contact_hash_key(&data_dir).unwrap();
        assert_eq!(weak.state(), ContactHashKeyState::WeakPermissions);
        assert_eq!(key_mode(&key_path) & 0o777, 0o644);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let ready = inspect_contact_hash_key(&data_dir).unwrap();
    assert_eq!(ready.state(), ContactHashKeyState::Ready);
    assert!(!format!("{ready:?}").contains("a".repeat(64).as_str()));

    remove_dir(&data_dir);
}

#[cfg(unix)]
#[test]
fn contact_hash_key_inspection_reports_unreadable_without_leaks() {
    use std::os::unix::fs::PermissionsExt;

    let data_dir = temp_dir("privacy-key-unreadable");
    let key_path = contact_hash_key_path(&data_dir);
    fs::create_dir_all(key_path.parent().unwrap()).unwrap();
    fs::write(&key_path, format!("{}\n", "b".repeat(64))).unwrap();
    fs::set_permissions(
        key_path.parent().unwrap(),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();

    let unreadable = inspect_contact_hash_key(&data_dir).unwrap();
    assert_eq!(unreadable.state(), ContactHashKeyState::Unreadable);
    assert!(!format!("{unreadable:?}").contains(path_str(&data_dir)));
    assert!(!format!("{unreadable:?}").contains("b".repeat(64).as_str()));

    fs::set_permissions(
        key_path.parent().unwrap(),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
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

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

#[cfg(unix)]
fn key_mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path).unwrap().permissions().mode()
}
