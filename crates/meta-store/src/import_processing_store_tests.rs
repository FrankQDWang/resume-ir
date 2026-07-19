use rusqlite::params;

use crate::{
    ClassificationStatus, ContentDigest, Document, DocumentId, DocumentStatus, EntityMention,
    EntityMentionId, EntityType, EphemeralMetaStore, FileExtension, IdentityInsertOutcome,
    ImportProcessingContract, ImportRootKind, ImportRootTaskHeadOutcome, ImportScanProfile,
    ImportScanScope, ImportSourceDispositionKind, ImportTask, ImportTaskId, ImportTaskPurpose,
    ImportTaskSourceDisposition, ImportTaskStatus, MigrationRebuildContractActivation, ReasonCode,
    ResumeVersion, ResumeVersionClassification, ResumeVersionId, ReviewDisposition,
    SearchProjectionServiceState, SearchRepairReason, SourceRevision, UnixTimestamp,
    CLASSIFIER_EPOCH,
};

#[test]
fn blocked_runtime_invariant_contract_hard_cut_restarts_only_derived_rebuild_state() {
    let store = migrated_store();
    let previous_contract = processing_contract("parser-v1");
    let replacement_contract = processing_contract("parser-v2");
    let initial_at = UnixTimestamp::from_unix_seconds(1_900_099_990);
    assert_eq!(
        store
            .activate_migration_rebuild_contract(&previous_contract, initial_at)
            .unwrap(),
        MigrationRebuildContractActivation::Activated
    );
    set_unpublished_visible_epoch(&store, 7);

    let immutable = immutable_fixture("blocked-hard-cut");
    assert_eq!(store.upsert_document(&immutable.document).unwrap(), ());
    assert_eq!(
        store.insert_source_revision(&immutable.revision).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store.insert_resume_version(&immutable.version).unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store
            .insert_resume_version_classification(&immutable.classification)
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );
    assert_eq!(
        store
            .insert_entity_mentions(
                &immutable.version.id,
                std::slice::from_ref(&immutable.mention),
            )
            .unwrap(),
        IdentityInsertOutcome::Inserted
    );

    let root_path = "synthetic/import/blocked-hard-cut";
    let queued_at = UnixTimestamp::from_unix_seconds(1_900_100_000);
    let task = import_task("blocked-hard-cut", root_path, queued_at);
    let mut scope = import_scope(&task, 1);
    scope.searchable_documents = 1;
    store
        .insert_import_task_with_scan_scope(&task, &scope, &previous_contract)
        .unwrap();
    mark_migration_rebuild_task(&store, &task, &previous_contract);
    let running = store
        .claim_observed_import_task_for_worker(
            &task,
            UnixTimestamp::from_unix_seconds(1_900_100_001),
        )
        .unwrap()
        .unwrap();
    store
        .stage_import_task_source_dispositions(
            &running.id,
            previous_contract.id(),
            &[ImportTaskSourceDisposition {
                source_ordinal: 0,
                document_id: immutable.document.id.clone(),
                source_revision_id: immutable.revision.id.clone(),
                resume_version_id: Some(immutable.version.id.clone()),
                kind: ImportSourceDispositionKind::Searchable,
            }],
        )
        .unwrap();
    scope.updated_at = UnixTimestamp::from_unix_seconds(1_900_100_002);
    store
        .complete_import_task(
            &running.id,
            previous_contract.id(),
            &scope,
            scope.updated_at,
        )
        .unwrap();
    seed_publication_attempt(&store, &previous_contract);
    assert_eq!(
        store
            .block_migration_rebuild(
                SearchRepairReason::RuntimeInvariant,
                UnixTimestamp::from_unix_seconds(1_900_100_003),
            )
            .unwrap(),
        crate::SearchProjectionTransitionOutcome::Applied
    );

    assert_eq!(
        store
            .activate_migration_rebuild_contract(
                &replacement_contract,
                UnixTimestamp::from_unix_seconds(1_900_100_004),
            )
            .unwrap(),
        MigrationRebuildContractActivation::Activated
    );

    let state = store.search_projection_state().unwrap();
    assert_eq!(state.service_state, SearchProjectionServiceState::Repairing);
    assert_eq!(
        state.repair_reason,
        Some(SearchRepairReason::MigrationRebuild)
    );
    assert_eq!(state.generation, None);
    assert_eq!(state.visible_epoch, 7);
    assert_eq!(
        active_contract_id(&store).as_deref(),
        Some(replacement_contract.id().as_str())
    );
    assert_eq!(row_count(&store, "import_task_completion"), 0);
    assert_eq!(row_count(&store, "import_task_source_disposition"), 0);
    assert_eq!(row_count(&store, "import_task"), 0);
    assert_eq!(
        row_count(&store, "migration_rebuild_publication_attempt"),
        0
    );
    assert_eq!(
        store.active_authorized_import_roots().unwrap(),
        vec![root_path.to_string()]
    );
    assert_eq!(
        store.source_revision_by_id(&immutable.revision.id).unwrap(),
        Some(immutable.revision.clone())
    );
    assert_eq!(
        store.resume_version_by_id(&immutable.version.id).unwrap(),
        Some(immutable.version.clone())
    );
    assert_eq!(
        store
            .resume_version_classification(&immutable.version.id, CLASSIFIER_EPOCH)
            .unwrap(),
        Some(immutable.classification)
    );
    assert_eq!(
        store
            .entity_mentions_for_version(&immutable.version.id)
            .unwrap(),
        vec![immutable.mention]
    );
}

