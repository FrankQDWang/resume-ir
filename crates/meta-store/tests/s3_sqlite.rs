use std::fs;
use std::ops::Range;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    ActiveSearchProjection, Candidate, CandidateId, ClassificationStatus, ContactHash,
    ContentDigest, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, Document, DocumentId,
    DocumentStatus, EntityMention, EntityMentionId, EntityType, EphemeralMetaStore, FileExtension,
    FullTextSnapshotDescriptor, IdentityInsertOutcome, ImportRootKind, ImportRootPreset,
    ImportScanBudgetKind, ImportScanError, ImportScanErrorKind, ImportScanErrorOperation,
    ImportScanErrorSummary, ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId,
    ImportTaskStatus, IndexStateStatus, IngestJob, IngestJobId, IngestJobKind, IngestJobStatus,
    MetaStoreErrorClass, MetadataEncryptionState, MigrationRebuildBarrierToken, OcrPageCacheEntry,
    OcrPageCacheKey, OcrPageCacheStatus, OcrWordBox, OwnedMetaStore, ReadMetaStore, ReasonCode,
    ResumeVersion, ResumeVersionClassification, ResumeVersionId, ReviewDisposition,
    SearchProjectionDigest, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationOutcome, SearchPublicationSession, SearchPublicationState,
    SearchPublicationValidation, SearchRepairReason, SourceRevision, TerminalDocumentUpdate,
    UnixTimestamp, VectorSnapshotDescriptor, WorkerTaskControl, WorkerTaskKind, CLASSIFIER_EPOCH,
};
mod support;

#[test]
fn migrations_are_idempotent_and_schema_v28_is_queryable() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();

    assert!(store.foreign_keys_enabled().unwrap());

    let first = store.run_migrations().unwrap();
    assert_eq!(
        first.applied_versions(),
        &[
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28,
        ]
    );
    assert_eq!(store.schema_version().unwrap(), 28);

    for table_name in [
        "candidate",
        "document",
        "resume_version",
        "ingest_job",
        "source_revision",
        "source_revision_triage",
        "resume_version_classification",
        "active_search_projection",
        "search_projection_state",
        "search_publication_journal",
        "search_publication_commit_guard",
        "authorized_import_root",
        "privacy_maintenance_state",
        "import_task",
        "entity_mention",
        "ocr_page_cache",
        "worker_task_control",
        "import_scan_scope",
        "import_scan_error",
        "embedding_job_spec",
        "import_task_cancellation",
        "query_observation",
        "candidate_contact_conflict",
        "import_processing_contract",
        "migration_rebuild_contract_state",
        "import_task_contract_binding",
        "import_task_source_disposition",
        "import_task_completion",
    ] {
        assert!(store.schema_table_exists(table_name).unwrap());
    }

    let second = store.run_migrations().unwrap();
    assert!(second.applied_versions().is_empty());
    assert_eq!(store.schema_version().unwrap(), 28);
}

#[test]
fn metadata_encryption_state_reports_plaintext_until_sqlcipher_is_enabled() {
    let store = migrated_store();

    assert_eq!(
        store.metadata_encryption_state(),
        MetadataEncryptionState::Plaintext
    );
    assert_eq!(store.metadata_encryption_state().label(), "plaintext");
}

#[test]
fn owner_created_metadata_store_survives_read_reopen_without_plaintext_header() {
    let data_dir = temp_data_dir("encrypted-metadata-store");
    let document = document(
        "encrypted-store-document",
        false,
        DocumentStatus::Searchable,
    );

    {
        let store = open_owned_store(&data_dir);
        assert_eq!(
            store.metadata_encryption_state(),
            MetadataEncryptionState::SqlCipher
        );
        assert_eq!(store.metadata_encryption_state().label(), "sqlcipher");
        store.upsert_document(&document).unwrap();
        assert_eq!(store.schema_version().unwrap(), 28);
    }

    let db_path = meta_store::metadata_store_path(&data_dir).unwrap();
    let encrypted_bytes = fs::read(&db_path).unwrap();
    assert!(!encrypted_bytes.starts_with(b"SQLite format 3"));
    assert!(!encrypted_bytes
        .windows(b"encrypted-store-document".len())
        .any(|window| window == b"encrypted-store-document"));

    let reopened = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    assert_eq!(
        reopened.metadata_encryption_state(),
        MetadataEncryptionState::SqlCipher
    );
    assert_eq!(
        reopened.document_by_id(&document.id).unwrap().unwrap().id,
        document.id
    );

    drop(reopened);
    remove_temp_dir(&data_dir);
}

#[test]
fn read_open_rejects_an_unpublished_plaintext_database() {
    let data_dir = temp_data_dir("unowned-plaintext-v28");
    let db_path = data_dir.join("unpublished.sqlite3");

    fs::write(&db_path, b"SQLite format 3\0synthetic unpublished fixture").unwrap();

    set_owner_only_file_permissions(&db_path);

    let plaintext_bytes = fs::read(&db_path).unwrap();
    assert!(plaintext_bytes.starts_with(b"SQLite format 3"));

    let error = ReadMetaStore::open_data_dir(&data_dir).unwrap_err();
    assert_eq!(
        error.class(),
        MetaStoreErrorClass::MigrationOwnershipRequired
    );
    assert!(db_path.exists());

    remove_temp_dir(&data_dir);
}

#[test]
fn worker_task_control_defaults_to_running_and_persists_pause_state() {
    let data_dir = temp_data_dir("worker-task-control-placeholder");
    let pause_at = UnixTimestamp::from_unix_seconds(1_800_000_330);
    let resume_at = UnixTimestamp::from_unix_seconds(1_800_000_360);

    {
        let store = open_owned_store(&data_dir);
        assert_eq!(
            store.worker_task_control(WorkerTaskKind::Ocr).unwrap(),
            WorkerTaskControl {
                task: WorkerTaskKind::Ocr,
                paused: false,
                updated_at: UnixTimestamp::from_unix_seconds(0),
            }
        );

        store
            .set_worker_task_paused(WorkerTaskKind::Ocr, true, pause_at)
            .unwrap();
        assert_eq!(
            store.worker_task_control(WorkerTaskKind::Ocr).unwrap(),
            WorkerTaskControl {
                task: WorkerTaskKind::Ocr,
                paused: true,
                updated_at: pause_at,
            }
        );
    }

    {
        let reopened = open_owned_store(&data_dir);
        assert_eq!(
            reopened.worker_task_control(WorkerTaskKind::Ocr).unwrap(),
            WorkerTaskControl {
                task: WorkerTaskKind::Ocr,
                paused: true,
                updated_at: pause_at,
            }
        );

        reopened
            .set_worker_task_paused(WorkerTaskKind::Ocr, false, resume_at)
            .unwrap();
        assert_eq!(
            reopened.worker_task_control(WorkerTaskKind::Ocr).unwrap(),
            WorkerTaskControl {
                task: WorkerTaskKind::Ocr,
                paused: false,
                updated_at: resume_at,
            }
        );
    }

    remove_temp_dir(&data_dir);
}

#[test]
fn import_scan_scope_persists_root_profile_and_redacted_progress_counts() {
    let data_dir = temp_data_dir("import-scan-scope-placeholder");
    let task = import_task(
        "scan-scope-task",
        "/private/root/Documents",
        ImportTaskStatus::Queued,
    );
    let queued_at = UnixTimestamp::from_unix_seconds(1_800_000_010);
    let updated_at = UnixTimestamp::from_unix_seconds(1_800_000_020);
    let initial_scope = ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Preset,
        root_preset: Some(ImportRootPreset::LocalDiscovery),
        scan_profile: ImportScanProfile::Discovery,
        requested_root_path: "/private/root".to_string(),
        canonical_root_path: "/private/root/Documents".to_string(),
        files_discovered: 0,
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
        updated_at: queued_at,
    };
    let completed_scope = ImportScanScope {
        files_discovered: 7,
        ignored_entries: 2,
        scan_errors: 1,
        searchable_documents: 4,
        ocr_required_documents: 1,
        ocr_jobs_queued: 1,
        failed_documents: 1,
        deleted_documents: 1,
        scan_budget_kind: Some(ImportScanBudgetKind::Files),
        scan_budget_limit: Some(10),
        scan_budget_observed: Some(10),
        scan_budget_exhausted: true,
        updated_at,
        ..initial_scope.clone()
    };
    let budgeted_not_exhausted_scope = ImportScanScope {
        files_discovered: 7,
        scan_budget_kind: Some(ImportScanBudgetKind::Files),
        scan_budget_limit: Some(10),
        scan_budget_observed: Some(7),
        scan_budget_exhausted: false,
        updated_at,
        ..initial_scope.clone()
    };

    {
        let store = open_owned_store(&data_dir);
        support::insert_import_task_owned(&store, &task);

        store.upsert_import_scan_scope(&initial_scope).unwrap();
        assert_eq!(
            store.import_scan_scope_by_task_id(&task.id).unwrap(),
            Some(initial_scope.clone())
        );

        store
            .upsert_import_scan_scope(&budgeted_not_exhausted_scope)
            .unwrap();
        assert_eq!(
            store.import_scan_scope_by_task_id(&task.id).unwrap(),
            Some(budgeted_not_exhausted_scope)
        );

        store.upsert_import_scan_scope(&completed_scope).unwrap();
        assert_eq!(store.status_summary().unwrap().import_scan_scopes, 1);
    }

    {
        let reopened = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let persisted = reopened
            .latest_import_scan_scope()
            .unwrap()
            .expect("latest import scan scope");

        assert_eq!(persisted, completed_scope);
        let debug = format!("{persisted:?}");
        assert!(!debug.contains("/private/root"));
        assert!(debug.contains("files_discovered"));
    }

    remove_temp_dir(&data_dir);
}

#[test]
fn import_task_and_scan_scope_insert_atomically_for_daemon_command_ipc() {
    let store = migrated_store();
    let task = import_task(
        "atomic-command-import",
        "/private/root/Documents",
        ImportTaskStatus::Queued,
    );
    let scope = ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: "/private/root".to_string(),
        canonical_root_path: "/private/root/Documents".to_string(),
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: Some(ImportScanBudgetKind::Files),
        scan_budget_limit: Some(10),
        scan_budget_observed: Some(0),
        scan_budget_exhausted: false,
        updated_at: UnixTimestamp::from_unix_seconds(1_800_000_020),
    };

    support::insert_import_task_with_scan_scope(&store, &task, &scope);

    assert_eq!(store.import_task_by_id(&task.id).unwrap(), Some(task));
    assert_eq!(
        store
            .import_scan_scope_by_task_id(&scope.import_task_id)
            .unwrap(),
        Some(scope)
    );
}

#[test]
fn import_scan_errors_replace_and_query_without_exposing_path_digest() {
    let store = migrated_store();
    let task = import_task(
        "scan-error-task",
        "/private/root/Documents",
        ImportTaskStatus::Queued,
    );
    let updated_at = UnixTimestamp::from_unix_seconds(1_800_000_120);
    let first_errors = vec![
        ImportScanError {
            import_task_id: task.id.clone(),
            error_index: 0,
            kind: ImportScanErrorKind::PermissionDenied,
            operation: ImportScanErrorOperation::ReadDirectory,
            path_digest: Some("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            updated_at,
        },
        ImportScanError {
            import_task_id: task.id.clone(),
            error_index: 1,
            kind: ImportScanErrorKind::LockedOrUnreadable,
            operation: ImportScanErrorOperation::Fingerprint,
            path_digest: Some("sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
            updated_at,
        },
    ];
    let replacement = vec![ImportScanError {
        import_task_id: task.id.clone(),
        error_index: 0,
        kind: ImportScanErrorKind::Io,
        operation: ImportScanErrorOperation::NormalizePath,
        path_digest: None,
        updated_at,
    }];

    support::insert_import_task(&store, &task);

    store
        .replace_import_scan_errors(&task.id, &first_errors)
        .unwrap();
    assert_eq!(
        store.import_scan_errors_for_task(&task.id).unwrap(),
        first_errors
    );
    assert_eq!(store.status_summary().unwrap().import_scan_errors, 2);
    assert_eq!(
        store.import_scan_error_breakdown().unwrap(),
        vec![
            ImportScanErrorSummary {
                kind: ImportScanErrorKind::PermissionDenied,
                operation: ImportScanErrorOperation::ReadDirectory,
                count: 1,
            },
            ImportScanErrorSummary {
                kind: ImportScanErrorKind::LockedOrUnreadable,
                operation: ImportScanErrorOperation::Fingerprint,
                count: 1,
            },
        ]
    );

    let debug = format!(
        "{:?}",
        store.import_scan_errors_for_task(&task.id).unwrap()[0]
    );
    assert!(debug.contains("path_digest"));
    assert!(!debug.contains("aaaaaaaa"));
    assert!(!debug.contains("/private/root"));

    store
        .replace_import_scan_errors(&task.id, &replacement)
        .unwrap();
    assert_eq!(
        store.import_scan_errors_for_task(&task.id).unwrap(),
        replacement
    );
    assert_eq!(store.status_summary().unwrap().import_scan_errors, 1);
    assert_eq!(
        store.import_scan_error_breakdown().unwrap(),
        vec![ImportScanErrorSummary {
            kind: ImportScanErrorKind::Io,
            operation: ImportScanErrorOperation::NormalizePath,
            count: 1,
        }]
    );
}

#[test]
fn candidates_persist_and_are_found_only_by_hashed_contact_material() {
    let store = migrated_store();
    let email_hash = contact_hash('a');
    let phone_hash = contact_hash('b');
    let candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["s19", "candidate-persist"]),
        primary_name: Some("Synthetic Candidate".to_string()),
        phone_hash: Some(phone_hash.clone()),
        email_hash: Some(email_hash.clone()),
        dedupe_key: Some("synthetic-key".to_string()),
        merge_confidence: Some(0.97),
        version_count: 2,
    };

    store.upsert_candidate(&candidate).unwrap();

    assert_eq!(
        store.candidate_by_id(&candidate.id).unwrap(),
        Some(candidate.clone())
    );
    assert_eq!(
        store
            .candidate_by_contact_hash(&email_hash)
            .unwrap()
            .map(|candidate| candidate.id),
        Some(candidate.id.clone())
    );
    assert_eq!(
        store
            .candidate_by_contact_hash(&phone_hash)
            .unwrap()
            .map(|candidate| candidate.id),
        Some(candidate.id.clone())
    );
    assert_eq!(
        store.candidate_by_contact_hash(&contact_hash('c')).unwrap(),
        None
    );

    let debug = format!("{candidate:?}");
    assert!(!debug.contains(email_hash.as_str()));
    assert!(!debug.contains(phone_hash.as_str()));
    assert!(!debug.contains("Synthetic Candidate"));
    assert!(!debug.contains("synthetic-key"));
}

