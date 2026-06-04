use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use privacy::{ContactHasher, ContactKind};

#[test]
fn privacy_cli_backs_up_and_restores_contact_hash_key_without_output_leaks() {
    let source_dir = temp_dir("privacy-cli-source");
    let restore_dir = temp_dir("privacy-cli-target");
    let backup_dir = temp_dir("privacy-cli-backup");
    let backup_path = backup_dir.join("contact-key.backup");
    let private_email = "shared.candidate@example.test";

    let source = ContactHasher::load_or_create(&source_dir).unwrap();
    let source_hash = source
        .hash_contact(ContactKind::Email, private_email)
        .unwrap();
    let key_material =
        fs::read_to_string(source_dir.join("secrets").join("contact-hash-key-v1")).unwrap();

    let backup = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&source_dir),
            "privacy",
            "backup-contact-key",
            "--output",
            path_str(&backup_path),
        ])
        .output()
        .expect("run contact key backup");
    assert!(backup.status.success());
    assert!(backup.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&backup.stdout);
    assert!(stdout.contains("contact hash key backup: written"));
    assert!(!stdout.contains(path_str(&source_dir)));
    assert!(!stdout.contains(path_str(&backup_path)));
    assert!(!stdout.contains(private_email));
    assert!(!stdout.contains(key_material.trim()));
    assert!(backup_path.exists());

    let restore = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&restore_dir),
            "privacy",
            "restore-contact-key",
            "--input",
            path_str(&backup_path),
        ])
        .output()
        .expect("run contact key restore");
    assert!(restore.status.success());
    assert!(restore.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&restore.stdout);
    assert!(stdout.contains("contact hash key restore: restored"));
    assert!(!stdout.contains(path_str(&restore_dir)));
    assert!(!stdout.contains(path_str(&backup_path)));
    assert!(!stdout.contains(private_email));
    assert!(!stdout.contains(key_material.trim()));

    let restored = ContactHasher::load_or_create(&restore_dir).unwrap();
    assert_eq!(
        source_hash,
        restored
            .hash_contact(ContactKind::Email, private_email)
            .unwrap()
    );

    let duplicate_restore = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&restore_dir),
            "privacy",
            "restore-contact-key",
            "--input",
            path_str(&backup_path),
        ])
        .output()
        .expect("run duplicate contact key restore");
    assert!(!duplicate_restore.status.success());
    assert!(duplicate_restore.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&duplicate_restore.stderr);
    assert!(stderr.contains("privacy key already exists"));
    assert!(!stderr.contains(path_str(&restore_dir)));
    assert!(!stderr.contains(path_str(&backup_path)));
    assert!(!stderr.contains(private_email));
    assert!(!stderr.contains(key_material.trim()));

    remove_dir(&source_dir);
    remove_dir(&restore_dir);
    remove_dir(&backup_dir);
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s142-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}