#[test]
fn blocked_contract_is_not_reopened_without_an_exact_different_active_contract() {
    for active_contract in [None, Some(processing_contract("parser-v1"))] {
        let store = migrated_store();
        if let Some(contract) = active_contract.as_ref() {
            assert_eq!(
                store
                    .activate_migration_rebuild_contract(
                        contract,
                        UnixTimestamp::from_unix_seconds(1_900_100_000),
                    )
                    .unwrap(),
                MigrationRebuildContractActivation::Activated
            );
        }
        store
            .block_migration_rebuild(
                SearchRepairReason::RuntimeInvariant,
                UnixTimestamp::from_unix_seconds(1_900_100_001),
            )
            .unwrap();
        let requested = active_contract
            .clone()
            .unwrap_or_else(|| processing_contract("parser-v2"));

        assert_eq!(
            store
                .activate_migration_rebuild_contract(
                    &requested,
                    UnixTimestamp::from_unix_seconds(1_900_100_002),
                )
                .unwrap(),
            MigrationRebuildContractActivation::Superseded
        );
        assert_blocked(&store, SearchRepairReason::RuntimeInvariant, None);
    }
}

#[test]
fn source_unavailable_block_never_reopens_for_a_new_contract() {
    let store = blocked_store(SearchRepairReason::SourceUnavailable);
    let replacement = processing_contract("parser-v2");

    assert_eq!(
        store
            .activate_migration_rebuild_contract(
                &replacement,
                UnixTimestamp::from_unix_seconds(1_900_100_003),
            )
            .unwrap(),
        MigrationRebuildContractActivation::Superseded
    );
    assert_blocked(&store, SearchRepairReason::SourceUnavailable, None);
    assert!(store
        .import_processing_contract(replacement.id())
        .unwrap()
        .is_none());
}

#[test]
fn ready_or_generation_bearing_heads_never_reopen_for_a_new_contract() {
    for (service_state, repair_reason) in [
        ("ready", None),
        ("repair_blocked", Some("runtime_invariant")),
    ] {
        let store = migrated_store();
        let previous = processing_contract("parser-v1");
        store
            .activate_migration_rebuild_contract(
                &previous,
                UnixTimestamp::from_unix_seconds(1_900_100_000),
            )
            .unwrap();
        force_generation_bearing_state(&store, service_state, repair_reason);
        let replacement = processing_contract("parser-v2");

        assert_eq!(
            store
                .activate_migration_rebuild_contract(
                    &replacement,
                    UnixTimestamp::from_unix_seconds(1_900_100_001),
                )
                .unwrap(),
            MigrationRebuildContractActivation::Superseded
        );
        assert_eq!(raw_projection_state(&store).0, service_state);
        assert_eq!(
            raw_projection_state(&store).1.as_deref(),
            Some("synthetic-generation")
        );
        assert_eq!(raw_projection_state(&store).2.as_deref(), repair_reason);
        assert_eq!(
            active_contract_id(&store).as_deref(),
            Some(previous.id().as_str())
        );
        assert!(store
            .import_processing_contract(replacement.id())
            .unwrap()
            .is_none());
    }
}

