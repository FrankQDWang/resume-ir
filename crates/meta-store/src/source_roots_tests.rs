use rusqlite::params;

use crate::{
    source_roots::SOURCE_ROOT_CLASSIFICATION_COUNTS_SQL, BeginScanOutcome, ClassificationCounts,
    ClassificationStatus, ContentDigest, Document, DocumentId, DocumentStatus, EphemeralMetaStore,
    FileExtension, ImportProcessingContract, ImportRootKind, ImportScanProfile, ImportScanScope,
    ImportTask, ImportTaskId, ImportTaskStatus, OccurrenceChange, ReasonCode, ResumeVersion,
    ResumeVersionClassification, ResumeVersionId, ReviewDisposition, ScanCounts, ScanPhase,
    ScanTrigger, SourceRevision, SourceRootScanCoordination, SourceRootState, SourceWatcherState,
    UnixTimestamp, CLASSIFIER_EPOCH,
};

#[test]
fn source_root_classification_counts_seek_resume_versions_by_document() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "source-root-query-plan-parser",
        "source-root-query-plan-ocr",
        "source-root-query-plan-schema",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    let source_triage_epoch = contract.source_triage_epoch();
    let explain = format!("EXPLAIN QUERY PLAN {SOURCE_ROOT_CLASSIFICATION_COUNTS_SQL}");
    let connection = store.connection.borrow();
    let mut statement = connection.prepare(&explain).unwrap();
    let details = statement
        .query_map(
            params![
                "root-query-plan",
                CLASSIFIER_EPOCH,
                source_triage_epoch.as_str()
            ],
            |row| row.get::<_, String>(3),
        )
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let indexed_seeks = details
        .iter()
        .filter(|detail| {
            detail
                .contains("SEARCH version USING INDEX resume_version_document_idx (document_id=?)")
        })
        .count();
    assert_eq!(indexed_seeks, 4, "query plan: {details:?}");
}

#[test]
fn source_root_classification_counts_only_include_the_current_source_revision() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_900_199_990);
    let root = store
        .register_source_root(
            "/synthetic/classification-counts",
            "/synthetic/classification-counts",
            "Classification counts",
            now,
        )
        .unwrap();
    store
        .begin_scan(
            &root.id,
            "classification-counts-scan",
            ScanTrigger::Manual,
            now,
        )
        .unwrap();

    let current_source = b"current synthetic source";
    let prior_source = b"prior synthetic source";
    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&["source-root-classification-counts"]),
        source_uri: "synthetic://classification-counts/document.txt".to_string(),
        normalized_path: "synthetic/classification-counts/document.txt".to_string(),
        file_name: "document.txt".to_string(),
        extension: FileExtension::Txt,
        byte_size: current_source.len() as u64,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::FieldsExtracted,
    };
    let prior_revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(prior_source),
        prior_source.len() as u64,
    );
    let current_revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(current_source),
        current_source.len() as u64,
    );
    document.content_hash = Some(current_revision.content_hash.as_str().to_string());
    store.upsert_document(&document).unwrap();
    store.insert_source_revision(&prior_revision).unwrap();
    store.insert_source_revision(&current_revision).unwrap();

    let prior_version = classification_counts_version(&document, &prior_revision, "prior");
    let current_version = classification_counts_version(&document, &current_revision, "current");
    store.insert_resume_version(&prior_version).unwrap();
    store.insert_resume_version(&current_version).unwrap();
    store
        .insert_resume_version_classification(&ResumeVersionClassification {
            resume_version_id: prior_version.id,
            status: ClassificationStatus::NonResume,
            classifier_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::CorroboratedNonResumeSignals],
            classified_at: now,
            review_disposition: ReviewDisposition::NotRequired,
        })
        .unwrap();
    store
        .insert_resume_version_classification(&ResumeVersionClassification {
            resume_version_id: current_version.id,
            status: ClassificationStatus::ResumeCandidate,
            classifier_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
            classified_at: now,
            review_disposition: ReviewDisposition::NotRequired,
        })
        .unwrap();
    assert_eq!(
        store
            .observe_source_occurrence(
                &root.id,
                "document.txt",
                &document.id,
                &current_revision.id,
                "classification-counts-scan",
                now,
            )
            .unwrap(),
        OccurrenceChange::Inserted
    );

    let contract = ImportProcessingContract::new(
        "source-root-counts-parser",
        "source-root-counts-ocr",
        "source-root-counts-schema",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    assert_eq!(
        store
            .source_root_classification_counts(&root.id, CLASSIFIER_EPOCH)
            .unwrap(),
        ClassificationCounts {
            resume_candidate: 1,
            non_resume: 0,
            needs_review: 0,
            ocr_backlog: 0,
            failed: 0,
        }
    );
}

fn classification_counts_version(
    document: &Document,
    revision: &SourceRevision,
    label: &str,
) -> ResumeVersion {
    let normalized_text_hash =
        ContentDigest::from_bytes(format!("synthetic normalized {label}").as_bytes());
    ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &revision.id,
            &normalized_text_hash,
            "synthetic-parser",
            "synthetic-schema",
        ),
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: "synthetic-parser".to_string(),
        schema_version: "synthetic-schema".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: None,
        clean_text: Some(format!("synthetic normalized {label}")),
        quality_score: Some(0.9),
    }
}

