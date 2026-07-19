use std::fs;
use std::path::{Path, PathBuf};

use crate::{
    Candidate, CandidateId, ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    Document, DocumentId, DocumentStatus, FileExtension, IdentityInsertOutcome, OwnedMetaStore,
    ResumeVersion, ResumeVersionId, SourceRevision, UnixTimestamp,
};

use super::{PrivacyMaintenanceFailpoint, PRIVACY_PURGE_BATCH_LIMIT};

const PRIVATE_MARKER: &str = "SYNTHETIC_PRIVATE_PURGE_MARKER_d271";

struct TestDatabase {
    path: PathBuf,
}

impl TestDatabase {
    fn new(label: &str) -> Self {
        let mut nonce = [0_u8; 8];
        getrandom::getrandom(&mut nonce).unwrap();
        let suffix = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Self {
            path: std::env::temp_dir().join(format!("resume-ir-{label}-{suffix}")),
        }
    }
}

impl Drop for TestDatabase {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn sqlite_files(path: &Path) -> [PathBuf; 2] {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-journal", path.display())),
    ]
}

fn document(label: &str) -> Document {
    let now = UnixTimestamp::from_unix_seconds(1_900_000_000);
    Document {
        id: DocumentId::from_non_secret_parts(&["privacy-maintenance", label]),
        source_uri: format!("synthetic://privacy/{label}"),
        normalized_path: format!("synthetic/privacy/{label}.txt"),
        file_name: format!("{label}.txt"),
        extension: FileExtension::Txt,
        byte_size: 128,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::Discovered,
    }
}

fn seed_private_tombstone(store: &OwnedMetaStore, label: &str) {
    let mut document = document(label);
    let source = format!("source {PRIVATE_MARKER}");
    let revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(source.as_bytes()),
        source.len() as u64,
    );
    let text = format!("body {PRIVATE_MARKER}");
    let normalized_text_hash = ContentDigest::from_bytes(text.as_bytes());
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &revision.id,
            &normalized_text_hash,
            "parser-v1",
            "schema-v27",
        ),
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v27".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some(text.clone()),
        clean_text: Some(text),
        quality_score: Some(0.9),
    };
    document.content_hash = Some(revision.content_hash.as_str().to_string());
    document.status = DocumentStatus::FieldsExtracted;
    store.upsert_document(&document).unwrap();
    assert_eq!(
        store.insert_source_revision(&revision).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store.insert_resume_version(&version).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    document.is_deleted = true;
    document.status = DocumentStatus::Deleted;
    store.upsert_document(&document).unwrap();
}

fn compaction_pending(store: &OwnedMetaStore) -> i64 {
    store
        .connection
        .borrow()
        .query_row(
            "SELECT compaction_pending FROM privacy_maintenance_state
             WHERE state_key = 'default'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn purge_is_bounded_to_one_batch() {
    let database = TestDatabase::new("privacy-batch");
    fs::create_dir_all(&database.path).unwrap();
    let owner = match DataDirectoryOwnerLease::try_acquire(&database.path).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("test data directory was contended"),
    };
    let store = owner.open_store().unwrap();
    for index in 0..=PRIVACY_PURGE_BATCH_LIMIT {
        let mut document = document(&format!("batch-{index}"));
        document.is_deleted = true;
        document.status = DocumentStatus::Deleted;
        store.upsert_document(&document).unwrap();
    }
    let unaffected = (0..512)
        .map(|index| Candidate {
            id: CandidateId::from_non_secret_parts(&[
                "privacy-maintenance-unaffected",
                &index.to_string(),
            ]),
            primary_name: None,
            phone_hash: None,
            email_hash: None,
            dedupe_key: None,
            merge_confidence: Some(1.0),
            version_count: (index % 17 + 1) as u32,
        })
        .collect::<Vec<_>>();
    for candidate in &unaffected {
        store.upsert_candidate(candidate).unwrap();
    }

    let first = store.purge_deleted_documents().unwrap();
    assert_eq!(first.deleted_documents, PRIVACY_PURGE_BATCH_LIMIT);
    assert_eq!(first.remaining_tombstones, 1);
    assert_eq!(compaction_pending(&store), 0);
    for expected in &unaffected {
        assert_eq!(
            store.candidate_by_id(&expected.id).unwrap(),
            Some(expected.clone())
        );
    }
    let second = store.purge_deleted_documents().unwrap();
    assert_eq!(second.deleted_documents, 1);
    assert_eq!(second.remaining_tombstones, 0);
    assert_eq!(compaction_pending(&store), 0);
}

#[test]
fn durable_receipt_resumes_vacuum_after_each_failpoint() {
    for failpoint in [
        PrivacyMaintenanceFailpoint::AfterDeleteCommit,
        PrivacyMaintenanceFailpoint::AfterVacuum,
    ] {
        let database = TestDatabase::new(&format!("privacy-resume-{failpoint:?}"));
        fs::create_dir_all(&database.path).unwrap();
        let owner = match DataDirectoryOwnerLease::try_acquire(&database.path).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => {
                panic!("test data directory was contended")
            }
        };
        let store = owner.open_store().unwrap();
        let store_path = crate::metadata_store_path(&database.path).unwrap();
        let secure_delete = store
            .connection
            .borrow()
            .query_row("PRAGMA secure_delete", [], |row| row.get::<_, i64>(0))
            .unwrap();
        assert_eq!(secure_delete, 1);
        seed_private_tombstone(&store, "resume");

        assert!(store.purge_deleted_documents_inner(failpoint).is_err());
        assert_eq!(compaction_pending(&store), 1);
        drop(store);

        let reopened = owner.open_store().unwrap();
        assert_eq!(compaction_pending(&reopened), 0);
        drop(reopened);
        for path in sqlite_files(&store_path) {
            if let Ok(bytes) = fs::read(&path) {
                assert!(
                    !bytes
                        .windows(PRIVATE_MARKER.len())
                        .any(|window| window == PRIVATE_MARKER.as_bytes()),
                    "private marker remained in {} after {failpoint:?}",
                    path.display()
                );
            }
        }
    }
}