#[test]
fn searchable_document_ids_with_contact_hashes_matches_active_projection_only() {
    let (_directory, store) = support::owned_store();
    let email_hash = contact_hash('a');
    let phone_hash = contact_hash('b');
    let deleted_hash = contact_hash('c');
    let hidden_hash = contact_hash('d');
    let failed_hash = contact_hash('e');
    let other_hash = contact_hash('f');
    let visible_doc = document("contact-visible", false, DocumentStatus::Searchable);
    let visible_version = resume_version("contact-visible-version", visible_doc.id.clone());
    let deleted_doc = document("contact-deleted", true, DocumentStatus::Searchable);
    let deleted_version = resume_version("contact-deleted-version", deleted_doc.id.clone());
    let hidden_doc = document("contact-hidden", false, DocumentStatus::Searchable);
    let hidden_version = resume_version("contact-hidden-version", hidden_doc.id.clone());
    let partial_doc = document("contact-partial", false, DocumentStatus::IndexedPartial);
    let partial_version = resume_version("contact-partial-version", partial_doc.id.clone());
    let failed_doc = document("contact-failed", false, DocumentStatus::FailedPermanent);
    let failed_version = resume_version("contact-failed-version", failed_doc.id.clone());

    for document in [
        visible_doc.clone(),
        deleted_doc.clone(),
        hidden_doc.clone(),
        partial_doc.clone(),
        failed_doc.clone(),
    ] {
        store.upsert_document(&document).unwrap();
    }
    for version in [
        visible_version.clone(),
        deleted_version.clone(),
        hidden_version.clone(),
        partial_version.clone(),
        failed_version.clone(),
    ] {
        insert_resume_version_owned(&store, &version);
    }

    let visible_candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["contact-visible-candidate"]),
        primary_name: None,
        phone_hash: None,
        email_hash: Some(email_hash.clone()),
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    let partial_candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["contact-partial-candidate"]),
        primary_name: None,
        phone_hash: Some(phone_hash.clone()),
        email_hash: None,
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    let deleted_candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["contact-deleted-candidate"]),
        primary_name: None,
        phone_hash: None,
        email_hash: Some(deleted_hash.clone()),
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    let hidden_candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["contact-hidden-candidate"]),
        primary_name: None,
        phone_hash: Some(hidden_hash.clone()),
        email_hash: None,
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    let failed_candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["contact-failed-candidate"]),
        primary_name: None,
        phone_hash: Some(failed_hash.clone()),
        email_hash: None,
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    for candidate in [
        visible_candidate.clone(),
        partial_candidate.clone(),
        deleted_candidate.clone(),
        hidden_candidate.clone(),
        failed_candidate.clone(),
    ] {
        store.upsert_candidate(&candidate).unwrap();
    }
    for (version, candidate) in [
        (&visible_version, &visible_candidate),
        (&partial_version, &partial_candidate),
        (&deleted_version, &deleted_candidate),
        (&hidden_version, &hidden_candidate),
        (&failed_version, &failed_candidate),
    ] {
        assert_eq!(
            store
                .insert_candidate_assignment(&version.id, &candidate.id)
                .unwrap(),
            IdentityInsertOutcome::Inserted
        );
    }
    publish_active_versions(&store, &[&visible_version, &partial_version]);

    let matches = store
        .searchable_document_ids_with_contact_hashes(&[
            email_hash,
            phone_hash.clone(),
            deleted_hash,
            hidden_hash,
            failed_hash,
            other_hash,
        ])
        .unwrap();
    let mut expected = vec![visible_doc.id, partial_doc.id.clone()];
    expected.sort();
    assert_eq!(matches, expected);

    let phone_matches = store
        .searchable_document_ids_with_contact_hashes(&[phone_hash])
        .unwrap();
    assert_eq!(phone_matches, vec![partial_doc.id]);
}

#[test]
fn candidate_contact_hash_indexes_are_unique_and_canonicalized() {
    let store = migrated_store();
    let lowercase_hash = ContactHash::from_keyed_digest("e".repeat(64)).unwrap();
    let uppercase_hash = ContactHash::from_keyed_digest("E".repeat(64)).unwrap();
    assert_eq!(uppercase_hash.as_str(), lowercase_hash.as_str());

    let first = Candidate {
        id: CandidateId::from_non_secret_parts(&["s19", "candidate-unique-first"]),
        primary_name: None,
        phone_hash: None,
        email_hash: Some(lowercase_hash),
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    let duplicate = Candidate {
        id: CandidateId::from_non_secret_parts(&["s19", "candidate-unique-duplicate"]),
        primary_name: None,
        phone_hash: None,
        email_hash: Some(uppercase_hash),
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };

    store.upsert_candidate(&first).unwrap();
    assert!(store.upsert_candidate(&duplicate).is_err());
}

#[test]
fn hashed_contact_assignment_reuses_candidate_and_updates_active_version_count() {
    let (_directory, store) = support::owned_store();
    let email_hash = contact_hash('d');
    let first_document = document("candidate-assign-first", false, DocumentStatus::Searchable);
    let second_document = document("candidate-assign-second", false, DocumentStatus::Searchable);
    let first_version = resume_version("candidate-assign-first-version", first_document.id.clone());
    let second_version = resume_version(
        "candidate-assign-second-version",
        second_document.id.clone(),
    );

    store.upsert_document(&first_document).unwrap();
    store.upsert_document(&second_document).unwrap();
    insert_resume_version_owned(&store, &first_version);
    insert_resume_version_owned(&store, &second_version);

    let first_assignment = store
        .assign_candidate_from_hashed_contacts(&first_version.id, Some(&email_hash), None)
        .unwrap()
        .expect("candidate assignment from hashed contact");
    assert_eq!(first_assignment.version_count, 0);

    let second_assignment = store
        .assign_candidate_from_hashed_contacts(&second_version.id, Some(&email_hash), None)
        .unwrap()
        .expect("existing candidate assignment from hashed contact");
    assert_eq!(second_assignment.id, first_assignment.id);
    assert_eq!(second_assignment.version_count, 0);
    publish_active_versions(&store, &[&first_version, &second_version]);
    assert_eq!(
        store
            .candidate_by_id(&first_assignment.id)
            .unwrap()
            .unwrap()
            .version_count,
        2
    );

    assert_eq!(
        store
            .assign_candidate_from_hashed_contacts(&first_version.id, None, None)
            .unwrap(),
        None
    );
}

#[test]
fn contact_hash_assignment_records_conflict_without_hash_or_contact_leakage() {
    let store = migrated_store();
    let email_hash = contact_hash('8');
    let phone_hash = contact_hash('9');
    let email_candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["s181", "email-candidate"]),
        primary_name: None,
        phone_hash: None,
        email_hash: Some(email_hash.clone()),
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    let phone_candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["s181", "phone-candidate"]),
        primary_name: None,
        phone_hash: Some(phone_hash.clone()),
        email_hash: None,
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };
    let document = document(
        "candidate-contact-conflict-document",
        false,
        DocumentStatus::Searchable,
    );
    let version = resume_version("candidate-contact-conflict-version", document.id.clone());

    store.upsert_candidate(&email_candidate).unwrap();
    store.upsert_candidate(&phone_candidate).unwrap();
    store.upsert_document(&document).unwrap();
    insert_resume_version(&store, &version);

    assert_eq!(
        store
            .assign_candidate_from_hashed_contacts(
                &version.id,
                Some(&email_hash),
                Some(&phone_hash)
            )
            .unwrap(),
        None
    );

    let conflicts = store.candidate_contact_conflicts().unwrap();
    assert_eq!(conflicts.len(), 1);
    let conflict = &conflicts[0];
    assert_eq!(conflict.resume_version_id, version.id);
    assert_eq!(conflict.email_candidate_id, email_candidate.id);
    assert_eq!(conflict.phone_candidate_id, phone_candidate.id);
    assert_eq!(conflict.updated_at, UnixTimestamp::from_unix_seconds(0));

    let debug = format!("{conflict:?}");
    assert!(!debug.contains(email_hash.as_str()));
    assert!(!debug.contains(phone_hash.as_str()));
    assert!(!debug.contains("candidate@example"));
    assert!(!debug.contains("+14155550100"));
}

#[test]
fn explicit_candidate_assignment_requires_existing_candidate() {
    let store = migrated_store();
    let document = document(
        "explicit-candidate-document",
        false,
        DocumentStatus::Searchable,
    );
    let version = resume_version("explicit-candidate-version", document.id.clone());
    let missing_candidate_id = CandidateId::from_non_secret_parts(&["s19", "missing-candidate"]);
    let candidate = Candidate {
        id: CandidateId::from_non_secret_parts(&["s19", "explicit-candidate"]),
        primary_name: None,
        phone_hash: Some(contact_hash('f')),
        email_hash: None,
        dedupe_key: None,
        merge_confidence: Some(1.0),
        version_count: 0,
    };

    store.upsert_document(&document).unwrap();
    insert_resume_version(&store, &version);

    assert!(store
        .insert_candidate_assignment(&version.id, &missing_candidate_id)
        .is_err());

    store.upsert_candidate(&candidate).unwrap();
    assert_eq!(
        store
            .insert_candidate_assignment(&version.id, &candidate.id)
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store.candidate_by_id(&candidate.id).unwrap(),
        Some(candidate)
    );
}

#[test]
fn visible_document_query_excludes_deleted_documents_by_default() {
    let store = migrated_store();
    let visible = document("visible-placeholder", false, DocumentStatus::Discovered);
    let deleted = document("deleted-placeholder", true, DocumentStatus::Deleted);

    store.upsert_document(&visible).unwrap();
    store.upsert_document(&deleted).unwrap();
    let visible_version = resume_version("visible-version-placeholder", visible.id.clone());
    insert_resume_version(&store, &visible_version);

    let visible_documents = store.visible_documents().unwrap();
    let visible_ids = visible_documents
        .iter()
        .map(|document| document.id.clone())
        .collect::<Vec<_>>();

    assert_eq!(visible_ids, vec![visible.id.clone()]);
    assert!(
        store
            .document_by_id(&deleted.id)
            .unwrap()
            .unwrap()
            .is_deleted
    );
    assert_eq!(
        store.resume_version_by_id(&visible_version.id).unwrap(),
        Some(visible_version.clone())
    );
    assert_eq!(
        store.resume_versions_for_document(&visible.id).unwrap(),
        vec![visible_version]
    );
}

#[test]
fn recovery_query_returns_interrupted_running_and_retryable_failed_jobs_only() {
    let store = migrated_store();
    let document = document(
        "job-document-placeholder",
        false,
        DocumentStatus::ParseQueued,
    );
    store.upsert_document(&document).unwrap();

    let running = job(
        "running-placeholder",
        &document.id,
        IngestJobStatus::Queued,
        0,
        3,
    );
    let interrupted = job(
        "interrupted-placeholder",
        &document.id,
        IngestJobStatus::Interrupted,
        1,
        3,
    );
    let retryable_failed = job(
        "retryable-failed-placeholder",
        &document.id,
        IngestJobStatus::FailedRetryable,
        2,
        3,
    );
    let exhausted_retryable = job(
        "exhausted-retryable-placeholder",
        &document.id,
        IngestJobStatus::FailedRetryable,
        3,
        3,
    );
    let completed = job(
        "completed-placeholder",
        &document.id,
        IngestJobStatus::Completed,
        1,
        3,
    );
    let permanent_failed = job(
        "permanent-failed-placeholder",
        &document.id,
        IngestJobStatus::FailedPermanent,
        1,
        3,
    );

    for ingest_job in [
        running.clone(),
        interrupted.clone(),
        retryable_failed.clone(),
        exhausted_retryable,
        completed,
        permanent_failed,
    ] {
        store.insert_ingest_job(&ingest_job).unwrap();
    }

    store
        .update_job_status(
            &running.id,
            IngestJobStatus::Running,
            UnixTimestamp::from_unix_seconds(1_800_000_050),
        )
        .unwrap();

    let recovery_ids = store
        .jobs_requiring_recovery()
        .unwrap()
        .into_iter()
        .map(|ingest_job| ingest_job.id)
        .collect::<Vec<_>>();

    assert_eq!(
        recovery_ids,
        vec![running.id, interrupted.id, retryable_failed.id]
    );
}

