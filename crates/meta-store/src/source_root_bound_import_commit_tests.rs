use super::*;
use crate::{
    ClassificationStatus, ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    DocumentStatus, FileExtension, ImportProcessingContract, ImportRootKind, ImportScanProfile,
    ImportScanScope, ImportTask, ImportTaskStatus, ReasonCode, ResumeVersion,
    ResumeVersionClassification, ReviewDisposition, ScanTrigger, SourceRevision,
    SourceRevisionTriage, StrongSourceFileObservation, CLASSIFIER_EPOCH,
};

const NOW: i64 = 1_900_500_000;

#[test]
fn classified_commit_persists_one_bound_operation_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), "reopen");
    let facts = fixture.facts("reopen");
    fixture.commit_classified(&facts, true).unwrap();
    fixture
        .store
        .commit_source_root_bound_import(SourceRootBoundImportCommit {
            task_id: &fixture.task.id,
            normalized_path: &fixture.normalized_path,
            observed_at: fixture.now,
            mutation: SourceRootBoundImportMutation::ExistingRevision {
                document_id: &facts.document.id,
                source_revision_id: &facts.source_revision.id,
            },
            observation: SourceRootBoundImportObservation::MetadataOnly,
        })
        .unwrap();
    let mut changed_document = facts.document.clone();
    changed_document.file_name = "renamed-source.txt".to_string();
    changed_document.updated_at = UnixTimestamp::from_unix_seconds(NOW + 1);
    fixture
        .store
        .commit_source_root_bound_import(SourceRootBoundImportCommit {
            task_id: &fixture.task.id,
            normalized_path: &fixture.normalized_path,
            observed_at: fixture.now,
            mutation: SourceRootBoundImportMutation::ExistingRevisionMetadata {
                document: &changed_document,
                source_revision_id: &facts.source_revision.id,
            },
            observation: SourceRootBoundImportObservation::MetadataOnly,
        })
        .unwrap();
    let task_id = fixture.task.id.clone();
    drop(fixture);

    let store = open_store(directory.path());
    assert_eq!(
        store.document_by_id(&changed_document.id).unwrap(),
        Some(changed_document)
    );
    assert!(store
        .source_file_observation_for_import_task(&task_id, "/synthetic/reopen/source.txt")
        .unwrap()
        .is_some());
    assert_eq!(count(&store, "source_occurrence"), 1);
    assert_eq!(count(&store, "resume_version"), 1);
}

#[test]
fn every_transaction_step_failure_rolls_back_private_rows() {
    for (label, table, ocr) in [
        ("stage", "source_revision", false),
        ("occurrence", "source_occurrence", false),
        ("observation", "source_file_observation", false),
        ("ocr", "ingest_job", true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let fixture = Fixture::new(directory.path(), label);
        fixture
            .store
            .connection
            .borrow()
            .execute_batch(&format!(
                "CREATE TEMP TRIGGER fail_{label} BEFORE INSERT ON {table}
             BEGIN SELECT RAISE(ABORT, 'synthetic rollback'); END;"
            ))
            .unwrap();
        let facts = fixture.facts(label);
        let observation = facts.observation();
        let triage = SourceRevisionTriage {
            source_revision_id: facts.source_revision.id.clone(),
            status: ClassificationStatus::OcrBacklog,
            triage_epoch: fixture.contract.source_triage_epoch().as_str().to_string(),
            reason_codes: vec![ReasonCode::OcrRequired],
            triaged_at: fixture.now,
        };
        let mutation = if ocr {
            SourceRootBoundImportMutation::OcrRequired(ImmutableIngestStage::SourceTriage {
                document: &facts.document,
                source_revision: &facts.source_revision,
                triage: &triage,
            })
        } else {
            facts.classified_mutation()
        };
        assert!(fixture
            .store
            .commit_source_root_bound_import(SourceRootBoundImportCommit {
                task_id: &fixture.task.id,
                normalized_path: &fixture.normalized_path,
                observed_at: fixture.now,
                mutation,
                observation: SourceRootBoundImportObservation::Strong(&observation),
            },)
            .is_err());
        fixture.assert_no_private_rows(label);
    }
}

#[test]
fn deletion_and_unknown_phase_fail_before_any_import_write() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), "deleting");
    let facts = fixture.facts("deleting");
    fixture
        .store
        .begin_source_root_deletion(&fixture.root.id, fixture.now)
        .unwrap();
    assert!(fixture.commit_classified(&facts, false).is_err());
    fixture.assert_no_private_rows("deleting");

    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), "missing-binding");
    let facts = fixture.facts("missing-binding");
    let missing_task = ImportTaskId::from_non_secret_parts(&["missing-root-bound-task"]);
    assert!(fixture
        .store
        .commit_source_root_bound_import(SourceRootBoundImportCommit {
            task_id: &missing_task,
            normalized_path: &fixture.normalized_path,
            observed_at: fixture.now,
            mutation: facts.classified_mutation(),
            observation: SourceRootBoundImportObservation::MetadataOnly,
        })
        .is_err());
    assert!(fixture
        .store
        .commit_source_root_bound_import(SourceRootBoundImportCommit {
            task_id: &fixture.task.id,
            normalized_path: "/synthetic/different-root/source.txt",
            observed_at: fixture.now,
            mutation: facts.classified_mutation(),
            observation: SourceRootBoundImportObservation::MetadataOnly,
        })
        .is_err());
    fixture.assert_no_private_rows("missing-or-mismatched-binding");

    fixture
        .store
        .begin_source_root_deletion(&fixture.root.id, fixture.now)
        .unwrap();
    fixture
        .store
        .connection
        .borrow()
        .execute(
            "UPDATE source_root_deletion SET phase = 'failed' WHERE root_id = ?1",
            [&fixture.root.id.as_str()],
        )
        .unwrap();
    assert!(fixture.commit_classified(&facts, false).is_err());
    fixture.assert_no_private_rows("stale-terminal-binding");

    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), "unknown");
    fixture
        .store
        .connection
        .borrow()
        .execute_batch(
            "PRAGMA ignore_check_constraints = ON;
         INSERT INTO source_root_deletion (
            root_id, canonical_path, phase, affected_documents, removed_documents,
            started_at_seconds, updated_at_seconds, checkpoint_protocol_version
         ) SELECT id, canonical_path, 'future_phase', 0, 0, 1900500000, 1900500000, 2
           FROM source_root;
         PRAGMA ignore_check_constraints = OFF;",
        )
        .unwrap();
    let facts = fixture.facts("unknown");
    assert!(fixture.commit_classified(&facts, false).is_err());
    fixture.assert_no_private_rows("unknown");
}

