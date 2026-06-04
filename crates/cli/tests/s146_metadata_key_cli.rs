use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{MetaStore, MetadataEncryptionState};

#[test]
fn privacy_cli_backs_up_and_restores_metadata_sqlcipher_key_without_output_leaks() {
    let source_dir = temp_dir("metadata-key-source");
    let restore_dir = temp_dir("metadata-key-target");
    let wrong_restore_dir = temp_dir("metadata-key-wrong-passphrase-target");
    let backup_dir = temp_dir("metadata-key-backup");
    let backup_path = backup_dir.join("metadata-key.backup");
    let passphrase_path = backup_dir.join("metadata-key.passphrase");
    let wrong_passphrase_path = backup_dir.join("metadata-key-wrong.passphrase");
    let backup_passphrase = "synthetic local metadata key backup passphrase";

    fs::write(&passphrase_path, format!("{backup_passphrase}\n")).unwrap();
    fs::write(
        &wrong_passphrase_path,
        "synthetic wrong metadata key backup passphrase\n",
    )
    .unwrap();

    let initialize = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&source_dir), "doctor"])
        .output()
        .expect("initialize encrypted metadata store");
    assert!(initialize.status.success());
    assert!(initialize.stderr.is_empty());
    let initialize_stdout = String::from_utf8_lossy(&initialize.stdout);
    assert!(initialize_stdout.contains("metadata encryption: sqlcipher"));
    assert!(!initialize_stdout.contains(path_str(&source_dir)));

    let source_db = source_dir.join("metadata.sqlite3");
    let key_path = source_dir
        .join("metadata-secrets")
        .join("metadata-sqlcipher-key-v1");
    let key_material = fs::read_to_string(&key_path).unwrap();
    let encrypted_bytes = fs::read(&source_db).unwrap();
    assert!(!encrypted_bytes.starts_with(b"SQLite format 3"));

    let backup = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&source_dir),
            "privacy",
            "backup-metadata-key",
            "--output",
            path_str(&backup_path),
            "--passphrase-file",
            path_str(&passphrase_path),
        ])
        .output()
        .expect("run metadata key backup");
    assert!(backup.status.success());
    assert!(backup.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&backup.stdout);
    assert!(stdout.contains("metadata encryption key backup: written"));
    assert!(!stdout.contains(path_str(&source_dir)));
    assert!(!stdout.contains(path_str(&backup_path)));
    assert!(!stdout.contains(path_str(&passphrase_path)));
    assert!(!stdout.contains(backup_passphrase));
    assert!(!stdout.contains(key_material.trim()));
    assert!(backup_path.exists());

    let backup_material = fs::read_to_string(&backup_path).unwrap();
    assert!(backup_material.contains("resume-ir-metadata-sqlcipher-key-backup-v1"));
    assert!(!backup_material.contains(backup_passphrase));
    assert!(!backup_material.contains(key_material.trim()));

    fs::copy(&source_db, restore_dir.join("metadata.sqlite3")).unwrap();
    let restore = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&restore_dir),
            "privacy",
            "restore-metadata-key",
            "--input",
            path_str(&backup_path),
            "--passphrase-file",
            path_str(&passphrase_path),
        ])
        .output()
        .expect("run metadata key restore");
    assert!(restore.status.success());
    assert!(restore.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&restore.stdout);
    assert!(stdout.contains("metadata encryption key restore: restored"));
    assert!(!stdout.contains(path_str(&restore_dir)));
    assert!(!stdout.contains(path_str(&backup_path)));
    assert!(!stdout.contains(path_str(&passphrase_path)));
    assert!(!stdout.contains(backup_passphrase));
    assert!(!stdout.contains(key_material.trim()));

    let restored = MetaStore::open_data_dir(&restore_dir).unwrap();
    assert_eq!(
        restored.metadata_encryption_state(),
        MetadataEncryptionState::SqlCipher
    );
    assert_eq!(restored.schema_version().unwrap(), 16);

    fs::copy(&source_db, wrong_restore_dir.join("metadata.sqlite3")).unwrap();
    let wrong_passphrase_restore = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&wrong_restore_dir),
            "privacy",
            "restore-metadata-key",
            "--input",
            path_str(&backup_path),
            "--passphrase-file",
            path_str(&wrong_passphrase_path),
        ])
        .output()
        .expect("run wrong-passphrase metadata key restore");
    assert!(!wrong_passphrase_restore.status.success());
    assert!(wrong_passphrase_restore.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&wrong_passphrase_restore.stderr);
    assert!(stderr.contains("metadata key backup is invalid or cannot be decrypted"));
    assert!(!stderr.contains(path_str(&wrong_restore_dir)));
    assert!(!stderr.contains(path_str(&backup_path)));
    assert!(!stderr.contains(path_str(&wrong_passphrase_path)));
    assert!(!stderr.contains("synthetic wrong metadata key backup passphrase"));
    assert!(!stderr.contains(key_material.trim()));
    assert!(!wrong_restore_dir
        .join("metadata-secrets")
        .join("metadata-sqlcipher-key-v1")
        .exists());

    let duplicate_restore = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&restore_dir),
            "privacy",
            "restore-metadata-key",
            "--input",
            path_str(&backup_path),
            "--passphrase-file",
            path_str(&passphrase_path),
        ])
        .output()
        .expect("run duplicate metadata key restore");
    assert!(!duplicate_restore.status.success());
    assert!(duplicate_restore.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&duplicate_restore.stderr);
    assert!(stderr.contains("metadata encryption key already exists"));
    assert!(!stderr.contains(path_str(&restore_dir)));
    assert!(!stderr.contains(path_str(&backup_path)));
    assert!(!stderr.contains(path_str(&passphrase_path)));
    assert!(!stderr.contains(backup_passphrase));
    assert!(!stderr.contains(key_material.trim()));

    remove_dir(&source_dir);
    remove_dir(&restore_dir);
    remove_dir(&wrong_restore_dir);
    remove_dir(&backup_dir);
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s146-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}