#[test]
fn retryable_queue_excludes_live_running_jobs() {
    let store = migrated_store();
    let document = document(
        "retryable-document-placeholder",
        false,
        DocumentStatus::ParseQueued,
    );
    store.upsert_document(&document).unwrap();

    let queued = job(
        "retryable-queued-placeholder",
        &document.id,
        IngestJobStatus::Queued,
        0,
        3,
    );
    let running = job(
        "retryable-running-placeholder",
        &document.id,
        IngestJobStatus::Running,
        1,
        3,
    )
    .started_at(UnixTimestamp::from_unix_seconds(1_800_000_010));
    let interrupted = job(
        "retryable-interrupted-placeholder",
        &document.id,
        IngestJobStatus::Interrupted,
        1,
        3,
    );
    let failed_retryable = job(
        "retryable-failed-placeholder",
        &document.id,
        IngestJobStatus::FailedRetryable,
        1,
        3,
    )
    .finished_at(UnixTimestamp::from_unix_seconds(1_800_000_020));
    let exhausted = job(
        "retryable-exhausted-placeholder",
        &document.id,
        IngestJobStatus::FailedRetryable,
        3,
        3,
    );

    for ingest_job in [
        queued.clone(),
        running,
        interrupted.clone(),
        failed_retryable.clone(),
        exhausted,
    ] {
        store.insert_ingest_job(&ingest_job).unwrap();
    }

    let retryable_ids = store
        .retryable_jobs()
        .unwrap()
        .into_iter()
        .map(|ingest_job| ingest_job.id)
        .collect::<Vec<_>>();

    assert_eq!(
        retryable_ids,
        vec![queued.id, interrupted.id, failed_retryable.id]
    );
}

#[test]
fn stale_running_ingest_jobs_are_recovered_to_interrupted_for_retry() {
    let store = migrated_store();
    let source_document = document(
        "stale-ingest-recovery-document-placeholder",
        false,
        DocumentStatus::OcrRequired,
    );
    store.upsert_document(&source_document).unwrap();
    let stale_started = UnixTimestamp::from_unix_seconds(1_800_000_010);
    let fresh_started = UnixTimestamp::from_unix_seconds(1_800_000_090);
    let recover_at = UnixTimestamp::from_unix_seconds(1_800_000_120);
    let stale_before = UnixTimestamp::from_unix_seconds(1_800_000_060);
    let claim_at = UnixTimestamp::from_unix_seconds(1_800_000_130);

    let stale = job(
        "stale-running-ingest-placeholder",
        &source_document.id,
        IngestJobStatus::Running,
        1,
        3,
    )
    .started_at(stale_started)
    .updated_at(stale_started);
    let fresh = job(
        "fresh-running-ingest-placeholder",
        &source_document.id,
        IngestJobStatus::Running,
        1,
        3,
    )
    .started_at(fresh_started)
    .updated_at(fresh_started);
    store.insert_ingest_job(&stale).unwrap();
    store.insert_ingest_job(&fresh).unwrap();

    let recovered = store
        .recover_stale_running_ingest_jobs(recover_at, stale_before)
        .unwrap();
    assert_eq!(recovered, 1);

    let recovered_job = store.ingest_job_by_id(&stale.id).unwrap().unwrap();
    assert_eq!(recovered_job.status, IngestJobStatus::Interrupted);
    assert_eq!(recovered_job.started_at, Some(stale_started));
    assert_eq!(recovered_job.updated_at, recover_at);
    assert_eq!(recovered_job.attempt_count, 1);
    assert_eq!(
        store.ingest_job_by_id(&fresh.id).unwrap().unwrap().status,
        IngestJobStatus::Running
    );
    let claimed = store.claim_next_job(claim_at).unwrap().unwrap();
    assert_eq!(claimed.id, stale.id);
    assert_eq!(claimed.status, IngestJobStatus::Running);
    assert_eq!(claimed.attempt_count, 2);
    assert_eq!(claimed.started_at, Some(claim_at));
    assert_eq!(store.claim_next_job(claim_at).unwrap(), None);
}

#[test]
fn embedding_update_jobs_are_durable_idempotent_and_claimable_by_resume_version() {
    let data_dir = temp_data_dir("embedding-update-job-placeholder");
    let document = document(
        "embedding-update-document-placeholder",
        false,
        DocumentStatus::Searchable,
    );
    let version = resume_version("embedding-update-version-placeholder", document.id.clone());
    let first_queued_at = UnixTimestamp::from_unix_seconds(1_800_000_610);
    let second_queued_at = UnixTimestamp::from_unix_seconds(1_800_000_611);
    let claim_at = UnixTimestamp::from_unix_seconds(1_800_000_710);
    let complete_at = UnixTimestamp::from_unix_seconds(1_800_000_720);
    let first_job_id;

    {
        let store = open_owned_store(&data_dir);
        store.upsert_document(&document).unwrap();
        insert_resume_version_owned(&store, &version);
        let mut unrelated_update_index = job(
            "embedding-update-unrelated-placeholder",
            &document.id,
            IngestJobStatus::Queued,
            0,
            3,
        );
        unrelated_update_index.kind = IngestJobKind::UpdateIndex;
        store.insert_ingest_job(&unrelated_update_index).unwrap();

        let first = store
            .enqueue_embedding_job_for_resume_version(
                &document.id,
                &version.id,
                "fixture-local-model",
                4,
                first_queued_at,
            )
            .unwrap();
        let second = store
            .enqueue_embedding_job_for_resume_version(
                &document.id,
                &version.id,
                "fixture-local-model",
                4,
                second_queued_at,
            )
            .unwrap();

        assert!(first.scheduled);
        assert!(!second.scheduled);
        assert_eq!(first.job.id, second.job.id);
        assert_eq!(first.job.kind, IngestJobKind::UpdateIndex);
        assert_eq!(first.job.resume_version_id, Some(version.id.clone()));
        assert_eq!(store.status_summary().unwrap().embedding_queue_depth, 1);
        first_job_id = first.job.id;
    }

    {
        let reopened = open_owned_store(&data_dir);
        let claimed = reopened
            .claim_next_embedding_job("fixture-local-model", 4, claim_at)
            .unwrap()
            .unwrap();

        assert_eq!(claimed.id, first_job_id);
        assert_eq!(claimed.kind, IngestJobKind::UpdateIndex);
        assert_eq!(claimed.resume_version_id, Some(version.id));
        assert_eq!(claimed.status, IngestJobStatus::Running);
        assert_eq!(claimed.attempt_count, 1);
        assert_eq!(reopened.status_summary().unwrap().embedding_queue_depth, 0);

        reopened
            .update_job_status(&claimed.id, IngestJobStatus::Completed, complete_at)
            .unwrap();
        assert_eq!(
            reopened
                .ingest_job_by_id(&claimed.id)
                .unwrap()
                .unwrap()
                .status,
            IngestJobStatus::Completed
        );
        assert_eq!(
            reopened
                .claim_next_embedding_job("fixture-local-model", 4, claim_at)
                .unwrap(),
            None
        );
    }

    remove_temp_dir(&data_dir);
}

#[test]
fn embedding_update_jobs_are_scoped_by_model_and_dimension() {
    let store = migrated_store();
    let document = document(
        "embedding-model-scope-document-placeholder",
        false,
        DocumentStatus::Searchable,
    );
    let version = resume_version(
        "embedding-model-scope-version-placeholder",
        document.id.clone(),
    );
    let queued_at = UnixTimestamp::from_unix_seconds(1_800_000_730);
    let claim_at = UnixTimestamp::from_unix_seconds(1_800_000_740);
    let complete_at = UnixTimestamp::from_unix_seconds(1_800_000_750);

    store.upsert_document(&document).unwrap();
    insert_resume_version(&store, &version);

    let first = store
        .enqueue_embedding_job_for_resume_version(
            &document.id,
            &version.id,
            "model-a",
            4,
            queued_at,
        )
        .unwrap();
    let duplicate = store
        .enqueue_embedding_job_for_resume_version(
            &document.id,
            &version.id,
            "model-a",
            4,
            queued_at,
        )
        .unwrap();
    assert!(first.scheduled);
    assert!(!duplicate.scheduled);
    assert_eq!(first.job.id, duplicate.job.id);

    let claimed_first = store
        .claim_next_embedding_job("model-a", 4, claim_at)
        .unwrap()
        .unwrap();
    store
        .update_job_status(&claimed_first.id, IngestJobStatus::Completed, complete_at)
        .unwrap();

    let second_model = store
        .enqueue_embedding_job_for_resume_version(
            &document.id,
            &version.id,
            "model-b",
            4,
            queued_at,
        )
        .unwrap();
    let second_dimension = store
        .enqueue_embedding_job_for_resume_version(
            &document.id,
            &version.id,
            "model-a",
            8,
            queued_at,
        )
        .unwrap();

    assert!(second_model.scheduled);
    assert!(second_dimension.scheduled);
    assert_ne!(second_model.job.id, first.job.id);
    assert_ne!(second_dimension.job.id, first.job.id);
    assert_eq!(store.status_summary().unwrap().embedding_queue_depth, 2);

    let claimed_second_model = store
        .claim_next_embedding_job("model-b", 4, claim_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed_second_model.id, second_model.job.id);
    assert_eq!(
        store
            .claim_next_embedding_job("model-b", 4, claim_at)
            .unwrap(),
        None
    );

    let claimed_second_dimension = store
        .claim_next_embedding_job("model-a", 8, claim_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed_second_dimension.id, second_dimension.job.id);
}

#[test]
fn completed_embedding_update_jobs_can_be_requeued_for_vector_snapshot_rebuild() {
    let store = migrated_store();
    let document = document(
        "embedding-rebuild-document-placeholder",
        false,
        DocumentStatus::Searchable,
    );
    let version = resume_version("embedding-rebuild-version-placeholder", document.id.clone());
    let queued_at = UnixTimestamp::from_unix_seconds(1_800_000_760);
    let claim_at = UnixTimestamp::from_unix_seconds(1_800_000_770);
    let complete_at = UnixTimestamp::from_unix_seconds(1_800_000_780);
    let requeue_at = UnixTimestamp::from_unix_seconds(1_800_000_790);
    let reclaim_at = UnixTimestamp::from_unix_seconds(1_800_000_800);

    store.upsert_document(&document).unwrap();
    insert_resume_version(&store, &version);
    let enqueued = store
        .enqueue_embedding_job_for_resume_version(
            &document.id,
            &version.id,
            "fixture-local-model",
            4,
            queued_at,
        )
        .unwrap();
    let claimed = store
        .claim_next_embedding_job("fixture-local-model", 4, claim_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, enqueued.job.id);
    store
        .update_job_status(&claimed.id, IngestJobStatus::Completed, complete_at)
        .unwrap();

    let requeued = store
        .requeue_completed_embedding_jobs_for_model("fixture-local-model", 4, requeue_at)
        .unwrap();
    assert_eq!(requeued, 1);
    assert_eq!(
        store
            .requeue_completed_embedding_jobs_for_model("other-model", 4, requeue_at)
            .unwrap(),
        0
    );
    let job = store.ingest_job_by_id(&claimed.id).unwrap().unwrap();
    assert_eq!(job.status, IngestJobStatus::Queued);
    assert_eq!(job.attempt_count, 0);
    assert_eq!(store.status_summary().unwrap().embedding_queue_depth, 1);

    let reclaimed = store
        .claim_next_embedding_job("fixture-local-model", 4, reclaim_at)
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.id, claimed.id);
    assert_eq!(reclaimed.status, IngestJobStatus::Running);
    assert_eq!(reclaimed.attempt_count, 1);
}

#[test]
fn ocr_page_cache_persists_success_and_retryable_failure_by_redacted_key() {
    let store = migrated_store();
    let key = OcrPageCacheKey::new(
        "synthetic-content-hash-for-ocr-cache",
        2,
        300,
        "eng+chi_sim",
        "balanced",
    )
    .unwrap();
    let success = OcrPageCacheEntry::succeeded(
        key.clone(),
        "Synthetic OCR text that must stay out of debug",
        0.87,
        "fixture-engine",
        42,
        UnixTimestamp::from_unix_seconds(1_800_000_800),
    )
    .unwrap();

    store.upsert_ocr_page_cache_entry(&success).unwrap();

    assert_eq!(
        store.ocr_page_cache_entry(&key).unwrap(),
        Some(success.clone())
    );
    let debug = format!("{success:?} {key:?}");
    assert!(!debug.contains("synthetic-content-hash-for-ocr-cache"));
    assert!(!debug.contains("Synthetic OCR text"));
    assert_eq!(
        success.text(),
        Some("Synthetic OCR text that must stay out of debug")
    );
    assert_eq!(success.status(), OcrPageCacheStatus::Succeeded);

    let retryable = OcrPageCacheEntry::failed_retryable(
        key.clone(),
        "WorkerUnavailable",
        UnixTimestamp::from_unix_seconds(1_800_000_801),
    )
    .unwrap();
    store.upsert_ocr_page_cache_entry(&retryable).unwrap();

    let loaded = store
        .ocr_page_cache_entry(&key)
        .unwrap()
        .expect("ocr cache entry");
    assert_eq!(loaded.status(), OcrPageCacheStatus::FailedRetryable);
    assert_eq!(loaded.text(), None);
    assert_eq!(loaded.error_kind(), Some("WorkerUnavailable"));
    assert!(!format!("{loaded:?}").contains("WorkerUnavailable"));
}