#[test]
fn read_failure_writes_nothing_and_a_sibling_root_remains_independent() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), "read-failure");
    fixture
        .store
        .commit_source_root_bound_import(SourceRootBoundImportCommit {
            task_id: &fixture.task.id,
            normalized_path: &fixture.normalized_path,
            observed_at: fixture.now,
            mutation: SourceRootBoundImportMutation::ReadFailureWithoutRevision,
            observation: SourceRootBoundImportObservation::MetadataOnly,
        })
        .unwrap();
    fixture.assert_no_private_rows("read-failure");

    let sibling = start_root(&fixture.store, &fixture.contract, "sibling", fixture.now);
    fixture
        .store
        .begin_source_root_deletion(&fixture.root.id, fixture.now)
        .unwrap();
    let facts = Facts::new("sibling", &sibling.normalized_path, fixture.now);
    fixture
        .store
        .commit_source_root_bound_import(SourceRootBoundImportCommit {
            task_id: &sibling.task.id,
            normalized_path: &sibling.normalized_path,
            observed_at: fixture.now,
            mutation: facts.classified_mutation(),
            observation: SourceRootBoundImportObservation::MetadataOnly,
        })
        .unwrap();
    assert_eq!(count(&fixture.store, "document"), 1);
}

struct Fixture {
    store: OwnedMetaStore,
    root: crate::SourceRoot,
    task: ImportTask,
    contract: ImportProcessingContract,
    normalized_path: String,
    now: UnixTimestamp,
}

