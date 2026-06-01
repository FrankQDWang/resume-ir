use std::fs;
use std::ops::Range;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use meta_store::{
    Candidate, CandidateId, ContactHash, Document, DocumentId, DocumentStatus, EntityMention,
    EntityMentionId, EntityType, FileExtension, ImportRootKind, ImportRootPreset,
    ImportScanBudgetKind, ImportScanError, ImportScanErrorKind, ImportScanErrorOperation,
    ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus, IndexState,
    IndexStateStatus, IngestJob, IngestJobId, IngestJobKind, IngestJobStatus, MetaStore,
    OcrPageCacheEntry, OcrPageCacheKey, OcrPageCacheStatus, ResumeVersion, ResumeVersionId,
    ResumeVisibility, UnixTimestamp, WorkerTaskControl, WorkerTaskKind,
};
use rusqlite::{params, Connection};

#[test]
fn migrations_are_idempotent_and_schema_v1_is_queryable() {
    let store = MetaStore::open_in_memory().unwrap();

    assert!(store.foreign_keys_enabled().unwrap());

    let first = store.run_migrations().unwrap();
    assert_eq!(
        first.applied_versions(),
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    assert_eq!(store.schema_version().unwrap(), 11);

    for table_name in [
        "candidate",
        "document",
        "resume_version",
        "ingest_job",
        "index_state",
        "import_task",
        "entity_mention",
        "ocr_page_cache",
        "worker_task_control",
        "import_scan_scope",
        "import_scan_error",
    ] {
        assert!(store.schema_table_exists(table_name).unwrap());
    }

    let second = store.run_migrations().unwrap();
    assert!(second.applied_versions().is_empty());
    assert_eq!(store.schema_version().unwrap(), 11);
}

#[test]
fn worker_task_control_defaults_to_running_and_persists_pause_state() {
    let db_path = temp_db_path("worker-task-control-placeholder");
    let pause_at = UnixTimestamp::from_unix_seconds(1_800_000_330);
    let resume_at = UnixTimestamp::from_unix_seconds(1_800_000_360);

    {
        let store = MetaStore::open(&db_path).unwrap();
        store.run_migrations().unwrap();
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
        let reopened = MetaStore::open(&db_path).unwrap();
        reopened.run_migrations().unwrap();
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

    remove_temp_db(&db_path);
}

#[test]
fn import_scan_scope_persists_root_profile_and_redacted_progress_counts() {
    let db_path = temp_db_path("import-scan-scope-placeholder");
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
        let store = MetaStore::open(&db_path).unwrap();
        store.run_migrations().unwrap();
        store.insert_import_task(&task).unwrap();

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
        let reopened = MetaStore::open(&db_path).unwrap();
        reopened.run_migrations().unwrap();
        let persisted = reopened
            .latest_import_scan_scope()
            .unwrap()
            .expect("latest import scan scope");

        assert_eq!(persisted, completed_scope);
        let debug = format!("{persisted:?}");
        assert!(!debug.contains("/private/root"));
        assert!(debug.contains("files_discovered"));
    }

    remove_temp_db(&db_path);
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

    store
        .insert_import_task_with_scan_scope(&task, &scope)
        .unwrap();

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

    store.insert_import_task(&task).unwrap();

    store
        .replace_import_scan_errors(&task.id, &first_errors)
        .unwrap();
    assert_eq!(
        store.import_scan_errors_for_task(&task.id).unwrap(),
        first_errors
    );
    assert_eq!(store.status_summary().unwrap().import_scan_errors, 2);

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
fn hashed_contact_assignment_reuses_candidate_and_updates_version_count() {
    let store = migrated_store();
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
    store.upsert_resume_version(&first_version).unwrap();
    store.upsert_resume_version(&second_version).unwrap();

    let first_assignment = store
        .assign_candidate_from_hashed_contacts(&first_version.id, Some(&email_hash), None)
        .unwrap()
        .expect("candidate assignment from hashed contact");
    assert_eq!(first_assignment.version_count, 1);
    assert_eq!(
        store
            .resume_version_by_id(&first_version.id)
            .unwrap()
            .unwrap()
            .candidate_id,
        Some(first_assignment.id.clone())
    );

    let second_assignment = store
        .assign_candidate_from_hashed_contacts(&second_version.id, Some(&email_hash), None)
        .unwrap()
        .expect("existing candidate assignment from hashed contact");
    assert_eq!(second_assignment.id, first_assignment.id);
    assert_eq!(second_assignment.version_count, 2);
    assert_eq!(
        store
            .resume_version_by_id(&second_version.id)
            .unwrap()
            .unwrap()
            .candidate_id,
        Some(first_assignment.id.clone())
    );
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
    store.upsert_resume_version(&version).unwrap();

    assert!(store
        .assign_candidate_to_version(&version.id, &missing_candidate_id)
        .is_err());

    store.upsert_candidate(&candidate).unwrap();
    let assigned = store
        .assign_candidate_to_version(&version.id, &candidate.id)
        .unwrap()
        .expect("assigned candidate exists");

    assert_eq!(assigned.id, candidate.id);
    assert_eq!(assigned.version_count, 1);
    assert_eq!(
        store
            .resume_version_by_id(&version.id)
            .unwrap()
            .unwrap()
            .candidate_id,
        Some(candidate.id)
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
    store.upsert_resume_version(&visible_version).unwrap();

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
fn latest_visible_resume_version_for_document_uses_latest_inserted_non_hidden_version() {
    let store = migrated_store();
    let document = document(
        "latest-version-placeholder",
        false,
        DocumentStatus::Searchable,
    );
    store.upsert_document(&document).unwrap();
    let mut old_visible = resume_version("latest-old-version-placeholder", document.id.clone());
    old_visible.clean_text = Some("OLD_VERSION_SHOULD_NOT_APPEAR".to_string());
    let mut latest_hidden =
        resume_version("latest-hidden-version-placeholder", document.id.clone());
    latest_hidden.visibility = ResumeVisibility::Hidden;
    latest_hidden.clean_text = Some("HIDDEN_VERSION_SHOULD_NOT_APPEAR".to_string());
    let mut latest_visible = resume_version("latest-new-version-placeholder", document.id.clone());
    latest_visible.clean_text = Some("LATEST_VERSION_SHOULD_APPEAR".to_string());

    store.upsert_resume_version(&old_visible).unwrap();
    store.upsert_resume_version(&latest_hidden).unwrap();
    store.upsert_resume_version(&latest_visible).unwrap();

    let selected = store
        .latest_visible_resume_version_for_document(&document.id)
        .unwrap()
        .unwrap();

    assert_eq!(selected.id, latest_visible.id);
    assert_eq!(
        selected.clean_text.as_deref(),
        Some("LATEST_VERSION_SHOULD_APPEAR")
    );
}

#[test]
fn mark_document_deleted_sets_tombstone_hides_versions_and_status_counts() {
    let store = migrated_store();
    let now = UnixTimestamp::from_unix_seconds(1_800_000_500);
    let visible = document("soft-delete-placeholder", false, DocumentStatus::Searchable);
    let version = resume_version("soft-delete-version-placeholder", visible.id.clone());

    store.upsert_document(&visible).unwrap();
    store.upsert_resume_version(&version).unwrap();
    assert_eq!(store.status_summary().unwrap().searchable_documents, 1);

    let deleted = store
        .mark_document_deleted(&visible.id, now)
        .unwrap()
        .expect("document exists");

    assert_eq!(deleted.id, visible.id);
    assert!(deleted.is_deleted);
    assert_eq!(deleted.status, DocumentStatus::Deleted);
    assert_eq!(deleted.updated_at, now);
    assert!(store.visible_documents().unwrap().is_empty());
    assert_eq!(store.status_summary().unwrap().searchable_documents, 0);
    assert_eq!(
        store
            .resume_version_by_id(&version.id)
            .unwrap()
            .unwrap()
            .visibility,
        ResumeVisibility::Hidden
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
fn ocr_document_jobs_are_durable_idempotent_and_claimable_by_kind() {
    let store = migrated_store();
    let document = document(
        "ocr-page-document-placeholder",
        false,
        DocumentStatus::OcrRequired,
    );
    store.upsert_document(&document).unwrap();

    let first = store
        .enqueue_ocr_job_for_document(
            &document.id,
            UnixTimestamp::from_unix_seconds(1_800_000_600),
        )
        .unwrap();
    let second = store
        .enqueue_ocr_job_for_document(
            &document.id,
            UnixTimestamp::from_unix_seconds(1_800_000_601),
        )
        .unwrap();

    assert!(first.inserted);
    assert!(!second.inserted);
    assert_eq!(first.job.id, second.job.id);
    assert_eq!(first.job.kind, IngestJobKind::OcrDocument);
    assert_eq!(store.status_summary().unwrap().ocr_jobs_queued, 1);

    let claimed = store
        .claim_next_job_by_kind(
            IngestJobKind::OcrDocument,
            UnixTimestamp::from_unix_seconds(1_800_000_700),
        )
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, first.job.id);
    assert_eq!(claimed.kind, IngestJobKind::OcrDocument);
    assert_eq!(claimed.status, IngestJobStatus::Running);
    assert_eq!(claimed.attempt_count, 1);
    assert_eq!(store.status_summary().unwrap().ocr_jobs_queued, 0);
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
}

#[test]
fn entity_mentions_replace_query_and_redact_values() {
    let store = migrated_store();
    let document = document("field-mention-document", false, DocumentStatus::Searchable);
    let version = resume_version("field-mention-version", document.id.clone());
    store.upsert_document(&document).unwrap();
    store.upsert_resume_version(&version).unwrap();

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
        .replace_entity_mentions(&version.id, &[email.clone(), skill.clone()])
        .unwrap();

    let mentions = store.entity_mentions_for_version(&version.id).unwrap();
    let mut expected_email = email;
    expected_email.raw_value = "<redacted:email>".to_string();
    expected_email.normalized_value = None;
    assert_eq!(mentions, vec![expected_email, skill]);
    assert_eq!(store.status_summary().unwrap().entity_mentions, 2);
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
        .replace_entity_mentions(&version.id, std::slice::from_ref(&title))
        .unwrap();

    assert_eq!(
        store.entity_mentions_for_version(&version.id).unwrap(),
        vec![title]
    );
    assert_eq!(store.status_summary().unwrap().entity_mentions, 1);
}

#[test]
fn contact_entity_mentions_do_not_persist_contact_values() {
    let db_path = temp_db_path("private-contact-mention");
    let store = MetaStore::open(&db_path).unwrap();
    store.run_migrations().unwrap();
    let document = document(
        "private-contact-mention-document",
        false,
        DocumentStatus::Searchable,
    );
    let version = resume_version("private-contact-mention-version", document.id.clone());
    store.upsert_document(&document).unwrap();
    store.upsert_resume_version(&version).unwrap();

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
        .replace_entity_mentions(&version.id, &[email, phone, skill.clone()])
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

    let raw_connection = open_raw_connection(&db_path);
    let raw_dump = raw_entity_mention_value_dump(&raw_connection);
    assert!(raw_dump.contains("<redacted:email>"));
    assert!(raw_dump.contains("<redacted:phone>"));
    assert!(!raw_dump.contains("Sensitive.Candidate"));
    assert!(!raw_dump.contains("sensitive.candidate@example.test"));
    assert!(!raw_dump.contains("415"));
    assert!(!raw_dump.contains("+14155550132"));

    remove_temp_db(&db_path);
}

#[test]
fn schema_v6_redacts_existing_contact_entity_mentions() {
    let db_path = temp_db_path("legacy-contact-mention");
    let document = document(
        "legacy-contact-mention-document",
        false,
        DocumentStatus::Searchable,
    );
    let version = resume_version("legacy-contact-mention-version", document.id.clone());
    let email = entity_mention(
        "legacy-email",
        &version.id,
        EntityType::Email,
        "Legacy.Candidate@Example.Test",
        Some("legacy.candidate@example.test"),
        9..38,
        0.99,
    );
    let phone = entity_mention(
        "legacy-phone",
        &version.id,
        EntityType::Phone,
        "(415) 555-0199",
        Some("+14155550199"),
        40..54,
        0.98,
    );
    let skill = entity_mention(
        "legacy-skill",
        &version.id,
        EntityType::Skill,
        "Go",
        Some("go"),
        80..82,
        0.91,
    );

    {
        let store = MetaStore::open(&db_path).unwrap();
        store.run_migrations().unwrap();
        store.upsert_document(&document).unwrap();
        store.upsert_resume_version(&version).unwrap();
    }

    {
        let connection = open_raw_connection(&db_path);
        connection
            .execute("DELETE FROM schema_migrations WHERE version IN (6, 7)", [])
            .unwrap();
        connection
            .execute("DROP TABLE IF EXISTS ocr_page_cache", [])
            .unwrap();
        for mention in [&email, &phone, &skill] {
            connection
                .execute(
                    "\
                    INSERT INTO entity_mention (
                        id, resume_version_id, section_id, entity_type, raw_value,
                        normalized_value, span_start, span_end, confidence, extractor
                    )
                    VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        mention.id.as_str(),
                        mention.resume_version_id.as_str(),
                        match mention.entity_type {
                            EntityType::Email => "email",
                            EntityType::Phone => "phone",
                            EntityType::Skill => "skill",
                            _ => unreachable!("test only uses email, phone, and skill"),
                        },
                        mention.raw_value.as_str(),
                        mention.normalized_value.as_deref(),
                        mention.span_start.unwrap() as i64,
                        mention.span_end.unwrap() as i64,
                        f64::from(mention.confidence),
                        mention.extractor.as_str(),
                    ],
                )
                .unwrap();
        }

        let legacy_dump = raw_entity_mention_value_dump(&connection);
        assert!(legacy_dump.contains("Legacy.Candidate@Example.Test"));
        assert!(legacy_dump.contains("legacy.candidate@example.test"));
        assert!(legacy_dump.contains("(415) 555-0199"));
        assert!(legacy_dump.contains("+14155550199"));
    }

    {
        let reopened = MetaStore::open(&db_path).unwrap();
        let report = reopened.run_migrations().unwrap();
        assert_eq!(report.applied_versions(), &[6, 7]);
        assert_eq!(reopened.schema_version().unwrap(), 11);

        let mentions = reopened.entity_mentions_for_version(&version.id).unwrap();
        let email = mentions
            .iter()
            .find(|mention| mention.entity_type == EntityType::Email)
            .expect("email mention");
        assert_eq!(email.raw_value, "<redacted:email>");
        assert_eq!(email.normalized_value, None);
        assert_eq!(email.extractor, "rules-v1");
        assert_eq!(email.span_start, Some(9));
        assert_eq!(email.span_end, Some(38));

        let phone = mentions
            .iter()
            .find(|mention| mention.entity_type == EntityType::Phone)
            .expect("phone mention");
        assert_eq!(phone.raw_value, "<redacted:phone>");
        assert_eq!(phone.normalized_value, None);
        assert_eq!(phone.extractor, "rules-v1");
        assert_eq!(phone.span_start, Some(40));
        assert_eq!(phone.span_end, Some(54));

        assert!(mentions.iter().any(|mention| mention == &skill));
    }

    {
        let connection = open_raw_connection(&db_path);
        let raw_dump = raw_entity_mention_value_dump(&connection);
        assert!(raw_dump.contains("<redacted:email>"));
        assert!(raw_dump.contains("<redacted:phone>"));
        assert!(raw_dump.contains("Go"));
        assert!(raw_dump.contains("Some(\"go\")"));
        assert!(!raw_dump.contains("Legacy.Candidate"));
        assert!(!raw_dump.contains("legacy.candidate@example.test"));
        assert!(!raw_dump.contains("555-0199"));
        assert!(!raw_dump.contains("+14155550199"));
    }

    remove_temp_db(&db_path);
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
fn index_state_persists_and_upserts_snapshot_status() {
    let store = migrated_store();

    assert_eq!(store.index_state().unwrap(), None);

    let ready = IndexState {
        manifest_version: "manifest-v1".to_string(),
        snapshot_token: Some("snapshot-token-v1".to_string()),
        status: IndexStateStatus::Ready,
        updated_at: UnixTimestamp::from_unix_seconds(1_800_000_060),
    };
    let stale = IndexState {
        manifest_version: "manifest-v2".to_string(),
        snapshot_token: Some("snapshot-token-v2".to_string()),
        status: IndexStateStatus::Stale,
        updated_at: UnixTimestamp::from_unix_seconds(1_800_000_120),
    };

    store.upsert_index_state(&ready).unwrap();
    assert_eq!(store.index_state().unwrap(), Some(ready));

    store.upsert_index_state(&stale).unwrap();
    assert_eq!(store.index_state().unwrap(), Some(stale));
}

#[test]
fn status_summary_aggregates_documents_jobs_imports_and_index_state() {
    let store = migrated_store();
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
        ocr_required,
        embedding_waiting,
        deleted,
    ] {
        store.upsert_document(&document).unwrap();
    }

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
    store
        .insert_import_task(&import_task(
            "status-import-placeholder",
            "synthetic/import/root",
            ImportTaskStatus::Queued,
        ))
        .unwrap();
    store
        .upsert_index_state(&IndexState {
            manifest_version: "manifest-v1".to_string(),
            snapshot_token: Some("snapshot-v1".to_string()),
            status: IndexStateStatus::Building,
            updated_at: now,
        })
        .unwrap();

    let summary = store.status_summary().unwrap();

    assert_eq!(summary.indexed_documents, 2);
    assert_eq!(summary.searchable_documents, 1);
    assert_eq!(summary.partial_documents, 1);
    assert_eq!(summary.failed_retryable, 1);
    assert_eq!(summary.failed_permanent, 1);
    assert_eq!(summary.ocr_queue_depth, 1);
    assert_eq!(summary.embedding_queue_depth, 1);
    assert_eq!(summary.recovery_queue_depth, 1);
    assert_eq!(summary.import_tasks_queued, 1);
    assert_eq!(summary.import_tasks_recoverable, 0);
    assert_eq!(summary.index_health, IndexStateStatus::Building);
    assert_eq!(summary.last_snapshot_id.as_deref(), Some("snapshot-v1"));
}

#[test]
fn import_tasks_persist_without_document_foreign_key() {
    let db_path = temp_db_path("import-task-placeholder");
    let task = import_task(
        "import-reopen-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Queued,
    );

    {
        let store = MetaStore::open(&db_path).unwrap();
        store.run_migrations().unwrap();
        store.insert_import_task(&task).unwrap();
        assert_eq!(
            store.import_task_by_id(&task.id).unwrap(),
            Some(task.clone())
        );
    }

    {
        let reopened = MetaStore::open(&db_path).unwrap();
        reopened.run_migrations().unwrap();
        assert_eq!(reopened.schema_version().unwrap(), 11);
        assert_eq!(reopened.import_task_by_id(&task.id).unwrap(), Some(task));
        assert!(reopened.visible_documents().unwrap().is_empty());
    }

    remove_temp_db(&db_path);
}

#[test]
fn import_task_status_updates_support_completion_and_retry() {
    let store = migrated_store();
    let task = import_task(
        "import-status-update-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Queued,
    );
    store.insert_import_task(&task).unwrap();

    let started_at = UnixTimestamp::from_unix_seconds(1_800_000_010);
    store
        .update_import_task_status(&task.id, ImportTaskStatus::Running, started_at)
        .unwrap();
    let running = store.import_task_by_id(&task.id).unwrap().unwrap();
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
    store
        .update_import_task_status(&task.id, ImportTaskStatus::Running, restarted_at)
        .unwrap();
    let restarted = store.import_task_by_id(&task.id).unwrap().unwrap();
    assert_eq!(restarted.status, ImportTaskStatus::Running);
    assert_eq!(restarted.started_at, Some(restarted_at));
    assert_eq!(restarted.finished_at, None);

    let completed_at = UnixTimestamp::from_unix_seconds(1_800_000_040);
    store
        .update_import_task_status(&task.id, ImportTaskStatus::Completed, completed_at)
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
    store.insert_import_task(&time_travel_task).unwrap();
    let before_queue = UnixTimestamp::from_unix_seconds(1_799_999_999);
    assert_redacted_store_error(
        store
            .update_import_task_status(
                &time_travel_task.id,
                ImportTaskStatus::Running,
                before_queue,
            )
            .unwrap_err(),
    );

    let retryable_to_running_backwards =
        store.update_import_task_status(&task.id, ImportTaskStatus::Running, retry_at);
    assert_redacted_store_error(retryable_to_running_backwards.unwrap_err());
}

#[test]
fn import_worker_claim_atomically_marks_next_task_running_and_skips_attempted_tasks() {
    let store = migrated_store();
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

    store.insert_import_task(&running).unwrap();
    store.insert_import_task(&completed).unwrap();
    store.insert_import_task(&retryable).unwrap();
    store.insert_import_task(&queued).unwrap();

    let claim_at = UnixTimestamp::from_unix_seconds(1_900_000_000);
    let claimed = store
        .claim_next_import_task_for_worker(claim_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, queued.id.clone());
    assert_eq!(claimed.status, ImportTaskStatus::Running);
    assert_eq!(claimed.started_at, Some(claim_at));
    assert_eq!(
        store.import_task_by_id(&queued.id).unwrap().unwrap().status,
        ImportTaskStatus::Running
    );

    let claimed_retryable = store
        .claim_next_import_task_for_worker_excluding(claim_at, std::slice::from_ref(&queued.id))
        .unwrap()
        .unwrap();
    assert_eq!(claimed_retryable.id, retryable.id);
    assert_eq!(claimed_retryable.status, ImportTaskStatus::Running);
}

#[test]
fn import_worker_claim_respects_retryable_due_time_without_delaying_queued_tasks() {
    let store = migrated_store();
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

    store.insert_import_task(&retryable).unwrap();
    store.insert_import_task(&queued).unwrap();

    let claim_at = UnixTimestamp::from_unix_seconds(1_900_000_000);
    let retryable_not_due = UnixTimestamp::from_unix_seconds(1_799_999_999);
    let claimed = store
        .claim_next_import_task_for_worker_excluding_due_at(claim_at, retryable_not_due, &[])
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, queued.id);
    assert_eq!(claimed.status, ImportTaskStatus::Running);

    assert!(store
        .claim_next_import_task_for_worker_excluding_due_at(
            claim_at,
            retryable_not_due,
            std::slice::from_ref(&claimed.id),
        )
        .unwrap()
        .is_none());

    let retryable_due = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let claimed_retryable = store
        .claim_next_import_task_for_worker_excluding_due_at(
            claim_at,
            retryable_due,
            std::slice::from_ref(&claimed.id),
        )
        .unwrap()
        .unwrap();
    assert_eq!(claimed_retryable.id, retryable.id);
    assert_eq!(claimed_retryable.status, ImportTaskStatus::Running);
}

#[test]
fn stale_running_import_tasks_can_be_recovered_for_worker_retry() {
    let store = migrated_store();
    let started_at = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let fresh_updated_at = UnixTimestamp::from_unix_seconds(1_900_000_000);
    let recovered_at = UnixTimestamp::from_unix_seconds(2_000_000_000);
    let stale_cutoff = UnixTimestamp::from_unix_seconds(1_850_000_000);
    let mut stale_running = import_task(
        "stale-running",
        "synthetic/import/stale",
        ImportTaskStatus::Running,
    );
    stale_running.started_at = Some(started_at);
    let mut fresh_running = import_task(
        "fresh-running",
        "synthetic/import/fresh",
        ImportTaskStatus::Running,
    );
    fresh_running.started_at = Some(started_at);
    fresh_running.updated_at = fresh_updated_at;

    store.insert_import_task(&stale_running).unwrap();
    store.insert_import_task(&fresh_running).unwrap();

    let recovered = store
        .recover_stale_running_import_tasks(recovered_at, stale_cutoff)
        .unwrap();
    assert_eq!(recovered, 1);

    let stale = store.import_task_by_id(&stale_running.id).unwrap().unwrap();
    assert_eq!(stale.status, ImportTaskStatus::FailedRetryable);
    assert_eq!(stale.finished_at, Some(recovered_at));
    assert_eq!(stale.updated_at, recovered_at);
    let fresh = store.import_task_by_id(&fresh_running.id).unwrap().unwrap();
    assert_eq!(fresh.status, ImportTaskStatus::Running);
    assert_eq!(fresh.finished_at, None);
}

#[test]
fn running_import_task_heartbeat_prevents_stale_recovery() {
    let store = migrated_store();
    let started_at = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let heartbeat_at = UnixTimestamp::from_unix_seconds(1_900_000_000);
    let recovered_at = UnixTimestamp::from_unix_seconds(2_000_000_000);
    let stale_cutoff = UnixTimestamp::from_unix_seconds(1_850_000_000);
    let mut running = import_task(
        "heartbeat-running",
        "synthetic/import/heartbeat",
        ImportTaskStatus::Running,
    );
    running.started_at = Some(started_at);

    store.insert_import_task(&running).unwrap();

    assert!(store
        .heartbeat_running_import_task(&running.id, heartbeat_at)
        .unwrap());
    let recovered = store
        .recover_stale_running_import_tasks(recovered_at, stale_cutoff)
        .unwrap();
    assert_eq!(recovered, 0);

    let task = store.import_task_by_id(&running.id).unwrap().unwrap();
    assert_eq!(task.status, ImportTaskStatus::Running);
    assert_eq!(task.updated_at, heartbeat_at);
    assert_eq!(task.finished_at, None);
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
    assert_redacted_store_error(store.insert_import_task(&queued_with_started).unwrap_err());

    let mut completed_without_finish = import_task(
        "invalid-completed-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Completed,
    );
    completed_without_finish.started_at = Some(timestamp);
    assert_redacted_store_error(
        store
            .insert_import_task(&completed_without_finish)
            .unwrap_err(),
    );

    let mut running_with_finish = import_task(
        "invalid-running-finished-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Running,
    );
    running_with_finish.started_at = Some(timestamp);
    running_with_finish.finished_at = Some(timestamp);
    assert_redacted_store_error(store.insert_import_task(&running_with_finish).unwrap_err());
}

#[test]
fn existing_schema_v1_database_upgrades_to_v2_without_losing_documents() {
    let db_path = temp_db_path("v1-upgrade-placeholder");
    let document = document(
        "v1-upgrade-document-placeholder",
        false,
        DocumentStatus::Discovered,
    );
    let task = import_task(
        "v1-upgrade-import-placeholder",
        "synthetic/import/root",
        ImportTaskStatus::Queued,
    );

    {
        let store = MetaStore::open(&db_path).unwrap();
        store.run_migrations().unwrap();
        store.upsert_document(&document).unwrap();
    }

    {
        let connection = open_raw_connection(&db_path);
        connection.execute("DROP TABLE import_task", []).unwrap();
        connection.execute("DROP TABLE entity_mention", []).unwrap();
        connection.execute("DROP TABLE candidate", []).unwrap();
        connection.execute("DROP TABLE ocr_page_cache", []).unwrap();
        connection
            .execute("DROP TABLE worker_task_control", [])
            .unwrap();
        connection
            .execute("DROP TABLE import_scan_scope", [])
            .unwrap();
        connection
            .execute("DROP TABLE import_scan_error", [])
            .unwrap();
        connection
            .execute(
                "DROP INDEX IF EXISTS ingest_job_ocr_document_unique_idx",
                [],
            )
            .unwrap();
        connection
            .execute("DROP INDEX IF EXISTS resume_version_candidate_idx", [])
            .unwrap();
        connection
            .execute(
                "DELETE FROM schema_migrations WHERE version IN (2, 3, 4, 5, 6, 7, 8, 9, 10, 11)",
                [],
            )
            .unwrap();
    }

    {
        let reopened = MetaStore::open(&db_path).unwrap();
        let report = reopened.run_migrations().unwrap();
        assert_eq!(report.applied_versions(), &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
        assert_eq!(reopened.schema_version().unwrap(), 11);
        assert_eq!(
            reopened.document_by_id(&document.id).unwrap(),
            Some(document)
        );

        reopened.insert_import_task(&task).unwrap();
        assert_eq!(reopened.import_task_by_id(&task.id).unwrap(), Some(task));
    }

    remove_temp_db(&db_path);
}

#[test]
fn file_backed_store_reopens_schema_and_index_state() {
    let db_path = temp_db_path("file-backed-placeholder");
    let state = IndexState {
        manifest_version: "manifest-file-v1".to_string(),
        snapshot_token: Some("snapshot-file-token-v1".to_string()),
        status: IndexStateStatus::Ready,
        updated_at: UnixTimestamp::from_unix_seconds(1_800_000_180),
    };

    {
        let store = MetaStore::open(&db_path).unwrap();
        store.run_migrations().unwrap();
        store.upsert_index_state(&state).unwrap();
    }

    {
        let reopened = MetaStore::open(&db_path).unwrap();
        assert!(reopened.foreign_keys_enabled().unwrap());
        assert_eq!(reopened.schema_version().unwrap(), 11);
        assert_eq!(reopened.index_state().unwrap(), Some(state));
    }

    remove_temp_db(&db_path);
}

#[test]
fn file_backed_connection_sets_pragmas() {
    let db_path = temp_db_path("pragma-placeholder");
    {
        let store = MetaStore::open(&db_path).unwrap();

        assert!(store.foreign_keys_enabled().unwrap());
        assert_eq!(store.busy_timeout_millis().unwrap(), 5_000);
        assert_eq!(store.journal_mode().unwrap(), "wal");
    }

    remove_temp_db(&db_path);
}

#[test]
fn raw_sql_invalid_enum_and_quality_values_are_rejected() {
    let db_path = temp_db_path("checks-placeholder");
    {
        let store = MetaStore::open(&db_path).unwrap();
        store.run_migrations().unwrap();
        let document = document(
            "checks-document-placeholder",
            false,
            DocumentStatus::Discovered,
        );
        store.upsert_document(&document).unwrap();
    }

    {
        let connection = open_raw_connection(&db_path);
        let valid_document_id =
            DocumentId::from_non_secret_parts(&["s3", "checks-document-placeholder"]);
        let valid_version_id =
            ResumeVersionId::from_non_secret_parts(&["s3", "checks-version-valid"]);

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO document (
                id, source_uri, normalized_path, file_name, extension, byte_size, mtime_seconds,
                is_deleted, created_at_seconds, updated_at_seconds, status
            )
            VALUES (?1, 'synthetic://invalid', 'synthetic/invalid.txt', 'invalid.txt', 'txt',
                1, 1, 0, 1, 1, 'not_a_status')",
            params![DocumentId::from_non_secret_parts(&["s3", "invalid-document"]).as_str()],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO resume_version (
                id, document_id, parse_version, schema_version, language_set_json,
                quality_score, visibility
            )
            VALUES (?1, ?2, 'parser-v1', 'schema-v1', '[]', 1.5, 'searchable')",
            params![valid_version_id.as_str(), valid_document_id.as_str()],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO resume_version (
                id, document_id, parse_version, schema_version, language_set_json, visibility
            )
            VALUES (?1, ?2, 'parser-v1', 'schema-v1', '[]', 'not_visibility')",
            params![
                ResumeVersionId::from_non_secret_parts(&["s3", "invalid-visibility"]).as_str(),
                valid_document_id.as_str(),
            ],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO ingest_job (
                id, document_id, kind, status, attempt_count, max_attempts,
                queued_at_seconds, updated_at_seconds
            )
            VALUES (?1, ?2, 'not_kind', 'queued', 0, 3, 1, 1)",
            params![
                IngestJobId::from_non_secret_parts(&["s3", "invalid-kind"]).as_str(),
                valid_document_id.as_str(),
            ],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO ingest_job (
                id, document_id, kind, status, attempt_count, max_attempts,
                queued_at_seconds, updated_at_seconds
            )
            VALUES (?1, ?2, 'parse_document', 'not_status', 0, 3, 1, 1)",
            params![
                IngestJobId::from_non_secret_parts(&["s3", "invalid-job-status"]).as_str(),
                valid_document_id.as_str(),
            ],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO index_state (state_key, manifest_version, status, updated_at_seconds)
            VALUES ('default', 'manifest-invalid', 'not_index_status', 1)",
            [],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO import_task (
                id, root_path, status, queued_at_seconds, updated_at_seconds
            )
            VALUES (?1, 'synthetic/import/root', 'not_import_status', 1, 1)",
            params![ImportTaskId::from_non_secret_parts(&["s4", "invalid-import"]).as_str()],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO import_task (
                id, root_path, status, queued_at_seconds, started_at_seconds, updated_at_seconds
            )
            VALUES (?1, 'synthetic/import/root', 'queued', 1, 1, 1)",
            params![
                ImportTaskId::from_non_secret_parts(&["s4", "invalid-queued-started"]).as_str()
            ],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO import_task (
                id, root_path, status, queued_at_seconds, started_at_seconds, updated_at_seconds
            )
            VALUES (?1, 'synthetic/import/root', 'completed', 1, 2, 3)",
            params![
                ImportTaskId::from_non_secret_parts(&["s4", "invalid-completed-missing-finished"])
                    .as_str()
            ],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO import_task (
                id, root_path, status, queued_at_seconds, started_at_seconds,
                finished_at_seconds, updated_at_seconds
            )
            VALUES (?1, 'synthetic/import/root', 'running', 1, 2, 2, 3)",
            params![
                ImportTaskId::from_non_secret_parts(&["s4", "invalid-running-finished"]).as_str()
            ],
        ));

        expect_raw_rejection(connection.execute(
            "\
            INSERT INTO import_task (
                id, root_path, status, queued_at_seconds, started_at_seconds,
                finished_at_seconds, updated_at_seconds
            )
            VALUES (?1, 'synthetic/import/root', 'completed', 3, 2, 4, 4)",
            params![
                ImportTaskId::from_non_secret_parts(&["s4", "invalid-timestamp-order"]).as_str()
            ],
        ));
    }

    remove_temp_db(&db_path);
}