#[test]
fn ocr_page_cache_persists_word_boxes_without_debug_payload_leak() {
    let store = migrated_store();
    let key =
        OcrPageCacheKey::new("synthetic-bbox-content-hash", 1, 300, "eng", "balanced").unwrap();
    let word_boxes = vec![
        OcrWordBox::new("SecretName", 12, 34, 56, 18, 0.92).unwrap(),
        OcrWordBox::new("Rust", 72, 34, 40, 18, 0.88).unwrap(),
    ];
    let success = OcrPageCacheEntry::succeeded_with_word_boxes(
        key.clone(),
        "SecretName Rust",
        0.90,
        "fixture-tesseract",
        33,
        word_boxes.clone(),
        UnixTimestamp::from_unix_seconds(1_800_000_810),
    )
    .unwrap();

    store.upsert_ocr_page_cache_entry(&success).unwrap();

    let loaded = store
        .ocr_page_cache_entry(&key)
        .unwrap()
        .expect("ocr cache entry");
    assert_eq!(loaded.word_boxes(), word_boxes.as_slice());
    assert_eq!(loaded.word_boxes()[0].text(), "SecretName");
    assert_eq!(loaded.word_boxes()[0].left(), 12);
    assert_eq!(loaded.word_boxes()[0].top(), 34);
    assert_eq!(loaded.word_boxes()[0].width(), 56);
    assert_eq!(loaded.word_boxes()[0].height(), 18);
    assert_eq!(loaded.word_boxes()[0].confidence(), 0.92);

    let debug = format!("{loaded:?} {:?}", loaded.word_boxes());
    assert!(!debug.contains("SecretName"));
    assert!(!debug.contains("synthetic-bbox-content-hash"));
}

#[test]
fn ocr_page_cache_rejects_invalid_keys_and_confidence() {
    assert!(OcrPageCacheKey::new("", 1, 300, "eng", "balanced").is_err());
    assert!(OcrPageCacheKey::new("hash", 0, 300, "eng", "balanced").is_err());
    let key = OcrPageCacheKey::new("hash", 1, 300, "eng", "balanced").unwrap();
    assert!(OcrPageCacheEntry::succeeded(
        key,
        "text",
        1.5,
        "engine",
        1,
        UnixTimestamp::from_unix_seconds(1_800_000_802),
    )
    .is_err());
    assert!(OcrWordBox::new("", 1, 1, 1, 1, 0.5).is_err());
    assert!(OcrWordBox::new("word", 1, 1, 0, 1, 0.5).is_err());
    assert!(OcrWordBox::new("word", 1, 1, 1, 1, 1.5).is_err());
}

#[test]
fn entity_mentions_insert_query_and_redact_values() {
    let (_directory, store) = support::owned_store();
    let document = document("field-mention-document", false, DocumentStatus::Searchable);
    let version = resume_version("field-mention-version", document.id.clone());
    store.upsert_document(&document).unwrap();
    insert_resume_version_owned(&store, &version);

    let email = entity_mention(
        "email",
        &version.id,
        EntityType::Email,
        "Synthetic.Candidate@Example.Test",
        Some("synthetic.candidate@example.test"),
        9..41,
        0.99,
    );
    let skill = entity_mention(
        "skill",
        &version.id,
        EntityType::Skill,
        "Java",
        Some("Java"),
        80..84,
        0.91,
    );
    store
        .insert_entity_mentions(&version.id, &[email.clone(), skill.clone()])
        .unwrap();

    let mentions = store.entity_mentions_for_version(&version.id).unwrap();
    let mut expected_email = email;
    expected_email.raw_value = "<redacted:email>".to_string();
    expected_email.normalized_value = None;
    assert_eq!(mentions, vec![expected_email.clone(), skill.clone()]);
    assert_eq!(store.status_summary().unwrap().entity_mentions, 0);
    assert!(!format!("{:?}", mentions[0]).contains("Synthetic.Candidate"));

    let title = entity_mention(
        "title",
        &version.id,
        EntityType::Title,
        "Senior Backend Engineer",
        Some("backend_engineer"),
        120..143,
        0.82,
    );
    store
        .insert_entity_mentions(&version.id, std::slice::from_ref(&title))
        .unwrap();
    publish_active_versions(&store, &[&version]);

    assert_eq!(
        store.entity_mentions_for_version(&version.id).unwrap(),
        vec![expected_email, skill, title]
    );
    assert_eq!(store.status_summary().unwrap().entity_mentions, 3);
}

#[test]
fn entity_mentions_accept_major_values_for_searchable_prefilter() {
    let (_directory, store) = support::owned_store();
    let target_document = document("major-target-document", false, DocumentStatus::Searchable);
    let target_version = resume_version("major-target-version", target_document.id.clone());
    let decoy_document = document("major-decoy-document", false, DocumentStatus::Searchable);
    let decoy_version = resume_version("major-decoy-version", decoy_document.id.clone());
    store.upsert_document(&target_document).unwrap();
    insert_resume_version_owned(&store, &target_version);
    store.upsert_document(&decoy_document).unwrap();
    insert_resume_version_owned(&store, &decoy_version);

    let target_major = entity_mention(
        "major-target",
        &target_version.id,
        EntityType::Major,
        "Computer Science",
        Some("computer_science"),
        20..36,
        0.92,
    );
    let decoy_major = entity_mention(
        "major-decoy",
        &decoy_version.id,
        EntityType::Major,
        "Economics",
        Some("economics"),
        20..29,
        0.92,
    );
    store
        .insert_entity_mentions(&target_version.id, &[target_major])
        .unwrap();
    store
        .insert_entity_mentions(&decoy_version.id, &[decoy_major])
        .unwrap();
    publish_active_versions(&store, &[&target_version, &decoy_version]);

    let mentions = store
        .entity_mentions_for_version(&target_version.id)
        .unwrap();
    assert_eq!(mentions[0].entity_type, EntityType::Major);
    assert_eq!(
        mentions[0].normalized_value.as_deref(),
        Some("computer_science")
    );
    assert!(!format!("{:?}", mentions[0]).contains("Computer Science"));

    let document_ids = store
        .searchable_document_ids_with_entity_values(
            EntityType::Major,
            &["computer_science".to_string()],
            0.75,
            true,
        )
        .unwrap();
    assert_eq!(document_ids, vec![target_document.id]);
}

#[test]
fn searchable_document_ids_without_entity_type_matches_active_projection_only() {
    let (_directory, store) = support::owned_store();
    let no_tier_document = document("without-school-tier", false, DocumentStatus::Searchable);
    let known_tier_document = document("with-school-tier", false, DocumentStatus::Searchable);
    let low_confidence_document = document(
        "low-confidence-school-tier",
        false,
        DocumentStatus::Searchable,
    );
    let hidden_version_document = document(
        "hidden-school-tier-version",
        false,
        DocumentStatus::Searchable,
    );
    let discovered_document =
        document("discovered-without-tier", false, DocumentStatus::Discovered);
    let deleted_document = document("deleted-without-tier", true, DocumentStatus::Deleted);
    for document in [
        &no_tier_document,
        &known_tier_document,
        &low_confidence_document,
        &hidden_version_document,
        &discovered_document,
        &deleted_document,
    ] {
        store.upsert_document(document).unwrap();
    }

    let no_tier_version =
        resume_version("without-school-tier-version", no_tier_document.id.clone());
    let known_tier_version =
        resume_version("with-school-tier-version", known_tier_document.id.clone());
    let low_confidence_version = resume_version(
        "low-confidence-school-tier-version",
        low_confidence_document.id.clone(),
    );
    let hidden_version = resume_version(
        "hidden-school-tier-version",
        hidden_version_document.id.clone(),
    );
    let discovered_version = resume_version(
        "discovered-without-tier-version",
        discovered_document.id.clone(),
    );
    let deleted_version =
        resume_version("deleted-without-tier-version", deleted_document.id.clone());
    for version in [
        &no_tier_version,
        &known_tier_version,
        &low_confidence_version,
        &hidden_version,
        &discovered_version,
        &deleted_version,
    ] {
        insert_resume_version_owned(&store, version);
    }

    let known_tier = entity_mention(
        "known-school-tier",
        &known_tier_version.id,
        EntityType::SchoolTier,
        "985",
        Some("985"),
        10..13,
        0.95,
    );
    store
        .insert_entity_mentions(&known_tier_version.id, &[known_tier])
        .unwrap();
    let low_confidence_tier = entity_mention(
        "low-confidence-school-tier",
        &low_confidence_version.id,
        EntityType::SchoolTier,
        "985",
        Some("985"),
        10..13,
        0.40,
    );
    store
        .insert_entity_mentions(&low_confidence_version.id, &[low_confidence_tier])
        .unwrap();
    publish_active_versions(
        &store,
        &[
            &no_tier_version,
            &known_tier_version,
            &low_confidence_version,
        ],
    );

    let document_ids = store
        .searchable_document_ids_without_entity_type(EntityType::SchoolTier, 0.75)
        .unwrap();

    assert_eq!(
        document_ids,
        vec![low_confidence_document.id, no_tier_document.id]
    );
}

#[test]
fn searchable_document_ids_with_date_range_overlap_matches_active_projection_only() {
    let (_directory, store) = support::owned_store();
    let overlapping_document = document("date-range-overlap", false, DocumentStatus::Searchable);
    let open_ended_document = document("date-range-open-ended", false, DocumentStatus::Searchable);
    let before_document = document("date-range-before", false, DocumentStatus::Searchable);
    let low_confidence_document = document(
        "date-range-low-confidence",
        false,
        DocumentStatus::Searchable,
    );
    let hidden_version_document = document(
        "date-range-hidden-version",
        false,
        DocumentStatus::Searchable,
    );
    let deleted_document = document("date-range-deleted", true, DocumentStatus::Deleted);
    for document in [
        &overlapping_document,
        &open_ended_document,
        &before_document,
        &low_confidence_document,
        &hidden_version_document,
        &deleted_document,
    ] {
        store.upsert_document(document).unwrap();
    }

    let overlapping_version = resume_version(
        "date-range-overlap-version",
        overlapping_document.id.clone(),
    );
    let open_ended_version = resume_version(
        "date-range-open-ended-version",
        open_ended_document.id.clone(),
    );
    let before_version = resume_version("date-range-before-version", before_document.id.clone());
    let low_confidence_version = resume_version(
        "date-range-low-confidence-version",
        low_confidence_document.id.clone(),
    );
    let hidden_version = resume_version(
        "date-range-hidden-version",
        hidden_version_document.id.clone(),
    );
    let deleted_version = resume_version("date-range-deleted-version", deleted_document.id.clone());
    for version in [
        &overlapping_version,
        &open_ended_version,
        &before_version,
        &low_confidence_version,
        &hidden_version,
        &deleted_version,
    ] {
        insert_resume_version_owned(&store, version);
    }

    for (version, normalized_value, confidence) in [
        (&overlapping_version, "2020-03/2022-06", 0.95),
        (&open_ended_version, "2020-03/PRESENT", 0.95),
        (&before_version, "2017-01/2018-12", 0.95),
        (&low_confidence_version, "2020-03/2022-06", 0.40),
        (&hidden_version, "2020-03/2022-06", 0.95),
        (&deleted_version, "2020-03/2022-06", 0.95),
    ] {
        let mention = entity_mention(
            normalized_value,
            &version.id,
            EntityType::DateRange,
            normalized_value,
            Some(normalized_value),
            10..27,
            confidence,
        );
        store
            .insert_entity_mentions(&version.id, &[mention])
            .unwrap();
    }
    publish_active_versions(
        &store,
        &[
            &overlapping_version,
            &open_ended_version,
            &before_version,
            &low_confidence_version,
        ],
    );

    let document_ids = store
        .searchable_document_ids_with_date_range_overlap(2021 * 12 + 1, Some(2021 * 12 + 12), 0.75)
        .unwrap();

    assert_eq!(
        document_ids,
        vec![open_ended_document.id, overlapping_document.id]
    );
}

#[test]
fn contact_entity_mentions_do_not_persist_contact_values() {
    let data_dir = temp_data_dir("private-contact-mention");
    let store = open_owned_store(&data_dir);
    let document = document(
        "private-contact-mention-document",
        false,
        DocumentStatus::Searchable,
    );
    let version = resume_version("private-contact-mention-version", document.id.clone());
    store.upsert_document(&document).unwrap();
    insert_resume_version_owned(&store, &version);

    let email = entity_mention(
        "private-email",
        &version.id,
        EntityType::Email,
        "Sensitive.Candidate@Example.Test",
        Some("sensitive.candidate@example.test"),
        9..41,
        0.99,
    );
    let phone = entity_mention(
        "private-phone",
        &version.id,
        EntityType::Phone,
        "(415) 555-0132",
        Some("+14155550132"),
        42..56,
        0.98,
    );
    let wechat = entity_mention(
        "private-wechat",
        &version.id,
        EntityType::WeChat,
        "Candidate_2026",
        Some("candidate_2026"),
        57..71,
        0.97,
    );
    let skill = entity_mention(
        "private-skill",
        &version.id,
        EntityType::Skill,
        "Rust",
        Some("rust"),
        80..84,
        0.91,
    );

    store
        .insert_entity_mentions(&version.id, &[email, phone, wechat, skill.clone()])
        .unwrap();

    let mentions = store.entity_mentions_for_version(&version.id).unwrap();
    let email = mentions
        .iter()
        .find(|mention| mention.entity_type == EntityType::Email)
        .expect("email mention");
    assert_eq!(email.raw_value, "<redacted:email>");
    assert_eq!(email.normalized_value, None);
    assert_eq!(email.span_start, Some(9));
    assert_eq!(email.span_end, Some(41));
    assert_eq!(email.confidence, 0.99);
    assert_eq!(email.extractor, "rules-v1");

    let phone = mentions
        .iter()
        .find(|mention| mention.entity_type == EntityType::Phone)
        .expect("phone mention");
    assert_eq!(phone.raw_value, "<redacted:phone>");
    assert_eq!(phone.normalized_value, None);
    assert_eq!(phone.span_start, Some(42));
    assert_eq!(phone.span_end, Some(56));
    assert_eq!(phone.confidence, 0.98);
    assert_eq!(phone.extractor, "rules-v1");

    let wechat = mentions
        .iter()
        .find(|mention| mention.entity_type == EntityType::WeChat)
        .expect("wechat mention");
    assert_eq!(wechat.raw_value, "<redacted:wechat>");
    assert_eq!(wechat.normalized_value, None);
    assert_eq!(wechat.span_start, Some(57));
    assert_eq!(wechat.span_end, Some(71));
    assert_eq!(wechat.confidence, 0.97);
    assert_eq!(wechat.extractor, "rules-v1");

    assert!(mentions.iter().any(|mention| mention == &skill));
    let joined = mentions
        .iter()
        .map(|mention| format!("{} {:?}", mention.raw_value, mention.normalized_value))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("Sensitive.Candidate"));
    assert!(!joined.contains("sensitive.candidate@example.test"));
    assert!(!joined.contains("415"));
    assert!(!joined.contains("+14155550132"));
    assert!(!joined.contains("Candidate_2026"));
    assert!(!joined.contains("candidate_2026"));

    drop(store);
    let reader = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let persisted = reader.entity_mentions_for_version(&version.id).unwrap();
    assert_eq!(persisted, mentions);
    let persisted_dump = persisted
        .iter()
        .map(|mention| format!("{} {:?}", mention.raw_value, mention.normalized_value))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(persisted_dump.contains("<redacted:email>"));
    assert!(persisted_dump.contains("<redacted:phone>"));
    assert!(persisted_dump.contains("<redacted:wechat>"));
    assert!(!persisted_dump.contains("Sensitive.Candidate"));
    assert!(!persisted_dump.contains("sensitive.candidate@example.test"));
    assert!(!persisted_dump.contains("415"));
    assert!(!persisted_dump.contains("+14155550132"));
    assert!(!persisted_dump.contains("Candidate_2026"));
    assert!(!persisted_dump.contains("candidate_2026"));

    drop(reader);
    remove_temp_dir(&data_dir);
}