#[test]
fn blocked_contract_hard_cut_never_deletes_a_running_task() {
    let store = blocked_store_with_running_task();
    let previous = processing_contract("parser-v1");
    let replacement = processing_contract("parser-v2");

    assert_eq!(
        store
            .activate_migration_rebuild_contract(
                &replacement,
                UnixTimestamp::from_unix_seconds(1_900_100_003),
            )
            .unwrap(),
        MigrationRebuildContractActivation::RunningTaskConflict
    );
    assert_blocked(&store, SearchRepairReason::RuntimeInvariant, None);
    assert_eq!(
        active_contract_id(&store).as_deref(),
        Some(previous.id().as_str())
    );
    assert_eq!(row_count(&store, "import_task"), 1);
    assert!(store
        .import_processing_contract(replacement.id())
        .unwrap()
        .is_none());
}

#[test]
fn competing_contract_cannot_delete_a_running_task() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let queued_at = UnixTimestamp::from_unix_seconds(1_900_100_000);
    let contract_a = processing_contract("parser-a");
    let contract_b = processing_contract("parser-b");
    assert_eq!(
        store
            .activate_migration_rebuild_contract(&contract_a, queued_at)
            .unwrap(),
        MigrationRebuildContractActivation::Activated
    );

    let root_path = "synthetic/import/processing-contract-race";
    let root_seed = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["processing-contract-race", "root-seed"]),
        root_path: root_path.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at,
        started_at: None,
        finished_at: None,
        updated_at: queued_at,
    };
    let root_seed_scope = ImportScanScope {
        import_task_id: root_seed.id.clone(),
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
        updated_at: queued_at,
    };
    store
        .insert_import_task_with_scan_scope(&root_seed, &root_seed_scope, &contract_a)
        .unwrap();
    assert!(store.cancel_import_task(&root_seed.id, queued_at).unwrap());

    let running_id = ImportTaskId::from_non_secret_parts(&["processing-contract-race", "running"]);
    assert!(matches!(
        store
            .enqueue_full_corpus_migration_rebuild_root(
                root_path,
                &running_id,
                &contract_a,
                queued_at,
            )
            .unwrap(),
        ImportRootTaskHeadOutcome::HeadInserted {
            purpose: ImportTaskPurpose::MigrationRebuildFullCorpus,
            ..
        }
    ));
    let queued = store.import_task_by_id(&running_id).unwrap().unwrap();
    assert_eq!(
        queued,
        ImportTask {
            id: running_id,
            root_path: "synthetic/import/processing-contract-race".to_string(),
            status: ImportTaskStatus::Queued,
            queued_at,
            started_at: None,
            finished_at: None,
            updated_at: queued_at,
        }
    );
    let running_at = UnixTimestamp::from_unix_seconds(1_900_100_001);
    let running = store
        .claim_observed_import_task_for_worker(&queued, running_at)
        .unwrap()
        .unwrap();

    assert_eq!(
        store
            .activate_migration_rebuild_contract(
                &contract_b,
                UnixTimestamp::from_unix_seconds(1_900_100_002),
            )
            .unwrap(),
        MigrationRebuildContractActivation::RunningTaskConflict
    );
    assert_eq!(
        store.import_task_by_id(&running.id).unwrap(),
        Some(running.clone())
    );
    assert_eq!(
        store
            .import_task_processing_contract_id(&running.id)
            .unwrap()
            .as_ref(),
        Some(contract_a.id())
    );
    assert!(
        store
            .import_processing_contract(contract_b.id())
            .unwrap()
            .is_none(),
        "rejected activation must roll back the competing contract insert"
    );
    assert_eq!(
        store
            .activate_migration_rebuild_contract(
                &contract_a,
                UnixTimestamp::from_unix_seconds(1_900_100_003),
            )
            .unwrap(),
        MigrationRebuildContractActivation::AlreadyActive
    );

    let failed_at = UnixTimestamp::from_unix_seconds(1_900_100_004);
    store
        .update_import_task_status(&running.id, ImportTaskStatus::FailedRetryable, failed_at)
        .unwrap();
    assert_eq!(
        store
            .activate_migration_rebuild_contract(
                &contract_b,
                UnixTimestamp::from_unix_seconds(1_900_100_005),
            )
            .unwrap(),
        MigrationRebuildContractActivation::Activated
    );
    assert_eq!(store.import_task_by_id(&running.id).unwrap(), None);
}

