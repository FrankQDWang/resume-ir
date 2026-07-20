use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{metadata_encryption_key_path, metadata_store_path, ReadMetaStore};
use rusqlite::{Connection, OpenFlags};

mod support;

#[test]
fn privacy_cli_rotates_metadata_sqlcipher_key_without_output_leaks() {
    let data_dir = temp_dir("metadata-key-rotation");

    drop(support::create_store(&data_dir));

    let initialize = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("initialize encrypted metadata store");
    assert!(initialize.status.success());
    assert!(initialize.stderr.is_empty());

    let db_path = metadata_store_path(&data_dir).unwrap();
    let key_path = metadata_encryption_key_path(&data_dir);
    let old_key_hex = fs::read_to_string(&key_path).unwrap();
    let old_key = decode_key(old_key_hex.trim());
    assert!(can_read_schema_with_key(&db_path, &old_key));

    let rotation = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "privacy",
            "rotate-metadata-key",
        ])
        .output()
        .expect("run metadata key rotation");
    assert!(rotation.status.success());
    assert!(rotation.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&rotation.stdout);
    assert!(stdout.contains("metadata encryption key rotation: rotated"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(path_str(&db_path)));
    assert!(!stdout.contains(path_str(&key_path)));
    assert!(!stdout.contains(old_key_hex.trim()));

    let new_key_hex = fs::read_to_string(&key_path).unwrap();
    let new_key = decode_key(new_key_hex.trim());
    assert_ne!(old_key_hex.trim(), new_key_hex.trim());
    assert!(!String::from_utf8_lossy(&rotation.stdout).contains(new_key_hex.trim()));
    assert!(!can_read_schema_with_key(&db_path, &old_key));
    assert!(can_read_schema_with_key(&db_path, &new_key));
    let reopened = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(reopened.schema_version().unwrap(), 29);

    let doctor = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "doctor"])
        .output()
        .expect("run doctor after metadata key rotation");
    assert!(doctor.status.success());
    assert!(doctor.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("metadata encryption: sqlcipher"));
    assert!(!stdout.contains(path_str(&data_dir)));
    assert!(!stdout.contains(old_key_hex.trim()));
    assert!(!stdout.contains(new_key_hex.trim()));

    remove_dir(&data_dir);
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s147-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

fn remove_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

fn decode_key(value: &str) -> [u8; 32] {
    assert_eq!(value.len(), 64);
    let mut key = [0_u8; 32];
    for (index, slot) in key.iter_mut().enumerate() {
        let start = index * 2;
        *slot = u8::from_str_radix(&value[start..start + 2], 16).unwrap();
    }
    key
}

fn can_read_schema_with_key(path: &Path, key: &[u8; 32]) -> bool {
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return false;
    };
    let key_hex = key
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if connection
        .pragma_update(None, "key", format!("x'{key_hex}'"))
        .is_err()
    {
        return false;
    }
    connection
        .query_row("SELECT COUNT(*) FROM sqlite_master", [], |_| Ok(()))
        .is_ok()
}