#[test]
fn claim_next_job_marks_one_retryable_job_running_with_attempt_increment() {
    let store = migrated_store();
    let document = document(
        "claim-document-placeholder",
        false,
        DocumentStatus::ParseQueued,
    );
    store.upsert_document(&document).unwrap();

    let failed_retryable = job(
        "claim-failed-placeholder",
        &document.id,
        IngestJobStatus::FailedRetryable,
        1,
        3,
    )
    .finished_at(UnixTimestamp::from_unix_seconds(1_800_000_020));
    let live_running = job(
        "claim-live-running-placeholder",
        &document.id,
        IngestJobStatus::Running,
        1,
        3,
    )
    .started_at(UnixTimestamp::from_unix_seconds(1_800_000_030));
    let claim_time = UnixTimestamp::from_unix_seconds(1_800_000_090);

    store.insert_ingest_job(&failed_retryable).unwrap();
    store.insert_ingest_job(&live_running).unwrap();

    let claimed = store.claim_next_job(claim_time).unwrap().unwrap();
    assert_eq!(claimed.id, failed_retryable.id);
    assert_eq!(claimed.status, IngestJobStatus::Running);
    assert_eq!(claimed.attempt_count, 2);
    assert_eq!(claimed.started_at, Some(claim_time));
    assert_eq!(claimed.updated_at, claim_time);
    assert_eq!(claimed.finished_at, None);

    assert_eq!(store.claim_next_job(claim_time).unwrap(), None);
}

#[test]
fn job_status_updates_set_timestamps_and_reject_terminal_transitions() {
    let store = migrated_store();
    let document = document(
        "transition-document-placeholder",
        false,
        DocumentStatus::ParseQueued,
    );
    store.upsert_document(&document).unwrap();

    let retrying = job(
        "transition-retrying-placeholder",
        &document.id,
        IngestJobStatus::FailedRetryable,
        1,
        3,
    )
    .finished_at(UnixTimestamp::from_unix_seconds(1_800_000_010));
    let completed = job(
        "transition-completed-placeholder",
        &document.id,
        IngestJobStatus::Completed,
        1,
        3,
    )
    .finished_at(UnixTimestamp::from_unix_seconds(1_800_000_020));

    store.insert_ingest_job(&retrying).unwrap();
    store.insert_ingest_job(&completed).unwrap();

    let running_at = UnixTimestamp::from_unix_seconds(1_800_000_100);
    store
        .update_job_status(&retrying.id, IngestJobStatus::Running, running_at)
        .unwrap();
    let running = store.ingest_job_by_id(&retrying.id).unwrap().unwrap();
    assert_eq!(running.status, IngestJobStatus::Running);
    assert_eq!(running.started_at, Some(running_at));
    assert_eq!(running.finished_at, None);
    assert_eq!(running.updated_at, running_at);

    let failed_at = UnixTimestamp::from_unix_seconds(1_800_000_130);
    store
        .update_job_status(&retrying.id, IngestJobStatus::FailedRetryable, failed_at)
        .unwrap();
    let failed = store.ingest_job_by_id(&retrying.id).unwrap().unwrap();
    assert_eq!(failed.status, IngestJobStatus::FailedRetryable);
    assert_eq!(failed.started_at, Some(running_at));
    assert_eq!(failed.finished_at, Some(failed_at));
    assert_eq!(failed.updated_at, failed_at);

    let completed_to_running = store.update_job_status(
        &completed.id,
        IngestJobStatus::Running,
        UnixTimestamp::from_unix_seconds(1_800_000_160),
    );
    assert_redacted_store_error(completed_to_running.unwrap_err());

    let missing = store.update_job_status(
        &IngestJobId::from_non_secret_parts(&["missing-transition-placeholder"]),
        IngestJobStatus::Running,
        UnixTimestamp::from_unix_seconds(1_800_000_170),
    );
    assert_redacted_store_error(missing.unwrap_err());
}

#[test]
fn status_summary_aggregates_documents_jobs_imports_and_search_projection() {
    let (_directory, store) = support::owned_store();
    let now = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let searchable = document(
        "status-searchable-placeholder",
        false,
        DocumentStatus::Searchable,
    );
    let partial = document(
        "status-partial-placeholder",
        false,
        DocumentStatus::IndexedPartial,
    );
    let failed_retryable = document(
        "status-retryable-placeholder",
        false,
        DocumentStatus::FailedRetryable,
    );
    let failed_permanent = document(
        "status-permanent-placeholder",
        false,
        DocumentStatus::FailedPermanent,
    );
    let ocr_required = document("status-ocr-placeholder", false, DocumentStatus::OcrRequired);
    let embedding_waiting = document(
        "status-embedding-placeholder",
        false,
        DocumentStatus::FieldsExtracted,
    );
    let deleted = document(
        "status-deleted-placeholder",
        true,
        DocumentStatus::Searchable,
    );

    for document in [
        searchable.clone(),
        partial,
        failed_retryable,
        failed_permanent,
        ocr_required.clone(),
        embedding_waiting.clone(),
        deleted,
    ] {
        store.upsert_document(&document).unwrap();
    }
    let searchable_version = resume_version("status-searchable-version", searchable.id.clone());
    insert_resume_version_owned(&store, &searchable_version);
    let embedding_version =
        resume_version("status-embedding-version", embedding_waiting.id.clone());
    insert_resume_version_owned(&store, &embedding_version);
    store
        .enqueue_embedding_job_for_resume_version(
            &embedding_waiting.id,
            &embedding_version.id,
            "fixture-local-model",
            4,
            now,
        )
        .unwrap();
    let running = job(
        "status-running-placeholder",
        &searchable.id,
        IngestJobStatus::Running,
        1,
        3,
    );
    let exhausted = job(
        "status-exhausted-placeholder",
        &searchable.id,
        IngestJobStatus::FailedRetryable,
        3,
        3,
    );
    store.insert_ingest_job(&running).unwrap();
    store.insert_ingest_job(&exhausted).unwrap();
    let queued_import = import_task(
        "status-import-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Queued,
    );
    publish_active_versions(&store, &[&searchable_version]);
    support::insert_import_task_owned(&store, &queued_import);

    let summary = store.status_summary().unwrap();

    assert_eq!(summary.indexed_documents, 1);
    assert_eq!(summary.searchable_documents, 1);
    assert_eq!(summary.partial_documents, 1);
    assert_eq!(summary.failed_retryable, 1);
    assert_eq!(summary.failed_permanent, 1);
    assert_eq!(summary.ocr_queue_depth, 1);
    assert_eq!(summary.embedding_queue_depth, 1);
    assert_eq!(summary.recovery_queue_depth, 1);
    assert_eq!(summary.import_tasks_queued, 1);
    assert_eq!(summary.import_tasks_recoverable, 0);
    assert_eq!(summary.ocr_jobs_queued, 0);
    assert_eq!(summary.ocr_language_unavailable, 0);
    assert_eq!(summary.index_health, IndexStateStatus::Ready);
    assert_eq!(
        summary.last_snapshot_id.as_deref(),
        Some("s3-active-projection")
    );
}

#[test]
fn import_tasks_persist_without_document_foreign_key() {
    let data_dir = temp_data_dir("import-task-placeholder");
    let task = import_task(
        "import-reopen-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Queued,
    );

    {
        let store = open_owned_store(&data_dir);
        support::insert_import_task_owned(&store, &task);
        assert_eq!(
            store.import_task_by_id(&task.id).unwrap(),
            Some(task.clone())
        );
    }

    {
        let reopened = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        assert_eq!(reopened.schema_version().unwrap(), 28);
        assert_eq!(reopened.import_task_by_id(&task.id).unwrap(), Some(task));
        assert!(reopened.visible_documents().unwrap().is_empty());
    }

    remove_temp_dir(&data_dir);
}

#[test]
fn purge_import_tasks_for_deleted_documents_keeps_unrelated_roots_with_visible_documents() {
    let store = migrated_store();
    let root_path = "synthetic/import/root";
    let mut first_document = document("purge-import-root-first", true, DocumentStatus::Deleted);
    let mut second_document = document(
        "purge-import-root-second",
        false,
        DocumentStatus::Searchable,
    );
    first_document.normalized_path = format!("{root_path}/first.pdf");
    second_document.normalized_path = format!("{root_path}/second.pdf");
    let task = import_task(
        "purge-import-root-task",
        root_path,
        ImportTaskStatus::Queued,
    );

    store.upsert_document(&first_document).unwrap();
    store.upsert_document(&second_document).unwrap();
    support::insert_import_task(&store, &task);
    let retained = store
        .purge_import_tasks_for_deleted_documents(std::slice::from_ref(&first_document.id))
        .unwrap();

    assert_eq!(retained.tasks(), 0);
    assert!(store.import_task_by_id(&task.id).unwrap().is_some());

    second_document.is_deleted = true;
    second_document.status = DocumentStatus::Deleted;
    second_document.updated_at = UnixTimestamp::from_unix_seconds(1_800_014_020);
    store.upsert_document(&second_document).unwrap();
    let purged = store
        .purge_import_tasks_for_deleted_documents(&[first_document.id, second_document.id])
        .unwrap();

    assert_eq!(purged.tasks(), 1);
    assert!(store.import_task_by_id(&task.id).unwrap().is_none());
}

#[test]
fn purge_import_tasks_matches_windows_canonical_root_to_normalized_document_path() {
    let store = migrated_store();
    let mut document = document("purge-import-root-windows", true, DocumentStatus::Deleted);
    document.source_uri = "file://c:/Synthetic/Import Root/resume.docx".to_string();
    document.normalized_path = "c:/Synthetic/Import Root/resume.docx".to_string();
    let task = import_task(
        "purge-import-root-windows-task",
        r"\\?\C:\Synthetic\Import Root",
        ImportTaskStatus::Queued,
    );

    store.upsert_document(&document).unwrap();
    support::insert_import_task(&store, &task);
    let purged = store
        .purge_import_tasks_for_deleted_documents(std::slice::from_ref(&document.id))
        .unwrap();

    assert_eq!(purged.tasks(), 1);
    assert!(store.import_task_by_id(&task.id).unwrap().is_none());
}

#[test]
fn import_task_status_updates_support_completion_and_retry() {
    let (_directory, store) = support::owned_store();
    let task = import_task(
        "import-status-update-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Queued,
    );
    support::insert_import_task_owned(&store, &task);
    let contract = support::processing_contract();

    let started_at = UnixTimestamp::from_unix_seconds(1_800_000_010);
    let running = store
        .claim_observed_import_task_for_worker(&task, started_at)
        .unwrap();
    let running = running.unwrap();
    assert_eq!(running.status, ImportTaskStatus::Running);
    assert_eq!(running.started_at, Some(started_at));
    assert_eq!(running.finished_at, None);
    let running_summary = store.status_summary().unwrap();
    assert_eq!(running_summary.import_tasks_queued, 0);
    assert_eq!(running_summary.import_tasks_recoverable, 1);
    assert_eq!(
        store.pending_import_task_by_root(&task.root_path).unwrap(),
        Some(running)
    );

    let retry_at = UnixTimestamp::from_unix_seconds(1_800_000_020);
    store
        .update_import_task_status(&task.id, ImportTaskStatus::FailedRetryable, retry_at)
        .unwrap();
    let retryable = store.import_task_by_id(&task.id).unwrap().unwrap();
    assert_eq!(retryable.status, ImportTaskStatus::FailedRetryable);
    assert_eq!(retryable.started_at, Some(started_at));
    assert_eq!(retryable.finished_at, Some(retry_at));
    assert_eq!(store.status_summary().unwrap().import_tasks_recoverable, 1);

    let restarted_at = UnixTimestamp::from_unix_seconds(1_800_000_030);
    let restarted = store
        .claim_observed_import_task_for_worker(&retryable, restarted_at)
        .unwrap();
    let restarted = restarted.unwrap();
    assert_eq!(restarted.status, ImportTaskStatus::Running);
    assert_eq!(restarted.started_at, Some(restarted_at));
    assert_eq!(restarted.finished_at, None);

    let completed_at = UnixTimestamp::from_unix_seconds(1_800_000_040);
    store
        .complete_import_task(
            &task.id,
            contract.id(),
            &support::import_scan_scope(&ImportTask {
                updated_at: completed_at,
                ..restarted.clone()
            }),
            completed_at,
        )
        .unwrap();
    let completed = store.import_task_by_id(&task.id).unwrap().unwrap();
    assert_eq!(completed.status, ImportTaskStatus::Completed);
    assert_eq!(completed.started_at, Some(restarted_at));
    assert_eq!(completed.finished_at, Some(completed_at));
    assert_eq!(store.status_summary().unwrap().import_tasks_recoverable, 0);
    assert_eq!(
        store.pending_import_task_by_root(&task.root_path).unwrap(),
        None
    );

    let completed_to_running =
        store.update_import_task_status(&task.id, ImportTaskStatus::Running, completed_at);
    assert_redacted_store_error(completed_to_running.unwrap_err());

    let time_travel_task = import_task(
        "import-status-time-travel-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Queued,
    );
    support::insert_import_task_owned(&store, &time_travel_task);
    let before_queue = UnixTimestamp::from_unix_seconds(1_799_999_999);
    let clock_safe_claim = store
        .claim_observed_import_task_for_worker(&time_travel_task, before_queue)
        .unwrap()
        .unwrap();
    assert_eq!(
        clock_safe_claim.started_at,
        Some(time_travel_task.queued_at)
    );

    assert!(store
        .claim_observed_import_task_for_worker(&retryable, retry_at)
        .unwrap()
        .is_none());
}

