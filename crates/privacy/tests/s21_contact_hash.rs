use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use privacy::{
    backup_contact_hash_key, contact_hash_key_path, inspect_contact_hash_key,
    restore_contact_hash_key, ContactHashKeyState, ContactHasher, ContactKind,
};

const BACKUP_PASSPHRASE: &[u8] = b"synthetic local backup passphrase";
const WRONG_BACKUP_PASSPHRASE: &[u8] = b"synthetic wrong backup passphrase";

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
fn contact_hash_key_backup_and_restore_round_trip_without_leaking_key_or_contacts() {
    let data_dir = temp_dir("privacy-key-backup-source");
    let restore_dir = temp_dir("privacy-key-backup-target");
    let backup_dir = temp_dir("privacy-key-backup-output");
    let backup_path = backup_dir.join("contact-key.backup");
    let private_email = "shared.candidate@example.test";
    let private_phone = "+14155550132";

    let source = ContactHasher::load_or_create(&data_dir).unwrap();
    let source_hash = source
        .hash_contact(ContactKind::Email, private_email)
        .unwrap();
    let key_path = contact_hash_key_path(&data_dir);
    let key_material = fs::read_to_string(&key_path).unwrap();

    let backup = backup_contact_hash_key(&data_dir, &backup_path, BACKUP_PASSPHRASE).unwrap();
    assert_eq!(
        inspect_contact_hash_key(&restore_dir).unwrap().state(),
        ContactHashKeyState::Missing
    );
    assert!(backup_path.exists());
    #[cfg(unix)]
    assert_eq!(key_mode(&backup_path) & 0o777, 0o600);
    let backup_material = fs::read_to_string(&backup_path).unwrap();
    assert!(backup_material.contains("resume-ir-contact-hash-key-backup-v2"));
    assert!(!backup_material.contains("resume-ir-contact-hash-key-v1"));
    assert!(!backup_material.contains(private_email));
    assert!(!backup_material.contains(private_phone));
    assert!(!backup_material.contains("synthetic local backup passphrase"));
    assert!(!backup_material.contains(key_material.trim()));
    assert!(!format!("{backup:?}").contains(path_str(&data_dir)));
    assert!(!format!("{backup:?}").contains(path_str(&backup_path)));
    assert!(!format!("{backup:?}").contains(key_material.trim()));

    let restore = restore_contact_hash_key(&restore_dir, &backup_path, BACKUP_PASSPHRASE).unwrap();
    let restored = ContactHasher::load_or_create(&restore_dir).unwrap();
    assert_eq!(
        source_hash,
        restored
            .hash_contact(ContactKind::Email, private_email)
            .unwrap()
    );
    let restored_key_path = contact_hash_key_path(&restore_dir);
    #[cfg(unix)]
    assert_eq!(key_mode(&restored_key_path) & 0o777, 0o600);
    assert!(!format!("{restore:?}").contains(path_str(&restore_dir)));
    assert!(!format!("{restore:?}").contains(path_str(&backup_path)));
    assert!(!format!("{restore:?}").contains(key_material.trim()));

    remove_dir(&data_dir);
    remove_dir(&restore_dir);
    remove_dir(&backup_dir);
}

#[test]
fn contact_hash_key_restore_refuses_to_overwrite_existing_key() {
    let source_dir = temp_dir("privacy-key-restore-source");
    let target_dir = temp_dir("privacy-key-restore-target");
    let backup_dir = temp_dir("privacy-key-restore-output");
    let backup_path = backup_dir.join("contact-key.backup");
    let private_email = "shared.candidate@example.test";

    let source = ContactHasher::load_or_create(&source_dir).unwrap();
    let source_hash = source
        .hash_contact(ContactKind::Email, private_email)
        .unwrap();
    backup_contact_hash_key(&source_dir, &backup_path, BACKUP_PASSPHRASE).unwrap();

    let target = ContactHasher::load_or_create(&target_dir).unwrap();
    let target_hash = target
        .hash_contact(ContactKind::Email, private_email)
        .unwrap();
    assert_ne!(source_hash, target_hash);

    let error = restore_contact_hash_key(&target_dir, &backup_path, BACKUP_PASSPHRASE).unwrap_err();
    assert_eq!(error.to_string(), "privacy key already exists");
    assert!(!format!("{error:?}").contains(path_str(&target_dir)));
    assert!(!format!("{error:?}").contains(path_str(&backup_path)));

    let still_target = ContactHasher::load_or_create(&target_dir).unwrap();
    assert_eq!(
        target_hash,
        still_target
            .hash_contact(ContactKind::Email, private_email)
            .unwrap()
    );

    remove_dir(&source_dir);
    remove_dir(&target_dir);
    remove_dir(&backup_dir);
}

#[test]
fn contact_hash_key_restore_with_wrong_passphrase_refuses_without_creating_key() {
    let source_dir = temp_dir("privacy-key-wrong-passphrase-source");
    let target_dir = temp_dir("privacy-key-wrong-passphrase-target");
    let backup_dir = temp_dir("privacy-key-wrong-passphrase-output");
    let backup_path = backup_dir.join("contact-key.backup");

    ContactHasher::load_or_create(&source_dir).unwrap();
    backup_contact_hash_key(&source_dir, &backup_path, BACKUP_PASSPHRASE).unwrap();

    let error =
        restore_contact_hash_key(&target_dir, &backup_path, WRONG_BACKUP_PASSPHRASE).unwrap_err();
    assert_eq!(
        error.to_string(),
        "privacy key backup is invalid or cannot be decrypted"
    );
    assert_eq!(
        inspect_contact_hash_key(&target_dir).unwrap().state(),
        ContactHashKeyState::Missing
    );
    assert!(!format!("{error:?}").contains(path_str(&target_dir)));
    assert!(!format!("{error:?}").contains(path_str(&backup_path)));
    assert!(!format!("{error:?}").contains("synthetic wrong backup passphrase"));

    remove_dir(&source_dir);
    remove_dir(&target_dir);
    remove_dir(&backup_dir);
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