#[test]
fn source_root_write_transactions_release_the_connection_before_result_reads() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let registered_at = UnixTimestamp::from_unix_seconds(1_900_200_000);
    let root = store
        .register_source_root(
            "/synthetic/source-root",
            "/synthetic/source-root",
            "Synthetic source root",
            registered_at,
        )
        .unwrap();
    assert_eq!(store.source_root(&root.id).unwrap(), Some(root.clone()));

    store
        .set_source_root_state(
            &root.id,
            SourceRootState::Active,
            SourceWatcherState::Paused,
            UnixTimestamp::from_unix_seconds(1_900_200_001),
        )
        .unwrap();
    let resumed = store
        .resume_source_root_monitoring(&root.id, UnixTimestamp::from_unix_seconds(1_900_200_002))
        .unwrap();
    assert_eq!(resumed.watcher_state, SourceWatcherState::Active);

    let scan = store
        .begin_scan(
            &root.id,
            "synthetic-scan",
            ScanTrigger::Manual,
            UnixTimestamp::from_unix_seconds(1_900_200_003),
        )
        .unwrap();
    assert!(matches!(scan, BeginScanOutcome::Started(_)));
}

#[test]
fn source_root_retry_source_change_supersedes_failed_task_without_reusing_scan_identity() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "retry-primary-v1",
        "retry-ocr-v1",
        "retry-derived-v1",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    let first_at = UnixTimestamp::from_unix_seconds(1_900_300_000);
    store
        .activate_migration_rebuild_contract(&contract, first_at)
        .unwrap();
    let root = store
        .register_source_root(
            "/synthetic/retryable-source-root",
            "/synthetic/retryable-source-root",
            "Retryable source root",
            first_at,
        )
        .unwrap();
    let first_task = queued_task("first-scan", &root.canonical_path, first_at);
    let first_scope = scan_scope(&first_task);
    assert!(matches!(
        store
            .coordinate_source_root_scan(
                &root.id,
                ScanTrigger::Manual,
                &first_task,
                &first_scope,
                &contract,
                first_at,
            )
            .unwrap(),
        SourceRootScanCoordination::Started { .. }
    ));

    let running = store
        .claim_observed_import_task_for_worker(
            &first_task,
            UnixTimestamp::from_unix_seconds(1_900_300_001),
        )
        .unwrap()
        .unwrap();
    let failed_at = UnixTimestamp::from_unix_seconds(1_900_300_002);
    store
        .update_import_task_status(&running.id, ImportTaskStatus::FailedRetryable, failed_at)
        .unwrap();
    store
        .fail_or_partial_scan(
            &root.id,
            running.id.as_str(),
            ScanCounts::default(),
            ScanPhase::Failed,
            failed_at,
        )
        .unwrap();

    let retry_at = UnixTimestamp::from_unix_seconds(1_900_300_003);
    let requested_retry = queued_task("requested-retry", &root.canonical_path, retry_at);
    let retry_scope = scan_scope(&requested_retry);
    let SourceRootScanCoordination::Started {
        snapshot,
        task_head: _,
    } = store
        .coordinate_source_root_scan(
            &root.id,
            ScanTrigger::Watcher,
            &requested_retry,
            &retry_scope,
            &contract,
            retry_at,
        )
        .unwrap()
    else {
        panic!("a new source change must start a new scan");
    };
    assert_eq!(snapshot.id, requested_retry.id.as_str());
    assert_eq!(snapshot.phase, ScanPhase::Queued);
    assert_eq!(snapshot.started_at, retry_at);
    assert_eq!(
        store
            .import_task_by_id(&first_task.id)
            .unwrap()
            .unwrap()
            .status,
        ImportTaskStatus::FailedRetryable
    );
}

#[test]
fn source_root_retry_background_claim_restarts_the_failed_scan_snapshot() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let contract = ImportProcessingContract::new(
        "retry-primary-v1",
        "retry-ocr-v1",
        "retry-derived-v1",
        CLASSIFIER_EPOCH,
    )
    .unwrap();
    let first_at = UnixTimestamp::from_unix_seconds(1_900_400_000);
    store
        .activate_migration_rebuild_contract(&contract, first_at)
        .unwrap();
    let root = store
        .register_source_root(
            "/synthetic/background-retry-source-root",
            "/synthetic/background-retry-source-root",
            "Background retry source root",
            first_at,
        )
        .unwrap();
    let first_task = queued_task("background-retry", &root.canonical_path, first_at);
    let first_scope = scan_scope(&first_task);
    store
        .coordinate_source_root_scan(
            &root.id,
            ScanTrigger::Manual,
            &first_task,
            &first_scope,
            &contract,
            first_at,
        )
        .unwrap();
    let running = store
        .claim_observed_import_task_for_worker(
            &first_task,
            UnixTimestamp::from_unix_seconds(1_900_400_001),
        )
        .unwrap()
        .unwrap();
    let failed_at = UnixTimestamp::from_unix_seconds(1_900_400_002);
    store
        .update_import_task_status(&running.id, ImportTaskStatus::FailedRetryable, failed_at)
        .unwrap();
    store
        .fail_or_partial_scan(
            &root.id,
            running.id.as_str(),
            ScanCounts::default(),
            ScanPhase::Failed,
            failed_at,
        )
        .unwrap();

    let retry_at = UnixTimestamp::from_unix_seconds(1_900_400_003);
    let retained = store.import_task_by_id(&first_task.id).unwrap().unwrap();
    let claimed = store
        .claim_observed_import_task_for_worker(&retained, retry_at)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, first_task.id);
    let restarted = store.latest_scan_snapshot(&root.id).unwrap().unwrap();
    assert_eq!(restarted.id, first_task.id.as_str());
    assert_eq!(restarted.phase, ScanPhase::Queued);
    assert_eq!(restarted.started_at, retry_at);
}

fn queued_task(label: &str, root: &str, now: UnixTimestamp) -> ImportTask {
    ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["source-root-retry", label]),
        root_path: root.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    }
}

fn scan_scope(task: &ImportTask) -> ImportScanScope {
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