#[test]
fn cancelled_import_tasks_are_not_claimed_or_reported_as_queued() {
    let (_directory, store) = support::owned_store();
    let cancel_at = UnixTimestamp::from_unix_seconds(1_800_000_050);
    let claim_at = UnixTimestamp::from_unix_seconds(1_800_000_060);
    let queued = import_task(
        "cancelled-queued-import",
        "/private/cancelled/queued",
        ImportTaskStatus::Queued,
    );
    let mut retryable = import_task(
        "cancelled-retryable-import",
        "/private/cancelled/retryable",
        ImportTaskStatus::FailedRetryable,
    );
    retryable.started_at = Some(UnixTimestamp::from_unix_seconds(1_800_000_010));
    retryable.finished_at = Some(UnixTimestamp::from_unix_seconds(1_800_000_020));
    retryable.updated_at = UnixTimestamp::from_unix_seconds(1_800_000_020);

    support::insert_import_task_owned(&store, &queued);
    support::insert_import_task_owned(&store, &retryable);

    assert!(store.cancel_import_task(&queued.id, cancel_at).unwrap());
    assert!(store.is_import_task_cancelled(&queued.id).unwrap());
    assert!(store.cancel_import_task(&retryable.id, cancel_at).unwrap());
    assert!(store.is_import_task_cancelled(&retryable.id).unwrap());

    assert_eq!(
        store
            .pending_import_task_by_root(&queued.root_path)
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .import_task_claim_candidate_for_worker_excluding_due_at(claim_at, &[])
            .unwrap(),
        None
    );
    let summary = store.status_summary().unwrap();
    assert_eq!(summary.import_tasks_queued, 0);
    assert_eq!(summary.import_tasks_recoverable, 0);
    assert_eq!(summary.import_tasks_cancelled, 2);
}

#[test]
fn running_import_task_cancellation_is_recorded_and_removed_from_recovery() {
    let (_directory, store) = support::owned_store();
    let started_at = UnixTimestamp::from_unix_seconds(1_800_000_010);
    let cancel_at = UnixTimestamp::from_unix_seconds(1_800_000_020);
    let mut running = import_task(
        "cancelled-running-import",
        "/private/cancelled/running",
        ImportTaskStatus::Running,
    );
    running.started_at = Some(started_at);
    running.updated_at = started_at;

    support::insert_import_task_owned(&store, &running);

    assert!(store.cancel_import_task(&running.id, cancel_at).unwrap());
    assert!(store.is_import_task_cancelled(&running.id).unwrap());
    assert_eq!(
        store
            .pending_import_task_by_root(&running.root_path)
            .unwrap(),
        None
    );
    let summary = store.status_summary().unwrap();
    assert_eq!(summary.import_tasks_queued, 0);
    assert_eq!(summary.import_tasks_recoverable, 0);
    assert_eq!(summary.import_tasks_cancelled, 1);
}

#[test]
fn import_worker_claim_atomically_marks_next_task_running_and_skips_attempted_tasks() {
    let (_directory, store) = support::owned_store();
    let timestamp = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let mut running = import_task(
        "next-import-running",
        "synthetic/import/running",
        ImportTaskStatus::Running,
    );
    running.started_at = Some(timestamp);
    let mut completed = import_task(
        "next-import-completed",
        "synthetic/import/completed",
        ImportTaskStatus::Completed,
    );
    completed.started_at = Some(timestamp);
    completed.finished_at = Some(timestamp);
    let mut retryable = import_task(
        "next-import-retryable",
        "synthetic/import/retryable",
        ImportTaskStatus::FailedRetryable,
    );
    retryable.started_at = Some(timestamp);
    retryable.finished_at = Some(timestamp);
    let queued = import_task(
        "next-import-queued",
        "synthetic/import/queued",
        ImportTaskStatus::Queued,
    );

    support::insert_import_task_owned(&store, &running);
    support::insert_import_task_owned(&store, &completed);
    support::insert_import_task_owned(&store, &retryable);
    support::insert_import_task_owned(&store, &queued);

    let claim_at = UnixTimestamp::from_unix_seconds(1_900_000_000);
    let candidate = store
        .import_task_claim_candidate_for_worker_excluding_due_at(claim_at, &[])
        .unwrap()
        .unwrap();
    let claimed = store
        .claim_observed_import_task_for_worker(&candidate, claim_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, queued.id.clone());
    assert_eq!(claimed.status, ImportTaskStatus::Running);
    assert_eq!(claimed.started_at, Some(claim_at));
    assert_eq!(
        store.import_task_by_id(&queued.id).unwrap().unwrap().status,
        ImportTaskStatus::Running
    );

    let retryable_candidate = store
        .import_task_claim_candidate_for_worker_excluding_due_at(
            claim_at,
            std::slice::from_ref(&queued.id),
        )
        .unwrap()
        .unwrap();
    let claimed_retryable = store
        .claim_observed_import_task_for_worker(&retryable_candidate, claim_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed_retryable.id, retryable.id);
    assert_eq!(claimed_retryable.status, ImportTaskStatus::Running);
}

#[test]
fn import_worker_claim_respects_retryable_due_time_without_delaying_queued_tasks() {
    let (_directory, store) = support::owned_store();
    let timestamp = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let mut retryable = import_task(
        "retryable-not-due",
        "synthetic/import/retryable",
        ImportTaskStatus::FailedRetryable,
    );
    retryable.started_at = Some(timestamp);
    retryable.finished_at = Some(timestamp);
    let queued = import_task(
        "queued-despite-retry-backoff",
        "synthetic/import/queued",
        ImportTaskStatus::Queued,
    );

    support::insert_import_task_owned(&store, &retryable);
    support::insert_import_task_owned(&store, &queued);

    let claim_at = UnixTimestamp::from_unix_seconds(1_900_000_000);
    let retryable_not_due = UnixTimestamp::from_unix_seconds(1_799_999_999);
    let candidate = store
        .import_task_claim_candidate_for_worker_excluding_due_at(retryable_not_due, &[])
        .unwrap()
        .unwrap();
    let claimed = store
        .claim_observed_import_task_for_worker(&candidate, claim_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, queued.id);
    assert_eq!(claimed.status, ImportTaskStatus::Running);

    assert!(store
        .import_task_claim_candidate_for_worker_excluding_due_at(
            retryable_not_due,
            std::slice::from_ref(&claimed.id),
        )
        .unwrap()
        .is_none());

    let retryable_due = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let retryable_candidate = store
        .import_task_claim_candidate_for_worker_excluding_due_at(
            retryable_due,
            std::slice::from_ref(&claimed.id),
        )
        .unwrap()
        .unwrap();
    let claimed_retryable = store
        .claim_observed_import_task_for_worker(&retryable_candidate, claim_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed_retryable.id, retryable.id);
    assert_eq!(claimed_retryable.status, ImportTaskStatus::Running);
}

#[test]
fn orphaned_running_import_tasks_are_listed_and_atomically_requeued() {
    let (_directory, store) = support::owned_store();
    let started_at = UnixTimestamp::from_unix_seconds(1_800_000_010);
    let cancel_at = UnixTimestamp::from_unix_seconds(1_800_000_020);
    let requeued_at = UnixTimestamp::from_unix_seconds(1_800_000_030);
    let mut orphaned = import_task(
        "orphaned-running",
        "synthetic/import/orphaned",
        ImportTaskStatus::Running,
    );
    orphaned.started_at = Some(started_at);
    orphaned.updated_at = started_at;
    let mut cancelled = import_task(
        "cancelled-running-not-orphaned",
        "synthetic/import/cancelled-running",
        ImportTaskStatus::Running,
    );
    cancelled.started_at = Some(started_at);
    cancelled.updated_at = started_at;
    let queued = import_task(
        "queued-not-orphaned",
        "synthetic/import/queued",
        ImportTaskStatus::Queued,
    );

    support::insert_import_task_owned(&store, &orphaned);
    support::insert_import_task_owned(&store, &cancelled);
    support::insert_import_task_owned(&store, &queued);
    assert!(store.cancel_import_task(&cancelled.id, cancel_at).unwrap());

    assert_eq!(
        store.running_import_task_ids().unwrap(),
        vec![orphaned.id.clone()]
    );
    assert!(store
        .requeue_running_import_task(&orphaned.id, started_at, requeued_at)
        .unwrap());
    assert!(!store
        .requeue_running_import_task(&orphaned.id, started_at, requeued_at)
        .unwrap());
    assert!(!store
        .requeue_running_import_task(&cancelled.id, cancel_at, requeued_at)
        .unwrap());

    let recovered = store.import_task_by_id(&orphaned.id).unwrap().unwrap();
    assert_eq!(recovered.status, ImportTaskStatus::Queued);
    assert_eq!(recovered.started_at, None);
    assert_eq!(recovered.finished_at, None);
    assert_eq!(recovered.updated_at, requeued_at);
    assert_eq!(store.running_import_task_ids().unwrap(), Vec::new());

    let still_cancelled = store.import_task_by_id(&cancelled.id).unwrap().unwrap();
    assert_eq!(still_cancelled.status, ImportTaskStatus::Running);
    assert_eq!(still_cancelled.started_at, Some(started_at));
    assert_eq!(still_cancelled.finished_at, None);
    assert_eq!(still_cancelled.updated_at, cancel_at);
}

#[test]
fn orphan_requeue_uses_observed_version_and_survives_clock_rollback() {
    let (_directory, store) = support::owned_store();
    let observed_at = UnixTimestamp::from_unix_seconds(1_900_000_000);
    let recovery_clock = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let mut running = import_task(
        "orphaned-clock-rollback",
        "synthetic/import/orphaned-clock-rollback",
        ImportTaskStatus::Running,
    );
    running.started_at = Some(observed_at);
    running.updated_at = observed_at;
    support::insert_import_task_owned(&store, &running);

    assert!(store
        .requeue_running_import_task(&running.id, observed_at, recovery_clock)
        .unwrap());
    let recovered = store.import_task_by_id(&running.id).unwrap().unwrap();
    assert_eq!(recovered.status, ImportTaskStatus::Queued);
    assert_eq!(recovered.updated_at, observed_at);
    let candidate = store
        .import_task_claim_candidate_for_worker_excluding_due_at(recovery_clock, &[])
        .unwrap()
        .unwrap();
    let claimed = store
        .claim_observed_import_task_for_worker(&candidate, recovery_clock)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.status, ImportTaskStatus::Running);
    assert_eq!(claimed.started_at, Some(observed_at));
    assert_eq!(claimed.updated_at, observed_at);
}

#[test]
fn orphan_requeue_rejects_a_task_changed_after_observation() {
    let (_directory, store) = support::owned_store();
    let observed_at = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let heartbeat_at = UnixTimestamp::from_unix_seconds(1_800_000_010);
    let recovery_at = UnixTimestamp::from_unix_seconds(1_800_000_020);
    let mut running = import_task(
        "orphaned-observation-race",
        "synthetic/import/orphaned-observation-race",
        ImportTaskStatus::Running,
    );
    running.started_at = Some(observed_at);
    running.updated_at = observed_at;
    support::insert_import_task_owned(&store, &running);
    assert!(store
        .heartbeat_running_import_task(&running.id, heartbeat_at)
        .unwrap());

    assert!(!store
        .requeue_running_import_task(&running.id, observed_at, recovery_at)
        .unwrap());
    let still_running = store.import_task_by_id(&running.id).unwrap().unwrap();
    assert_eq!(still_running.status, ImportTaskStatus::Running);
    assert_eq!(still_running.updated_at, heartbeat_at);
}

