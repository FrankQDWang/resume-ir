use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    Candidate, CandidateId, ContactHash, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    EntityType, OwnedMetaStore, ReadMetaStore, ResumeVersion,
};

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
    assert!(!key_material.contains("415-555-0132"));
    assert!(!key_material.contains("+14155550132"));
    #[cfg(unix)]
    assert_eq!(key_mode(&key_path) & 0o777, 0o600);

    let versions = searchable_versions(&data_dir);
    assert_eq!(versions.len(), 2);
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let first_candidate_id = store
        .candidate_assignment_for_version(&versions[0].id)
        .unwrap()
        .expect("first version candidate");
    assert!(
        versions.iter().all(|version| store
            .candidate_assignment_for_version(&version.id)
            .unwrap()
            .as_ref()
            == Some(&first_candidate_id)),
        "same hashed contact should assign one candidate"
    );

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

    for contact_query in [
        "Shared.Candidate@Example.Test",
        "shared.candidate@example.test",
        "(415) 555-0132",
        "+14155550132",
    ] {
        let search = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
            .args(["--data-dir", path_str(&data_dir), "search", contact_query])
            .output()
            .expect("run resume-cli contact search");
        assert!(search.status.success());
        assert!(search.stderr.is_empty());
        let stdout = String::from_utf8_lossy(&search.stdout);
        assert!(stdout.contains("results: 0"));
        assert!(!stdout.contains("Shared.Candidate"));
        assert!(!stdout.contains("shared.candidate@example.test"));
        assert!(!stdout.contains("415-555-0132"));
        assert!(!stdout.contains("+14155550132"));
    }
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
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert!(versions_after_reimport.iter().all(|version| {
        store
            .candidate_assignment_for_version(&version.id)
            .unwrap()
            .as_ref()
            == Some(&first_candidate_id)
    }));
    let candidate = store
        .candidate_by_id(&first_candidate_id)
        .unwrap()
        .expect("candidate after reimport");
    assert_eq!(candidate.version_count, 2);

    remove_dir(&data_dir);
    remove_dir(&root);
}

#[test]
fn sealed_resume_version_rejects_a_late_candidate_assignment() {
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
    let store = create_owned_store(&data_dir);
    assert!(store
        .candidate_assignment_for_version(&version.id)
        .unwrap()
        .is_none());
    let manual_candidate_id = CandidateId::from_non_secret_parts(&["s21", "manual-candidate"]);
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
    assert!(store
        .insert_candidate_assignment(&version.id, &manual_candidate_id)
        .is_err());
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
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert!(store
        .candidate_assignment_for_version(&versions[0].id)
        .unwrap()
        .is_none());

    remove_dir(&data_dir);
    remove_dir(&root);
}

fn searchable_versions(data_dir: &Path) -> Vec<ResumeVersion> {
    let store = ReadMetaStore::open_data_dir(data_dir).unwrap();
    let mut versions = Vec::new();
    for document in store.visible_documents().unwrap() {
        versions.extend(store.resume_versions_for_document(&document.id).unwrap());
    }
    versions.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    versions
}

fn create_owned_store(data_dir: &Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test store owner contended"),
    };
    owner.open_store().unwrap()
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
) -> Vec<u8> {
    let mut content = String::from("BT\n/F1 12 Tf\n72 720 Td\n");
    for line in [
        heading.to_string(),
        "SUMMARY".to_string(),
        "Synthetic software profile.".to_string(),
        "EXPERIENCE".to_string(),
        "Built deterministic local search services.".to_string(),
        java_line.to_string(),
        "SKILLS".to_string(),
        "Java Rust Search".to_string(),
        email
            .map(|value| format!("Email: {value}"))
            .unwrap_or_default(),
        phone
            .map(|value| format!("Phone: {value}"))
            .unwrap_or_default(),
    ] {
        if line.is_empty() {
            continue;
        }
        if content.ends_with("Td\n") {
            content.push_str(&format!("({line}) Tj\n"));
        } else {
            content.push_str(&format!("T* ({line}) Tj\n"));
        }
    }
    content.push_str("ET\n");
    simple_font_text_pdf_bytes(content.as_bytes())
}

fn simple_font_text_pdf_bytes(content: &[u8]) -> Vec<u8> {
    build_valid_pdf(vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
            content.to_vec(),
            b"endstream".to_vec(),
        ]
        .concat(),
    ])
}

fn build_valid_pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());

    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        if !object.ends_with(b"\n") {
            pdf.push(b'\n');
        }
        pdf.extend_from_slice(b"endobj\n");
    }

    let xref_offset = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    pdf.extend_from_slice(b"0000000000 65535 f\r\n");
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{}\n%%EOF",
            objects.len() + 1,
            xref_offset
        )
        .as_bytes(),
    );

    pdf
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