struct ImmutableFixture {
    document: Document,
    revision: SourceRevision,
    version: ResumeVersion,
    classification: ResumeVersionClassification,
    mention: EntityMention,
}

fn migrated_store() -> EphemeralMetaStore {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    store
}

fn blocked_store(reason: SearchRepairReason) -> EphemeralMetaStore {
    let store = migrated_store();
    let contract = processing_contract("parser-v1");
    store
        .activate_migration_rebuild_contract(
            &contract,
            UnixTimestamp::from_unix_seconds(1_900_100_000),
        )
        .unwrap();
    store
        .block_migration_rebuild(reason, UnixTimestamp::from_unix_seconds(1_900_100_001))
        .unwrap();
    store
}

fn blocked_store_with_running_task() -> EphemeralMetaStore {
    let store = migrated_store();
    let contract = processing_contract("parser-v1");
    let queued_at = UnixTimestamp::from_unix_seconds(1_900_100_000);
    store
        .activate_migration_rebuild_contract(&contract, queued_at)
        .unwrap();
    let task = import_task(
        "blocked-running-task",
        "synthetic/import/blocked-running-task",
        queued_at,
    );
    store
        .insert_import_task_with_scan_scope(&task, &import_scope(&task, 0), &contract)
        .unwrap();
    mark_migration_rebuild_task(&store, &task, &contract);
    store
        .claim_observed_import_task_for_worker(
            &task,
            UnixTimestamp::from_unix_seconds(1_900_100_001),
        )
        .unwrap()
        .unwrap();
    store
        .block_migration_rebuild(
            SearchRepairReason::RuntimeInvariant,
            UnixTimestamp::from_unix_seconds(1_900_100_002),
        )
        .unwrap();
    store
}

fn immutable_fixture(label: &str) -> ImmutableFixture {
    let timestamp = UnixTimestamp::from_unix_seconds(1_900_099_991);
    let source = format!("synthetic source {label}");
    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&["processing-contract-hard-cut", label]),
        source_uri: format!("synthetic://processing-contract/{label}"),
        normalized_path: format!("synthetic/processing-contract/{label}.txt"),
        file_name: format!("{label}.txt"),
        extension: FileExtension::Txt,
        byte_size: source.len() as u64,
        mtime: timestamp,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: timestamp,
        updated_at: timestamp,
        status: DocumentStatus::FieldsExtracted,
    };
    let revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(source.as_bytes()),
        source.len() as u64,
    );
    document.content_hash = Some(revision.content_hash.as_str().to_string());
    let clean_text = format!("synthetic normalized resume {label}");
    let normalized_text_hash = ContentDigest::from_bytes(clean_text.as_bytes());
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &revision.id,
            &normalized_text_hash,
            "parser-v1",
            "schema-v28",
        ),
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v28".to_string(),
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
        classified_at: UnixTimestamp::from_unix_seconds(1_900_099_992),
        review_disposition: ReviewDisposition::NotRequired,
    };
    let mention = EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[
            "processing-contract-hard-cut",
            version.id.as_str(),
            "skill",
        ]),
        resume_version_id: version.id.clone(),
        section_id: None,
        entity_type: EntityType::Skill,
        raw_value: "synthetic skill".to_string(),
        normalized_value: Some("synthetic skill".to_string()),
        span_start: None,
        span_end: None,
        confidence: 0.9,
        extractor: "synthetic-extractor".to_string(),
    };
    ImmutableFixture {
        document,
        revision,
        version,
        classification,
        mention,
    }
}

fn import_task(label: &str, root_path: &str, queued_at: UnixTimestamp) -> ImportTask {
    ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["processing-contract-hard-cut", label]),
        root_path: root_path.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at,
        started_at: None,
        finished_at: None,
        updated_at: queued_at,
    }
}