#[test]
fn interrupted_requeue_uses_the_exact_failed_attempt_observation() {
    let (_directory, store) = support::owned_store();
    let observed_at = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let newer_attempt_at = UnixTimestamp::from_unix_seconds(1_800_000_010);
    let shutdown_at = UnixTimestamp::from_unix_seconds(1_800_000_020);
    let mut interrupted = import_task(
        "interrupted-observation-race",
        "synthetic/import/interrupted-observation-race",
        ImportTaskStatus::FailedRetryable,
    );
    interrupted.started_at = Some(observed_at);
    interrupted.finished_at = Some(observed_at);
    interrupted.updated_at = observed_at;
    support::insert_import_task_owned(&store, &interrupted);
    let restarted = store
        .claim_observed_import_task_for_worker(&interrupted, newer_attempt_at)
        .unwrap();
    assert!(restarted.is_some());
    store
        .update_import_task_status(
            &interrupted.id,
            ImportTaskStatus::FailedRetryable,
            newer_attempt_at,
        )
        .unwrap();

    assert!(!store
        .requeue_interrupted_import_task(&interrupted.id, observed_at, shutdown_at)
        .unwrap());
    let current = store.import_task_by_id(&interrupted.id).unwrap().unwrap();
    assert_eq!(current.status, ImportTaskStatus::FailedRetryable);
    assert_eq!(current.updated_at, newer_attempt_at);

    assert!(store
        .requeue_interrupted_import_task(&interrupted.id, newer_attempt_at, shutdown_at)
        .unwrap());
    let requeued = store.import_task_by_id(&interrupted.id).unwrap().unwrap();
    assert_eq!(requeued.status, ImportTaskStatus::Queued);
    assert_eq!(requeued.updated_at, shutdown_at);
}

#[test]
fn migration_publication_barrier_requires_the_latest_task_for_every_root_to_complete() {
    let store = migrated_store();
    let contract = support::activate_processing_contract(
        &store,
        UnixTimestamp::from_unix_seconds(1_799_999_999),
    );
    assert!(store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .is_some());
    let root_path = "synthetic/import/migration-barrier";
    let first = import_task(
        "migration-barrier-first",
        root_path,
        ImportTaskStatus::Queued,
    );
    let scope = ImportScanScope {
        import_task_id: first.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: root_path.to_string(),
        canonical_root_path: root_path.to_string(),
        files_discovered: 0,
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
        updated_at: first.updated_at,
    };
    support::insert_migration_rebuild_import_task_with_scan_scope(&store, &first, &scope);
    assert!(store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .is_none());
    support::complete_import_task_with_empty_manifest(
        &store,
        &first,
        UnixTimestamp::from_unix_seconds(1_800_000_001),
        UnixTimestamp::from_unix_seconds(1_800_000_002),
    );
    assert!(store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .is_some());

    let mut retry = import_task(
        "migration-barrier-retry",
        root_path,
        ImportTaskStatus::FailedRetryable,
    );
    retry.queued_at = UnixTimestamp::from_unix_seconds(1_700_000_000);
    retry.started_at = Some(retry.queued_at);
    retry.finished_at = Some(retry.queued_at);
    retry.updated_at = retry.queued_at;
    let mut retry_scope = migration_scan_scope(&retry);
    retry_scope.requested_root_path = "synthetic/import/migration-barrier-retry".to_string();
    support::insert_migration_rebuild_import_task_with_scan_scope(&store, &retry, &retry_scope);
    assert!(store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .is_none());
}

#[test]
fn migration_barrier_rejects_an_unfinished_full_corpus_task() {
    let store = migrated_store();
    let unknown = import_task(
        "migration-unknown-root",
        "synthetic/import/migration-unknown-root",
        ImportTaskStatus::Queued,
    );
    support::insert_migration_rebuild_import_task_with_scan_scope(
        &store,
        &unknown,
        &migration_scan_scope(&unknown),
    );

    assert!(store
        .acquire_migration_rebuild_barrier_token(support::processing_contract().id())
        .unwrap()
        .is_none());
}

#[test]
fn migration_barrier_excludes_paused_roots_with_cancelled_queued_tasks() {
    let store = migrated_store();
    let active = import_task(
        "migration-active-completed",
        "synthetic/import/migration-active",
        ImportTaskStatus::Queued,
    );
    support::insert_migration_rebuild_import_task_with_scan_scope(
        &store,
        &active,
        &migration_scan_scope(&active),
    );
    support::complete_import_task_with_empty_manifest(
        &store,
        &active,
        UnixTimestamp::from_unix_seconds(1_800_000_010),
        UnixTimestamp::from_unix_seconds(1_800_000_011),
    );

    let paused = import_task(
        "migration-paused-queued",
        "synthetic/import/migration-paused",
        ImportTaskStatus::Queued,
    );
    support::insert_migration_rebuild_import_task_with_scan_scope(
        &store,
        &paused,
        &migration_scan_scope(&paused),
    );
    let update = store
        .pause_import_root(
            &paused.root_path,
            UnixTimestamp::from_unix_seconds(1_800_000_020),
        )
        .unwrap();
    assert_eq!(update.cancellation_requests, 1);

    let token = store
        .acquire_migration_rebuild_barrier_token(support::processing_contract().id())
        .unwrap()
        .expect("the paused root must be atomically outside the barrier");
    let debug = format!("{token:?}");
    assert!(!debug.contains(&active.root_path));
    assert!(!debug.contains(&paused.root_path));
}

#[test]
fn migration_barrier_rejects_completed_task_with_scan_errors() {
    let store = migrated_store();
    let task = import_task(
        "migration-completed-scan-errors",
        "synthetic/import/migration-scan-errors",
        ImportTaskStatus::Queued,
    );
    let mut scope = migration_scan_scope(&task);
    scope.scan_errors = 1;
    support::insert_migration_rebuild_import_task_with_scan_scope(&store, &task, &scope);
    support::complete_import_task_with_final_scope(
        &store,
        &task,
        &scope,
        UnixTimestamp::from_unix_seconds(1_800_000_010),
        UnixTimestamp::from_unix_seconds(1_800_000_011),
    );

    assert!(store
        .acquire_migration_rebuild_barrier_token(support::processing_contract().id())
        .unwrap()
        .is_none());
}

#[test]
fn migration_barrier_rejects_completed_task_with_exhausted_scan_budget() {
    let store = migrated_store();
    let task = import_task(
        "migration-completed-budget-exhausted",
        "synthetic/import/migration-budget-exhausted",
        ImportTaskStatus::Queued,
    );
    let mut scope = migration_scan_scope(&task);
    scope.scan_budget_kind = Some(ImportScanBudgetKind::Files);
    scope.scan_budget_limit = Some(1);
    scope.scan_budget_observed = Some(1);
    scope.scan_budget_exhausted = true;
    support::insert_migration_rebuild_import_task_with_scan_scope(&store, &task, &scope);
    support::complete_import_task_with_final_scope(
        &store,
        &task,
        &scope,
        UnixTimestamp::from_unix_seconds(1_800_000_010),
        UnixTimestamp::from_unix_seconds(1_800_000_011),
    );

    assert!(store
        .acquire_migration_rebuild_barrier_token(support::processing_contract().id())
        .unwrap()
        .is_none());
}

#[test]
fn migration_commit_rejects_a_root_that_arrives_and_completes_during_snapshot_build() {
    let (_directory, store) = support::owned_store();
    let token = support::acquire_migration_rebuild_barrier_owned(
        &store,
        UnixTimestamp::from_unix_seconds(1_799_999_999),
    );
    let generation = "migration-new-root-race";
    let session = prepare_empty_migration_publication(&store, generation);

    let new_root = import_task(
        "migration-new-root-completed",
        "synthetic/import/migration-new-root",
        ImportTaskStatus::Queued,
    );
    support::insert_migration_rebuild_import_task_with_scan_scope_owned(
        &store,
        &new_root,
        &migration_scan_scope(&new_root),
    );
    support::complete_import_task_with_empty_manifest_owned(
        &store,
        &new_root,
        UnixTimestamp::from_unix_seconds(1_800_000_030),
        UnixTimestamp::from_unix_seconds(1_800_000_031),
    );

    assert_migration_commit_superseded(&session, generation, &token, 1_800_000_040);
}

#[test]
fn migration_commit_rejects_a_changed_latest_task_head() {
    let (_directory, store) = support::owned_store();
    let first = import_task(
        "migration-head-first",
        "synthetic/import/migration-head",
        ImportTaskStatus::Queued,
    );
    support::insert_migration_rebuild_import_task_with_scan_scope_owned(
        &store,
        &first,
        &migration_scan_scope(&first),
    );
    support::complete_import_task_with_empty_manifest_owned(
        &store,
        &first,
        UnixTimestamp::from_unix_seconds(1_800_000_010),
        UnixTimestamp::from_unix_seconds(1_800_000_011),
    );
    let token = store
        .acquire_migration_rebuild_barrier_token(support::processing_contract().id())
        .unwrap()
        .unwrap();
    let generation = "migration-task-head-race";
    let session = prepare_empty_migration_publication(&store, generation);

    let second = import_task(
        "migration-head-second",
        &first.root_path,
        ImportTaskStatus::Queued,
    );
    let mut second_scope = migration_scan_scope(&second);
    second_scope.requested_root_path = "synthetic/import/migration-head-updated".to_string();
    support::insert_migration_rebuild_import_task_with_scan_scope_owned(
        &store,
        &second,
        &second_scope,
    );
    support::complete_import_task_with_empty_manifest_owned(
        &store,
        &second,
        UnixTimestamp::from_unix_seconds(1_800_000_030),
        UnixTimestamp::from_unix_seconds(1_800_000_031),
    );

    assert_migration_commit_superseded(&session, generation, &token, 1_800_000_040);
}

#[test]
fn migration_commit_abandons_validated_publication_after_repair_blocked_race() {
    let (_directory, store) = support::owned_store();
    let token = support::acquire_migration_rebuild_barrier_owned(
        &store,
        UnixTimestamp::from_unix_seconds(1_799_999_999),
    );
    let generation = "migration-repair-blocked-race";
    let session = prepare_empty_migration_publication(&store, generation);
    store
        .block_migration_rebuild(
            SearchRepairReason::RuntimeInvariant,
            UnixTimestamp::from_unix_seconds(1_800_000_030),
        )
        .unwrap();

    assert_migration_commit_superseded(&session, generation, &token, 1_800_000_040);
    let state = store.search_projection_state().unwrap();
    assert_eq!(
        state.service_state,
        meta_store::SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        state.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(state.generation, None);
}

#[test]
fn ordinary_publication_cannot_overwrite_repair_blocked() {
    let (_directory, store) = support::owned_store();
    let generation = "ordinary-repair-blocked";
    let session = prepare_empty_migration_publication(&store, generation);
    store
        .block_migration_rebuild(
            SearchRepairReason::SourceUnavailable,
            UnixTimestamp::from_unix_seconds(1_800_000_030),
        )
        .unwrap();

    let outcome = session
        .commit_search_publication(&empty_publication_commit(generation, 1_800_000_040))
        .unwrap();
    assert_eq!(outcome, SearchPublicationOutcome::Superseded);
    assert_eq!(
        store.search_publication(generation).unwrap().unwrap().state,
        SearchPublicationState::Abandoned
    );
}

#[test]
fn import_task_api_rejects_invalid_lifecycle_timestamps() {
    let store = migrated_store();
    let timestamp = UnixTimestamp::from_unix_seconds(1_800_000_001);

    let mut queued_with_started = import_task(
        "invalid-queued-started-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Queued,
    );
    queued_with_started.started_at = Some(timestamp);
    assert_redacted_store_error(
        store
            .insert_import_task_with_scan_scope(
                &queued_with_started,
                &support::import_scan_scope(&queued_with_started),
                &support::processing_contract(),
            )
            .unwrap_err(),
    );

    let mut completed_without_finish = import_task(
        "invalid-completed-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Completed,
    );
    completed_without_finish.started_at = Some(timestamp);
    assert_redacted_store_error(
        store
            .insert_import_task_with_scan_scope(
                &completed_without_finish,
                &support::import_scan_scope(&completed_without_finish),
                &support::processing_contract(),
            )
            .unwrap_err(),
    );

    let mut running_with_finish = import_task(
        "invalid-running-finished-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Running,
    );
    running_with_finish.started_at = Some(timestamp);
    running_with_finish.finished_at = Some(timestamp);
    assert_redacted_store_error(
        store
            .insert_import_task_with_scan_scope(
                &running_with_finish,
                &support::import_scan_scope(&running_with_finish),
                &support::processing_contract(),
            )
            .unwrap_err(),
    );
}

#[test]
fn file_backed_connection_sets_pragmas() {
    let data_dir = temp_data_dir("pragma-placeholder");
    {
        let store = open_owned_store(&data_dir);

        assert!(store.foreign_keys_enabled().unwrap());
        assert_eq!(store.busy_timeout_millis().unwrap(), 5_000);
        assert_eq!(store.journal_mode().unwrap(), "delete");
    }

    remove_temp_dir(&data_dir);
}

#[test]
fn typed_write_rejects_out_of_range_resume_quality() {
    let store = migrated_store();
    let document = document(
        "checks-document-placeholder",
        false,
        DocumentStatus::Discovered,
    );
    store.upsert_document(&document).unwrap();
    let mut version = resume_version("checks-version-invalid-quality", document.id);
    version.quality_score = Some(1.5);

    assert_redacted_store_error(store.insert_resume_version(&version).unwrap_err());
}

#[test]
fn foreign_keys_reject_missing_parents_and_delete_cascades_children() {
    let store = migrated_store();
    let document = document("fk-document-placeholder", false, DocumentStatus::Discovered);
    let version = resume_version("fk-version-placeholder", document.id.clone());
    let ingest_job = job(
        "fk-job-placeholder",
        &document.id,
        IngestJobStatus::Queued,
        0,
        3,
    )
    .resume_version_id(version.id.clone());
    let classification = ResumeVersionClassification {
        resume_version_id: version.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        classified_at: UnixTimestamp::from_unix_seconds(1_800_000_001),
        review_disposition: ReviewDisposition::NotRequired,
    };

    let missing_parent = source_revision(&DocumentId::from_non_secret_parts(&[
        "s3",
        "missing-parent",
    ]));
    assert_redacted_store_error(store.insert_source_revision(&missing_parent).unwrap_err());

    store.upsert_document(&document).unwrap();
    insert_resume_version(&store, &version);
    store.insert_ingest_job(&ingest_job).unwrap();
    store
        .insert_resume_version_classification(&classification)
        .unwrap();

    let mut deleted = document.clone();
    deleted.is_deleted = true;
    deleted.status = DocumentStatus::Deleted;
    store.upsert_document(&deleted).unwrap();
    assert_eq!(
        store.purge_deleted_documents().unwrap().deleted_documents,
        1
    );
    assert_eq!(store.document_by_id(&document.id).unwrap(), None);
    assert_eq!(store.resume_version_by_id(&version.id).unwrap(), None);
    assert_eq!(store.ingest_job_by_id(&ingest_job.id).unwrap(), None);
    assert_eq!(
        store
            .resume_version_classification(&version.id, CLASSIFIER_EPOCH)
            .unwrap(),
        None
    );
}

#[test]
fn file_backed_store_recovers_unfinished_jobs_after_reopen() {
    let data_dir = temp_data_dir("recovery-reopen-placeholder");
    let document = document(
        "recovery-reopen-document-placeholder",
        false,
        DocumentStatus::ParseQueued,
    );
    let version = resume_version("recovery-reopen-version-placeholder", document.id.clone());
    let running = job(
        "recovery-reopen-running-placeholder",
        &document.id,
        IngestJobStatus::Running,
        1,
        3,
    )
    .resume_version_id(version.id.clone())
    .started_at(UnixTimestamp::from_unix_seconds(1_800_000_040));
    let interrupted = job(
        "recovery-reopen-interrupted-placeholder",
        &document.id,
        IngestJobStatus::Interrupted,
        1,
        3,
    )
    .resume_version_id(version.id.clone());
    let retryable_failed = job(
        "recovery-reopen-failed-placeholder",
        &document.id,
        IngestJobStatus::FailedRetryable,
        1,
        3,
    )
    .resume_version_id(version.id.clone())
    .finished_at(UnixTimestamp::from_unix_seconds(1_800_000_050));
    let completed = job(
        "recovery-reopen-completed-placeholder",
        &document.id,
        IngestJobStatus::Completed,
        1,
        3,
    )
    .resume_version_id(version.id.clone())
    .finished_at(UnixTimestamp::from_unix_seconds(1_800_000_060));

    {
        let store = open_owned_store(&data_dir);
        store.upsert_document(&document).unwrap();
        insert_resume_version_owned(&store, &version);
        for ingest_job in [
            running.clone(),
            interrupted.clone(),
            retryable_failed.clone(),
            completed,
        ] {
            store.insert_ingest_job(&ingest_job).unwrap();
        }
    }

    {
        let reopened = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        assert_eq!(
            reopened.document_by_id(&document.id).unwrap(),
            Some(document)
        );
        assert_eq!(
            reopened.resume_version_by_id(&version.id).unwrap(),
            Some(version)
        );
        let recovery_ids = reopened
            .jobs_requiring_recovery()
            .unwrap()
            .into_iter()
            .map(|ingest_job| ingest_job.id)
            .collect::<Vec<_>>();
        assert_eq!(
            recovery_ids,
            vec![running.id, interrupted.id, retryable_failed.id]
        );
    }

    remove_temp_dir(&data_dir);
}

fn migrated_store() -> EphemeralMetaStore {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    store
}

fn open_owned_store(data_dir: &Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data directory contended"),
    };
    owner.open_store().unwrap()
}