impl Fixture {
    fn new(data_dir: &std::path::Path, label: &str) -> Self {
        let store = open_store(data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(NOW);
        let contract = ImportProcessingContract::new(
            "root-bound-parser",
            "root-bound-ocr",
            "root-bound-derived",
            CLASSIFIER_EPOCH,
        )
        .unwrap();
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap();
        let started = start_root(&store, &contract, label, now);
        Self {
            store,
            root: started.root,
            task: started.task,
            contract,
            normalized_path: started.normalized_path,
            now,
        }
    }

    fn facts(&self, label: &str) -> Facts {
        Facts::new(label, &self.normalized_path, self.now)
    }

    fn commit_classified(&self, facts: &Facts, strong: bool) -> Result<()> {
        let observation = facts.observation();
        self.store
            .commit_source_root_bound_import(SourceRootBoundImportCommit {
                task_id: &self.task.id,
                normalized_path: &self.normalized_path,
                observed_at: self.now,
                mutation: facts.classified_mutation(),
                observation: if strong {
                    SourceRootBoundImportObservation::Strong(&observation)
                } else {
                    SourceRootBoundImportObservation::MetadataOnly
                },
            })
            .map(|_| ())
    }

    fn assert_no_private_rows(&self, label: &str) {
        for table in [
            "document",
            "source_revision",
            "resume_version",
            "resume_version_classification",
            "source_revision_triage",
            "source_occurrence",
            "source_occurrence_revision",
            "source_file_observation",
            "ocr_job_spec",
            "ingest_job",
        ] {
            assert_eq!(count(&self.store, table), 0, "{label} left {table}");
        }
    }
}

struct StartedRoot {
    root: crate::SourceRoot,
    task: ImportTask,
    normalized_path: String,
}

struct Facts {
    document: Document,
    source_revision: SourceRevision,
    version: ResumeVersion,
    classification: ResumeVersionClassification,
}

impl Facts {
    fn new(label: &str, normalized_path: &str, now: UnixTimestamp) -> Self {
        let source = format!("synthetic source {label}");
        let mut document = Document {
            id: DocumentId::from_non_secret_parts(&["root-bound", label]),
            source_uri: format!("file://{normalized_path}"),
            normalized_path: normalized_path.to_string(),
            file_name: "source.txt".to_string(),
            extension: FileExtension::Txt,
            byte_size: source.len() as u64,
            mtime: now,
            content_hash: None,
            text_hash: None,
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::FieldsExtracted,
        };
        let source_revision = SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(source.as_bytes()),
            source.len() as u64,
        );
        document.content_hash = Some(source_revision.content_hash.as_str().to_string());
        let clean_text = format!("synthetic normalized {label}");
        let normalized_text_hash = ContentDigest::from_bytes(clean_text.as_bytes());
        let version = ResumeVersion {
            id: crate::ResumeVersionId::from_content_identity(
                &document.id,
                &source_revision.id,
                &normalized_text_hash,
                "root-bound-parser",
                "root-bound-derived",
            ),
            document_id: document.id.clone(),
            source_revision_id: source_revision.id.clone(),
            normalized_text_hash,
            parse_version: "root-bound-parser".to_string(),
            schema_version: "root-bound-derived".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: None,
            clean_text: Some(clean_text),
            quality_score: Some(0.9),
        };
        let classification = ResumeVersionClassification {
            resume_version_id: version.id.clone(),
            status: ClassificationStatus::ResumeCandidate,
            classifier_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
            classified_at: now,
            review_disposition: ReviewDisposition::NotRequired,
        };
        Self {
            document,
            source_revision,
            version,
            classification,
        }
    }

    fn classified_mutation(&self) -> SourceRootBoundImportMutation<'_> {
        SourceRootBoundImportMutation::Immutable(ImmutableIngestStage::ClassifiedResume {
            document: &self.document,
            source_revision: &self.source_revision,
            version: &self.version,
            classification: &self.classification,
            mentions: &[],
            email_hash: None,
            phone_hash: None,
        })
    }

    fn observation(&self) -> StrongSourceFileObservation {
        StrongSourceFileObservation {
            source_revision_id: self.source_revision.id.clone(),
            stable_file_id: format!("sfi_{}", "1".repeat(32)),
            byte_size: self.source_revision.byte_size,
            mtime_seconds: NOW,
            mtime_nanoseconds: 1,
            ctime_seconds: NOW,
            ctime_nanoseconds: 2,
            strongly_verified_at: UnixTimestamp::from_unix_seconds(NOW),
            next_strong_verification_at: UnixTimestamp::from_unix_seconds(NOW + 60),
        }
    }
}

fn start_root(
    store: &OwnedMetaStore,
    contract: &ImportProcessingContract,
    label: &str,
    now: UnixTimestamp,
) -> StartedRoot {
    let root_path = format!("/synthetic/{label}");
    let root = store
        .register_source_root(&root_path, &root_path, label, now)
        .unwrap();
    let queued = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["root-bound", label]),
        root_path: root_path.clone(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
        import_task_id: queued.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: root_path.clone(),
        canonical_root_path: root_path.clone(),
        files_discovered: 1,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: None,
        scan_budget_limit: None,
        scan_budget_observed: None,
        scan_budget_exhausted: false,
        updated_at: now,
    };
    store
        .begin_scan(&root.id, queued.id.as_str(), ScanTrigger::Manual, now)
        .unwrap();
    store
        .insert_import_task_with_scan_scope(&queued, &scope, contract)
        .unwrap();
    StartedRoot {
        root,
        task: queued,
        normalized_path: format!("{root_path}/source.txt"),
    }
}

fn open_store(data_dir: &std::path::Path) -> OwnedMetaStore {
    match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner.open_store().unwrap(),
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic store contended"),
    }
}

fn count(store: &OwnedMetaStore, table: &str) -> i64 {
    store
        .connection
        .borrow()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}