fn import_scope(task: &ImportTask, files_discovered: u64) -> ImportScanScope {
    ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind: ImportRootKind::Explicit,
        root_preset: None,
        scan_profile: ImportScanProfile::Explicit,
        requested_root_path: task.root_path.clone(),
        canonical_root_path: task.root_path.clone(),
        files_discovered,
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
        updated_at: task.queued_at,
    }
}

fn set_unpublished_visible_epoch(store: &EphemeralMetaStore, visible_epoch: i64) {
    let connection = store.connection.borrow();
    connection
        .execute(
            "INSERT INTO metadata_cow_staging_authority (
                state_key, target_visible_epoch
             ) VALUES ('default', ?1)",
            [visible_epoch],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE search_projection_state SET visible_epoch = ?1
             WHERE state_key = 'default'",
            [visible_epoch],
        )
        .unwrap();
}

fn mark_migration_rebuild_task(
    store: &EphemeralMetaStore,
    task: &ImportTask,
    contract: &ImportProcessingContract,
) {
    store
        .connection
        .borrow()
        .execute(
            "INSERT INTO migration_rebuild_full_corpus_task (
                import_task_id, processing_contract_id
             ) VALUES (?1, ?2)",
            params![task.id.as_str(), contract.id().as_str()],
        )
        .unwrap();
}

fn seed_publication_attempt(store: &EphemeralMetaStore, contract: &ImportProcessingContract) {
    let barrier_digest = ContentDigest::from_bytes(b"synthetic-hard-cut-barrier");
    let attempt_id = ContentDigest::from_bytes(b"synthetic-hard-cut-attempt");
    store
        .connection
        .borrow()
        .execute(
            "INSERT INTO migration_rebuild_publication_attempt (
                state_key, processing_contract_id, barrier_digest, attempt_id,
                attempt_count, phase, started_at_seconds, next_retry_at_seconds,
                last_error_class, updated_at_seconds
             ) VALUES (
                'default', ?1, ?2, ?3, 1, 'running', ?4, NULL, NULL, ?4
             )",
            params![
                contract.id().as_str(),
                barrier_digest.as_str(),
                attempt_id.as_str(),
                1_900_100_002_i64,
            ],
        )
        .unwrap();
}

fn force_generation_bearing_state(
    store: &EphemeralMetaStore,
    service_state: &str,
    repair_reason: Option<&str>,
) {
    let connection = store.connection.borrow();
    connection
        .execute_batch("PRAGMA foreign_keys = OFF;")
        .unwrap();
    connection
        .execute_batch(
            "DROP TRIGGER search_projection_head_change_requires_commit_guard;
             DROP TRIGGER ready_projection_head_matches_journal;",
        )
        .unwrap();
    connection
        .execute(
            "UPDATE search_projection_state
             SET service_state = ?1, generation = 'synthetic-generation',
                 repair_reason = ?2
             WHERE state_key = 'default'",
            params![service_state, repair_reason],
        )
        .unwrap();
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .unwrap();
}

fn assert_blocked(
    store: &EphemeralMetaStore,
    expected_reason: SearchRepairReason,
    expected_generation: Option<&str>,
) {
    let state = store.search_projection_state().unwrap();
    assert_eq!(
        state.service_state,
        SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(state.repair_reason, Some(expected_reason));
    assert_eq!(state.generation.as_deref(), expected_generation);
}

fn raw_projection_state(store: &EphemeralMetaStore) -> (String, Option<String>, Option<String>) {
    store
        .connection
        .borrow()
        .query_row(
            "SELECT service_state, generation, repair_reason
             FROM search_projection_state WHERE state_key = 'default'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

fn active_contract_id(store: &EphemeralMetaStore) -> Option<String> {
    store
        .connection
        .borrow()
        .query_row(
            "SELECT active_contract_id FROM migration_rebuild_contract_state
             WHERE state_key = 'default'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn row_count(store: &EphemeralMetaStore, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    store
        .connection
        .borrow()
        .query_row(&sql, [], |row| row.get(0))
        .unwrap()
}

fn processing_contract(parser: &str) -> ImportProcessingContract {
    ImportProcessingContract::new(parser, "ocr-parser-v1", "schema-v28", CLASSIFIER_EPOCH).unwrap()
}