fn temp_data_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s3-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn set_owner_only_file_permissions(path: &PathBuf) {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    #[cfg(not(unix))]
    let _ = path;
}

fn remove_temp_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

fn document(label: &str, is_deleted: bool, status: DocumentStatus) -> Document {
    let now = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let id = DocumentId::from_non_secret_parts(&["s3", label]);
    let content_hash = ContentDigest::from_bytes(id.as_str().as_bytes());

    Document {
        id,
        source_uri: format!("synthetic://document/{label}"),
        normalized_path: format!("synthetic/root/{label}.txt"),
        file_name: format!("{label}.txt"),
        extension: FileExtension::Txt,
        byte_size: 128,
        mtime: now,
        content_hash: Some(content_hash.as_str().to_string()),
        text_hash: Some(
            ContentDigest::from_bytes(label.as_bytes())
                .as_str()
                .to_string(),
        ),
        is_deleted,
        created_at: now,
        updated_at: now,
        status,
    }
}

fn resume_version(label: &str, document_id: DocumentId) -> ResumeVersion {
    let revision = source_revision(&document_id);
    let clean_text = format!("SYNTHETIC CLEAN TEXT {label}");
    let normalized_text_hash = ContentDigest::from_bytes(clean_text.as_bytes());
    ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document_id,
            &revision.id,
            &normalized_text_hash,
            "parser-v1",
            "schema-v27",
        ),
        document_id,
        source_revision_id: revision.id,
        normalized_text_hash,
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v27".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some(format!("SYNTHETIC RAW TEXT {label}")),
        clean_text: Some(clean_text),
        quality_score: Some(0.8),
    }
}

fn source_revision(document_id: &DocumentId) -> SourceRevision {
    SourceRevision::for_content(
        document_id.clone(),
        ContentDigest::from_bytes(document_id.as_str().as_bytes()),
        128,
    )
}

fn insert_resume_version(store: &EphemeralMetaStore, version: &ResumeVersion) {
    let revision = source_revision(&version.document_id);
    assert_eq!(version.source_revision_id, revision.id);
    assert!(matches!(
        store.insert_source_revision(&revision).unwrap(),
        IdentityInsertOutcome::Inserted | IdentityInsertOutcome::AlreadyPresent
    ));
    assert!(matches!(
        store.insert_resume_version(version).unwrap(),
        IdentityInsertOutcome::Inserted | IdentityInsertOutcome::AlreadyPresent
    ));
}

fn insert_resume_version_owned(store: &OwnedMetaStore, version: &ResumeVersion) {
    let revision = source_revision(&version.document_id);
    assert_eq!(version.source_revision_id, revision.id);
    assert!(matches!(
        store.insert_source_revision(&revision).unwrap(),
        IdentityInsertOutcome::Inserted | IdentityInsertOutcome::AlreadyPresent
    ));
    assert!(matches!(
        store.insert_resume_version(version).unwrap(),
        IdentityInsertOutcome::Inserted | IdentityInsertOutcome::AlreadyPresent
    ));
}

fn publish_active_versions(store: &OwnedMetaStore, versions: &[&ResumeVersion]) {
    let projections = versions
        .iter()
        .map(|version| ActiveSearchProjection {
            document_id: version.document_id.clone(),
            resume_version_id: version.id.clone(),
        })
        .collect::<Vec<_>>();
    for version in versions {
        assert!(matches!(
            store
                .insert_resume_version_classification(&ResumeVersionClassification {
                    resume_version_id: version.id.clone(),
                    status: ClassificationStatus::ResumeCandidate,
                    classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                    reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
                    classified_at: UnixTimestamp::from_unix_seconds(1_800_000_001),
                    review_disposition: ReviewDisposition::NotRequired,
                })
                .unwrap(),
            IdentityInsertOutcome::Inserted | IdentityInsertOutcome::AlreadyPresent
        ));
    }
    let projection_digest =
        SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
            (
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
        }))
        .unwrap();
    let migration_barrier = support::acquire_migration_rebuild_barrier_owned(
        store,
        UnixTimestamp::from_unix_seconds(1_799_999_999),
    );
    let generation = "s3-active-projection";
    let session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: None,
                expected_visible_epoch: 0,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now: UnixTimestamp::from_unix_seconds(1_800_000_010),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.to_string(),
        projections.len() as u64,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"s3-fulltext-snapshot"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        projections.len() as u64,
        projection_digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(b"s3-vector-snapshot"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: UnixTimestamp::from_unix_seconds(1_800_000_020),
        })
        .unwrap();
    let terminal_documents = versions
        .iter()
        .map(|version| {
            let document = store.document_by_id(&version.document_id).unwrap().unwrap();
            TerminalDocumentUpdate {
                document_id: document.id,
                expected_status: document.status,
                expected_is_deleted: document.is_deleted,
                expected_content_hash: source_revision(&version.document_id).content_hash,
                terminal_status: DocumentStatus::Searchable,
                terminal_is_deleted: false,
            }
        })
        .collect::<Vec<_>>();
    let commit_now = UnixTimestamp::from_unix_seconds(1_800_000_030);
    let projected_documents = support::projected_documents_for_commit(
        store,
        &projections,
        &terminal_documents,
        commit_now,
    );
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation,
                    terminal_documents: &terminal_documents,
                    projections: &projections,
                    projected_documents: &projected_documents,
                    vector_coverage: &[],
                    now: commit_now,
                },
                &migration_barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
}

fn contact_hash(hex: char) -> ContactHash {
    let digest = std::iter::repeat_n(hex, 64).collect::<String>();
    ContactHash::from_keyed_digest(digest).unwrap()
}

fn entity_mention(
    label: &str,
    version_id: &ResumeVersionId,
    entity_type: EntityType,
    raw_value: &str,
    normalized_value: Option<&str>,
    span: Range<usize>,
    confidence: f32,
) -> EntityMention {
    EntityMention {
        id: EntityMentionId::from_non_secret_parts(&["s16", version_id.as_str(), label]),
        resume_version_id: version_id.clone(),
        section_id: None,
        entity_type,
        raw_value: raw_value.to_string(),
        normalized_value: normalized_value.map(str::to_string),
        span_start: Some(span.start),
        span_end: Some(span.end),
        confidence,
        extractor: "rules-v1".to_string(),
    }
}

fn job(
    label: &str,
    document_id: &DocumentId,
    status: IngestJobStatus,
    attempt_count: u32,
    max_attempts: u32,
) -> IngestJob {
    let now = UnixTimestamp::from_unix_seconds(1_800_000_000);

    IngestJob {
        id: IngestJobId::from_non_secret_parts(&["s3", label]),
        document_id: document_id.clone(),
        resume_version_id: None,
        kind: IngestJobKind::ParseDocument,
        status,
        attempt_count,
        max_attempts,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
        failure_kind: None,
    }
}

fn import_task(label: &str, root_path: &str, status: ImportTaskStatus) -> ImportTask {
    let now = UnixTimestamp::from_unix_seconds(1_800_000_000);

    ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s4", label]),
        root_path: root_path.to_string(),
        status,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    }
}

fn migration_scan_scope(task: &ImportTask) -> ImportScanScope {
    ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: task.root_path.clone(),
        canonical_root_path: task.root_path.clone(),
        files_discovered: 0,
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
        updated_at: task.updated_at,
    }
}

fn prepare_empty_migration_publication(
    store: &OwnedMetaStore,
    generation: &str,
) -> SearchPublicationSession {
    let projection_digest = SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap();
    let session = store.wait_for_search_publication_session().unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: generation.to_string(),
                base_generation: None,
                expected_visible_epoch: 0,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: projection_digest.clone(),
                now: UnixTimestamp::from_unix_seconds(1_800_000_010),
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.to_string(),
        0,
        projection_digest.clone(),
        ContentDigest::from_bytes(b"empty-migration-fulltext"),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        0,
        projection_digest,
        SearchProjectionDigest::from_pairs::<_, &str, &str>([]).unwrap(),
        ContentDigest::from_bytes(b"empty-migration-vector"),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now: UnixTimestamp::from_unix_seconds(1_800_000_020),
        })
        .unwrap();
    session
}

fn empty_publication_commit(generation: &str, now_seconds: i64) -> SearchPublicationCommit<'_> {
    SearchPublicationCommit {
        generation,
        terminal_documents: &[],
        projections: &[],
        projected_documents: &[],
        vector_coverage: &[],
        now: UnixTimestamp::from_unix_seconds(now_seconds),
    }
}

fn assert_migration_commit_superseded(
    session: &SearchPublicationSession,
    generation: &str,
    token: &MigrationRebuildBarrierToken,
    now_seconds: i64,
) {
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &empty_publication_commit(generation, now_seconds),
                token,
            )
            .unwrap(),
        SearchPublicationOutcome::Superseded
    );
    assert_eq!(
        session
            .owned_store()
            .search_publication(generation)
            .unwrap()
            .unwrap()
            .state,
        SearchPublicationState::Abandoned
    );
}

trait IngestJobTestExt {
    fn started_at(self, timestamp: UnixTimestamp) -> Self;
    fn finished_at(self, timestamp: UnixTimestamp) -> Self;
    fn updated_at(self, timestamp: UnixTimestamp) -> Self;
    fn resume_version_id(self, id: ResumeVersionId) -> Self;
}

impl IngestJobTestExt for IngestJob {
    fn started_at(mut self, timestamp: UnixTimestamp) -> Self {
        self.started_at = Some(timestamp);
        self
    }

    fn finished_at(mut self, timestamp: UnixTimestamp) -> Self {
        self.finished_at = Some(timestamp);
        self
    }

    fn updated_at(mut self, timestamp: UnixTimestamp) -> Self {
        self.updated_at = timestamp;
        self
    }

    fn resume_version_id(mut self, id: ResumeVersionId) -> Self {
        self.resume_version_id = Some(id);
        self
    }
}

fn assert_redacted_store_error(error: meta_store::MetaStoreError) {
    let display = error.to_string();
    let debug = format!("{error:?}");

    for leaked in [
        "SELECT",
        "INSERT",
        "UPDATE",
        "DELETE",
        "synthetic/root",
        "synthetic://",
        ".sqlite",
    ] {
        assert!(!display.contains(leaked), "display leaked {leaked}");
        assert!(!debug.contains(leaked), "debug leaked {leaked}");
    }
}
