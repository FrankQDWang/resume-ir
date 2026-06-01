use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{Candidate, CandidateId, ContactHash, EntityType, MetaStore, ResumeVersion};

#[test]
fn import_assigns_candidates_from_hashed_contacts_and_search_folds_versions() {
    let data_dir = temp_dir("candidate-data");
    let root = temp_dir("candidate-root");
    write_pdf_resume(
        &root.join("synthetic-shared-contact-a.pdf"),
        "Synthetic backend profile A",
        "Built Java services for local search ranking.",
        "Shared.Candidate@Example.Test",
        "(415) 555-0132",
    );
    write_pdf_resume(
        &root.join("synthetic-shared-contact-b.pdf"),
        "Synthetic backend profile B",
        "Maintained Java systems for indexing and ranking.",
        "shared.candidate@example.test",
        "+1 415-555-0132",
    );

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root),
        ])
        .output()
        .expect("run resume-cli import");
    assert!(
        import.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&import.stdout),
        String::from_utf8_lossy(&import.stderr)
    );
    assert!(import.stderr.is_empty());

    let key_path = data_dir.join("secrets").join("contact-hash-key-v1");
    assert!(
        key_path.exists(),
        "contact hash key must be local and durable"
    );
    let key_material = fs::read_to_string(&key_path).expect("read contact hash key");
    assert_eq!(key_material.trim().len(), 64);
    assert!(!key_material.contains("Shared.Candidate"));
    assert!(!key_material.contains("415"));
    #[cfg(unix)]
    assert_eq!(key_mode(&key_path) & 0o777, 0o600);

    let versions = searchable_versions(&data_dir);
    assert_eq!(versions.len(), 2);
    let first_candidate_id = versions[0]
        .candidate_id
        .as_ref()
        .expect("first version candidate")
        .clone();
    assert!(
        versions
            .iter()
            .all(|version| version.candidate_id.as_ref() == Some(&first_candidate_id)),
        "same hashed contact should assign one candidate"
    );

    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let candidate = store
        .candidate_by_id(&first_candidate_id)
        .unwrap()
        .expect("assigned candidate");
    assert!(candidate.email_hash.is_some());
    assert!(candidate.phone_hash.is_some());
    assert_eq!(candidate.version_count, 2);
    assert!(!format!("{candidate:?}").contains("shared.candidate@example.test"));
    assert!(!format!("{candidate:?}").contains("+14155550132"));

    for version in &versions {
        let mentions = store.entity_mentions_for_version(&version.id).unwrap();
        let contact_dump = mentions
            .iter()
            .filter(|mention| matches!(mention.entity_type, EntityType::Email | EntityType::Phone))
            .map(|mention| format!("{} {:?}", mention.raw_value, mention.normalized_value))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(contact_dump.contains("<redacted:email>"));
        assert!(contact_dump.contains("<redacted:phone>"));
        assert!(!contact_dump.contains("Shared.Candidate"));
        assert!(!contact_dump.contains("shared.candidate@example.test"));
        assert!(!contact_dump.contains("415-555-0132"));
        assert!(!contact_dump.contains("+14155550132"));
    }

    let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args(["--data-dir", path_str(&data_dir), "search", "Java"])
        .output()
        .expect("run resume-cli search");
    assert!(search.status.success());
    assert!(search.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&search.stdout);
    assert!(stdout.contains("results: 1"));
    assert!(!stdout.contains("Shared.Candidate"));
    assert!(!stdout.contains("shared.candidate@example.test"));
    assert!(!stdout.contains("415-555-0132"));
    drop(store);

    let reimport = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root),
        ])
        .output()
        .expect("run resume-cli reimport");
    assert!(
        reimport.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&reimport.stdout),
        String::from_utf8_lossy(&reimport.stderr)
    );
    assert_eq!(fs::read_to_string(&key_path).unwrap(), key_material);
    let versions_after_reimport = searchable_versions(&data_dir);
    assert_eq!(versions_after_reimport.len(), 2);
    assert!(versions_after_reimport
        .iter()
        .all(|version| { version.candidate_id.as_ref() == Some(&first_candidate_id) }));
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let candidate = store
        .candidate_by_id(&first_candidate_id)
        .unwrap()
        .expect("candidate after reimport");
    assert_eq!(candidate.version_count, 2);

    remove_dir(&data_dir);
    remove_dir(&root);
}

#[test]
fn reimport_preserves_existing_candidate_assignment_without_contacts() {
    let data_dir = temp_dir("manual-candidate-data");
    let root = temp_dir("manual-candidate-root");
    let resume_path = root.join("synthetic-manual-candidate.pdf");
    write_pdf_resume_without_contacts(
        &resume_path,
        "Synthetic manual merge profile",
        "Built Java services without contact fields.",
    );

    let import = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root),
        ])
        .output()
        .expect("run initial import");
    assert!(import.status.success());

    let version = searchable_versions(&data_dir)
        .pop()
        .expect("imported version");
    assert!(version.candidate_id.is_none());
    let manual_candidate_id = CandidateId::from_non_secret_parts(&["s21", "manual-candidate"]);
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    store
        .upsert_candidate(&Candidate {
            id: manual_candidate_id.clone(),
            primary_name: None,
            phone_hash: Some(ContactHash::from_keyed_digest("f".repeat(64)).unwrap()),
            email_hash: None,
            dedupe_key: None,
            merge_confidence: Some(0.93),
            version_count: 0,
        })
        .unwrap();
    store
        .assign_candidate_to_version(&version.id, &manual_candidate_id)
        .unwrap()
        .expect("manual assignment");
    drop(store);

    let reimport = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .args([
            "--data-dir",
            path_str(&data_dir),
            "import",
            "--root",
            path_str(&root),
        ])
        .output()
        .expect("run reimport");
    assert!(reimport.status.success());

    let versions = searchable_versions(&data_dir);
    assert_eq!(versions.len(), 1);
    assert_eq!(
        versions[0].candidate_id.as_ref(),
        Some(&manual_candidate_id),
        "reimport should not clear existing manual assignment"
    );

    remove_dir(&data_dir);
    remove_dir(&root);
}

fn searchable_versions(data_dir: &Path) -> Vec<ResumeVersion> {
    let store = MetaStore::open(data_dir.join("metadata.sqlite3")).unwrap();
    store.run_migrations().unwrap();
    let mut versions = Vec::new();
    for document in store.visible_documents().unwrap() {
        versions.extend(store.resume_versions_for_document(&document.id).unwrap());
    }
    versions.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    versions
}

fn write_pdf_resume(path: &Path, heading: &str, java_line: &str, email: &str, phone: &str) {
    let body = pdf_resume_body(heading, java_line, Some(email), Some(phone));
    fs::write(path, body).unwrap();
}

fn write_pdf_resume_without_contacts(path: &Path, heading: &str, java_line: &str) {
    fs::write(path, pdf_resume_body(heading, java_line, None, None)).unwrap();
}

fn pdf_resume_body(
    heading: &str,
    java_line: &str,
    email: Option<&str>,
    phone: Option<&str>,
) -> String {
    let email_line = email
        .map(|value| format!("(Email: {value}) Tj\n"))
        .unwrap_or_default();
    let phone_line = phone
        .map(|value| format!("(Phone: {value}) Tj\n"))
        .unwrap_or_default();
    format!(
        "%PDF-1.4\n1 0 obj\n<< /Type /Page >>\nstream\nBT\n({heading}) Tj\n({java_line}) Tj\n{email_line}{phone_line}ET\nendstream\nendobj\n%%EOF\n"
    )
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