#[test]
fn foreign_keys_reject_missing_parents_and_delete_cascades_children() {
    let db_path = temp_db_path("fk-placeholder");
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

    {
        let store = MetaStore::open(&db_path).unwrap();
        store.run_migrations().unwrap();
        let missing_parent = resume_version(
            "fk-missing-parent-placeholder",
            DocumentId::from_non_secret_parts(&["s3", "missing-parent"]),
        );
        assert_redacted_store_error(store.upsert_resume_version(&missing_parent).unwrap_err());

        store.upsert_document(&document).unwrap();
        store.upsert_resume_version(&version).unwrap();
        store.insert_ingest_job(&ingest_job).unwrap();
    }

    {
        let connection = open_raw_connection(&db_path);
        connection
            .execute(
                "DELETE FROM document WHERE id = ?1",
                params![document.id.as_str()],
            )
            .unwrap();
    }

    {
        let reopened = MetaStore::open(&db_path).unwrap();
        assert_eq!(reopened.document_by_id(&document.id).unwrap(), None);
        assert_eq!(reopened.resume_version_by_id(&version.id).unwrap(), None);
        assert_eq!(reopened.ingest_job_by_id(&ingest_job.id).unwrap(), None);
    }

    remove_temp_db(&db_path);
}

#[test]
fn file_backed_store_recovers_unfinished_jobs_after_reopen() {
    let db_path = temp_db_path("recovery-reopen-placeholder");
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
        let store = MetaStore::open(&db_path).unwrap();
        store.run_migrations().unwrap();
        store.upsert_document(&document).unwrap();
        store.upsert_resume_version(&version).unwrap();
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
        let reopened = MetaStore::open(&db_path).unwrap();
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

    remove_temp_db(&db_path);
}

fn migrated_store() -> MetaStore {
    let store = MetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    store
}

fn temp_db_path(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    std::env::temp_dir().join(format!("resume-ir-s3-{label}-{unique}.sqlite3"))
}

fn remove_temp_db(db_path: &PathBuf) {
    let _ = fs::remove_file(db_path);
    let _ = fs::remove_file(format!("{}-wal", db_path.display()));
    let _ = fs::remove_file(format!("{}-shm", db_path.display()));
}

fn document(label: &str, is_deleted: bool, status: DocumentStatus) -> Document {
    let now = UnixTimestamp::from_unix_seconds(1_800_000_000);
    let id = DocumentId::from_non_secret_parts(&["s3", label]);

    Document {
        id,
        source_uri: format!("synthetic://document/{label}"),
        normalized_path: format!("synthetic/root/{label}.txt"),
        file_name: format!("{label}.txt"),
        extension: FileExtension::Txt,
        byte_size: 128,
        mtime: now,
        content_hash: Some(format!("sha256:SYNTHETIC_CONTENT_HASH_{label}")),
        text_hash: Some(format!("sha256:SYNTHETIC_TEXT_HASH_{label}")),
        is_deleted,
        created_at: now,
        updated_at: now,
        status,
    }
}

fn resume_version(label: &str, document_id: DocumentId) -> ResumeVersion {
    ResumeVersion {
        id: ResumeVersionId::from_non_secret_parts(&["s3", label]),
        document_id,
        candidate_id: None,
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v1".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some("SYNTHETIC RAW TEXT".to_string()),
        clean_text: Some("SYNTHETIC CLEAN TEXT".to_string()),
        quality_score: Some(0.8),
        visibility: ResumeVisibility::Searchable,
    }
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

trait IngestJobTestExt {
    fn started_at(self, timestamp: UnixTimestamp) -> Self;
    fn finished_at(self, timestamp: UnixTimestamp) -> Self;
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

    fn resume_version_id(mut self, id: ResumeVersionId) -> Self {
        self.resume_version_id = Some(id);
        self
    }
}

fn open_raw_connection(db_path: &PathBuf) -> Connection {
    let connection = Connection::open(db_path).unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .unwrap();
    connection
}

fn raw_entity_mention_value_dump(connection: &Connection) -> String {
    connection
        .prepare("SELECT raw_value, normalized_value FROM entity_mention ORDER BY rowid")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .iter()
        .map(|(raw, normalized)| format!("{raw} {normalized:?}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn expect_raw_rejection(result: rusqlite::Result<usize>) {
    assert!(result.is_err());
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
