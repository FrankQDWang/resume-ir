use super::*;
use crate::{
    schema_v37, schema_v38, source_root_commit_fence as fence, ClassificationStatus, ContentDigest,
    CurrentClassifierEpoch, Document, DocumentId, DocumentStatus, EphemeralMetaStore,
    FileExtension, ImportProcessingContract, ImportRootKind, ImportScanProfile, ImportScanScope,
    ImportTask, ImportTaskId, ImportTaskStatus, IngestJobStatus, MetaStoreErrorClass, ReasonCode,
    ScanTrigger, SourceRevision, SourceRevisionTriage, SourceRoot, SourceRootId,
    SourceRootScanCoordination, UnixTimestamp, CLASSIFIER_EPOCH,
};

fn v32_connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE source_revision (
                 id TEXT PRIMARY KEY NOT NULL
             );
             CREATE TABLE resume_version (
                 id TEXT PRIMARY KEY NOT NULL,
                 source_revision_id TEXT NOT NULL,
                 parse_version TEXT NOT NULL
             );
             CREATE TABLE source_root (
                 id TEXT PRIMARY KEY NOT NULL,
                 state TEXT NOT NULL
             );
             CREATE TABLE source_occurrence (
                 root_id TEXT NOT NULL,
                 relative_path TEXT NOT NULL,
                 source_revision_id TEXT NOT NULL,
                 state TEXT NOT NULL,
                 PRIMARY KEY (root_id, relative_path)
             );",
        )
        .unwrap();
    connection
}

#[test]
fn pdf_reprocess_backfill_uses_a_bounded_lookup_and_leaves_no_schema_artifact() {
    let mut connection = v32_connection();
    connection
        .execute(
            "INSERT INTO source_root (id, state) VALUES ('root', 'active')",
            [],
        )
        .unwrap();
    for index in 0..256 {
        let revision = format!("revision-{index:03}");
        connection
            .execute("INSERT INTO source_revision (id) VALUES (?1)", [&revision])
            .unwrap();
        connection
            .execute(
                "INSERT INTO source_occurrence (
                     root_id, relative_path, source_revision_id, state
                 ) VALUES ('root', ?1, ?2, 'present')",
                rusqlite::params![format!("resume-{index:03}.pdf"), revision],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO resume_version (
                     id, source_revision_id, parse_version
                 ) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    format!("version-{index:03}"),
                    revision,
                    if index % 2 == 0 {
                        PDFIUM_PARSER_CONTRACT
                    } else {
                        "parser-legacy"
                    }
                ],
            )
            .unwrap();
    }

    let transaction = connection.transaction().unwrap();
    transaction.execute_batch(schema_v33::SCHEMA).unwrap();
    create_pdf_reprocess_lookup_index(&transaction).unwrap();
    let mut plan = transaction
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT 1 FROM resume_version AS version
             WHERE version.source_revision_id = ?1
               AND version.parse_version = ?2",
        )
        .unwrap();
    let details = plan
        .query_map(
            rusqlite::params!["revision-001", PDFIUM_PARSER_CONTRACT],
            |row| row.get::<_, String>(3),
        )
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        details
            .iter()
            .any(|detail| detail.contains(PDF_REPROCESS_LOOKUP_INDEX)),
        "{details:?}"
    );
    drop(plan);
    transaction.rollback().unwrap();

    let transaction = connection.transaction().unwrap();
    apply_v32_to_v33(&transaction).unwrap();
    transaction.commit().unwrap();

    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM pdf_reprocess_job", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        128
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema
                 WHERE type = 'index' AND name = ?1",
                [PDF_REPROCESS_LOOKUP_INDEX],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn v38_running_ocr_job_reopens_without_authority_then_reclaims_with_v39_fence() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_500_900);
    let root_path = "/synthetic/v38-legacy-ocr";
    let root = store
        .register_source_root(root_path, root_path, "Legacy OCR", now)
        .unwrap();
    store
        .begin_scan(&root.id, "legacy-ocr-scan", ScanTrigger::Manual, now)
        .unwrap();
    let content_hash = ContentDigest::from_bytes(b"synthetic legacy OCR source");
    let document = Document {
        id: DocumentId::from_non_secret_parts(&["v38", "legacy-ocr"]),
        source_uri: "synthetic://v38/legacy-ocr.pdf".to_string(),
        normalized_path: format!("{root_path}/legacy-ocr.pdf"),
        file_name: "legacy-ocr.pdf".to_string(),
        extension: FileExtension::Pdf,
        byte_size: 64,
        mtime: now,
        content_hash: Some(content_hash.as_str().to_string()),
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::OcrRequired,
    };
    let revision = SourceRevision::for_content(document.id.clone(), content_hash, 64);
    store.upsert_document(&document).unwrap();
    store.insert_source_revision(&revision).unwrap();
    store
        .observe_source_occurrence(
            &root.id,
            "legacy-ocr.pdf",
            &document.id,
            &revision.id,
            "legacy-ocr-scan",
            now,
        )
        .unwrap();
    store
        .insert_source_revision_triage(&SourceRevisionTriage {
            source_revision_id: revision.id.clone(),
            status: ClassificationStatus::OcrBacklog,
            triage_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::OcrRequired],
            triaged_at: now,
        })
        .unwrap();
    store
        .enqueue_ocr_job_for_source_triage(
            &revision.id,
            CurrentClassifierEpoch::parse(CLASSIFIER_EPOCH).unwrap(),
            now,
        )
        .unwrap();
    let legacy_claim = store.claim_next_ocr_job(now).unwrap().unwrap();
    store
        .connection
        .borrow()
        .execute_batch(
            "DROP TABLE ocr_claim_source_fence;
             DELETE FROM forward_migration_history WHERE to_version = 39;
             DELETE FROM schema_migrations WHERE version = 39;",
        )
        .unwrap();
    {
        let mut connection = store.connection.borrow_mut();
        let transaction = connection.transaction().unwrap();
        apply_v38_to_v39(&transaction).unwrap();
        transaction.commit().unwrap();
        validate_v39(&connection).unwrap();
    }

    assert!(!crate::source_root_ocr_claim_fence::is_current(
        &store.connection.borrow(),
        &legacy_claim,
    )
    .unwrap());
    assert_eq!(
        store
            .recover_stale_running_ingest_jobs(
                UnixTimestamp::from_unix_seconds(1_800_500_902),
                UnixTimestamp::from_unix_seconds(1_800_500_901),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .ingest_job_by_id(&legacy_claim.job.id)
            .unwrap()
            .unwrap()
            .status,
        IngestJobStatus::Interrupted
    );
    let reclaimed = store
        .claim_next_ocr_job(UnixTimestamp::from_unix_seconds(1_800_500_903))
        .unwrap()
        .unwrap();
    assert!(
        crate::source_root_ocr_claim_fence::is_current(&store.connection.borrow(), &reclaimed,)
            .unwrap()
    );
}

#[test]
fn v36_backfills_zero_attempt_evidence_for_existing_deletion_receipts() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_300_400);
    let root = store
        .register_source_root(
            "/synthetic/v36-backfill",
            "/synthetic/v36-backfill",
            "Synthetic v36 backfill",
            now,
        )
        .unwrap();
    store.begin_source_root_deletion(&root.id, now).unwrap();

    let mut connection = store.connection.borrow_mut();
    connection
        .execute_batch("DROP TABLE source_root_deletion_attempt_evidence;")
        .unwrap();
    let transaction = connection.transaction().unwrap();
    apply_v35_to_v36(&transaction).unwrap();
    transaction.commit().unwrap();

    let evidence = connection
        .query_row(
            "SELECT attempt_count, last_attempt_at_seconds, last_error_code
             FROM source_root_deletion_attempt_evidence
             WHERE root_id = ?1",
            [root.id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(evidence, (0, None, None));
}

#[test]
fn v37_distinguishes_legacy_and_current_deletion_checkpoint_snapshots() {
    #[derive(Debug, Eq, PartialEq)]
    struct LegacyReceiptWitness {
        canonical_path: String,
        phase: String,
        affected_documents: i64,
        removed_documents: i64,
        started_at_seconds: i64,
        updated_at_seconds: i64,
        completed_at_seconds: Option<i64>,
        document_id: String,
        content_hash: String,
        attempt_count: i64,
        last_attempt_at_seconds: Option<i64>,
        last_error_phase: Option<String>,
        last_error_code: Option<String>,
        last_error_at_seconds: Option<i64>,
    }

    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.initialize_empty_schema(schema_v36::VERSION).unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_400_500);
    let legacy_root = store
        .register_source_root(
            "/synthetic/v37-legacy",
            "/synthetic/v37-legacy",
            "Synthetic v37 legacy",
            now,
        )
        .unwrap();
    let legacy_document_id = format!("doc_{}", "1".repeat(32));
    let legacy_content_hash = format!("sha256:{}", "2".repeat(64));
    let before = {
        let connection = store.connection.borrow();
        connection
            .execute(
                "INSERT INTO source_root_deletion (
                    root_id, canonical_path, phase,
                    affected_documents, removed_documents,
                    started_at_seconds, updated_at_seconds
                 ) VALUES (?1, ?2, 'quiescing', 1, 0, ?3, ?4)",
                rusqlite::params![
                    legacy_root.id.as_str(),
                    legacy_root.canonical_path,
                    now.as_unix_seconds(),
                    now.as_unix_seconds() + 1,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO source_root_deletion_document (
                    root_id, document_id, content_hash
                 ) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    legacy_root.id.as_str(),
                    legacy_document_id,
                    legacy_content_hash,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO source_root_deletion_attempt_evidence (
                    root_id, attempt_count, last_attempt_at_seconds,
                    last_error_phase, last_error_code, last_error_at_seconds
                 ) VALUES (?1, 3, ?2, 'quiescing',
                    'ocr_quiescence_timeout', ?3)",
                rusqlite::params![
                    legacy_root.id.as_str(),
                    now.as_unix_seconds() + 2,
                    now.as_unix_seconds() + 3,
                ],
            )
            .unwrap();
        connection
            .query_row(
                "SELECT deletion.canonical_path, deletion.phase,
                        deletion.affected_documents, deletion.removed_documents,
                        deletion.started_at_seconds, deletion.updated_at_seconds,
                        deletion.completed_at_seconds,
                        snapshot.document_id, snapshot.content_hash,
                        evidence.attempt_count, evidence.last_attempt_at_seconds,
                        evidence.last_error_phase, evidence.last_error_code,
                        evidence.last_error_at_seconds
                 FROM source_root_deletion AS deletion
                 JOIN source_root_deletion_document AS snapshot
                   ON snapshot.root_id = deletion.root_id
                 JOIN source_root_deletion_attempt_evidence AS evidence
                   ON evidence.root_id = deletion.root_id
                 WHERE deletion.root_id = ?1",
                [legacy_root.id.as_str()],
                |row| {
                    Ok(LegacyReceiptWitness {
                        canonical_path: row.get(0)?,
                        phase: row.get(1)?,
                        affected_documents: row.get(2)?,
                        removed_documents: row.get(3)?,
                        started_at_seconds: row.get(4)?,
                        updated_at_seconds: row.get(5)?,
                        completed_at_seconds: row.get(6)?,
                        document_id: row.get(7)?,
                        content_hash: row.get(8)?,
                        attempt_count: row.get(9)?,
                        last_attempt_at_seconds: row.get(10)?,
                        last_error_phase: row.get(11)?,
                        last_error_code: row.get(12)?,
                        last_error_at_seconds: row.get(13)?,
                    })
                },
            )
            .unwrap()
    };

    let mut connection = store.connection.borrow_mut();
    let transaction = connection.transaction().unwrap();
    apply_v36_to_v37(&transaction).unwrap();
    transaction.commit().unwrap();
    let legacy_version = connection
        .query_row(
            "SELECT checkpoint_protocol_version
             FROM source_root_deletion WHERE root_id = ?1",
            [legacy_root.id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let after = connection
        .query_row(
            "SELECT deletion.canonical_path, deletion.phase,
                    deletion.affected_documents, deletion.removed_documents,
                    deletion.started_at_seconds, deletion.updated_at_seconds,
                    deletion.completed_at_seconds,
                    snapshot.document_id, snapshot.content_hash,
                    evidence.attempt_count, evidence.last_attempt_at_seconds,
                    evidence.last_error_phase, evidence.last_error_code,
                    evidence.last_error_at_seconds
             FROM source_root_deletion AS deletion
             JOIN source_root_deletion_document AS snapshot
               ON snapshot.root_id = deletion.root_id
             JOIN source_root_deletion_attempt_evidence AS evidence
               ON evidence.root_id = deletion.root_id
             WHERE deletion.root_id = ?1",
            [legacy_root.id.as_str()],
            |row| {
                Ok(LegacyReceiptWitness {
                    canonical_path: row.get(0)?,
                    phase: row.get(1)?,
                    affected_documents: row.get(2)?,
                    removed_documents: row.get(3)?,
                    started_at_seconds: row.get(4)?,
                    updated_at_seconds: row.get(5)?,
                    completed_at_seconds: row.get(6)?,
                    document_id: row.get(7)?,
                    content_hash: row.get(8)?,
                    attempt_count: row.get(9)?,
                    last_attempt_at_seconds: row.get(10)?,
                    last_error_phase: row.get(11)?,
                    last_error_code: row.get(12)?,
                    last_error_at_seconds: row.get(13)?,
                })
            },
        )
        .unwrap();
    assert_eq!(legacy_version, schema_v37::LEGACY_OR_UNATTESTED);
    assert_eq!(after, before);
    let transaction = connection.transaction().unwrap();
    apply_v37_to_v38(&transaction).unwrap();
    transaction.commit().unwrap();
    drop(connection);

    let current_root = store
        .register_source_root(
            "/synthetic/v37-current",
            "/synthetic/v37-current",
            "Synthetic v37 current",
            now,
        )
        .unwrap();
    store
        .begin_source_root_deletion(&current_root.id, now)
        .unwrap();
    let current_version = || {
        store
            .connection
            .borrow()
            .query_row(
                "SELECT checkpoint_protocol_version
                 FROM source_root_deletion WHERE root_id = ?1",
                [current_root.id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert_eq!(current_version(), schema_v37::SNAPSHOT_INVARIANT_V2);

    let plan = store
        .connection
        .borrow()
        .prepare(
            "EXPLAIN QUERY PLAN
             SELECT 1 FROM source_root_deletion_attempt_evidence AS evidence
             WHERE evidence.root_id = ?1
               AND EXISTS (
                 SELECT 1 FROM source_root_deletion AS deletion
                 WHERE deletion.root_id = ?1
                   AND deletion.phase NOT IN ('complete', 'failed')
               )",
        )
        .unwrap()
        .query_map([current_root.id.as_str()], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter().any(|detail| {
            detail.contains("source_root_deletion_attempt_evidence") && detail.contains("root_id=?")
        }),
        "{plan:?}"
    );
    assert!(
        plan.iter().any(|detail| {
            detail.contains("source_root_deletion") && detail.contains("root_id=?")
        }),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .all(|detail| !detail.contains("source_root_deletion_document")),
        "{plan:?}"
    );

    for offset in [4, 5] {
        let before_changes = store
            .connection
            .borrow()
            .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
            .unwrap();
        store
            .begin_source_root_deletion_attempt(
                &current_root.id,
                UnixTimestamp::from_unix_seconds(now.as_unix_seconds() + offset),
            )
            .unwrap();
        let after_changes = store
            .connection
            .borrow()
            .query_row("SELECT total_changes()", [], |row| row.get::<_, i64>(0))
            .unwrap();
        assert_eq!(after_changes - before_changes, 1);
        assert_eq!(current_version(), schema_v37::SNAPSHOT_INVARIANT_V2);
    }
}

#[test]
fn v38_migration_conservatively_revokes_failed_receipt_history() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.initialize_empty_schema(schema_v37::VERSION).unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_500_700);
    let failed_root = synthetic_root(&store, "v38-failed-history", now);
    let clean_root = synthetic_root(&store, "v38-no-history", now);
    insert_failed_deletion_receipt(&store, &failed_root, now);
    let connection = store.connection.borrow();
    connection
        .execute(
            "INSERT INTO scan_snapshot (
                id, root_id, trigger, phase, completeness,
                started_at_seconds, updated_at_seconds, completed_at_seconds
             ) VALUES ('legacy-failed-scan', ?1, 'manual', 'failed', 'partial', ?2, ?2, ?2)",
            rusqlite::params![failed_root.id.as_str(), now.as_unix_seconds()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO source_root_deletion_attempt_evidence (
                root_id, attempt_count, last_attempt_at_seconds,
                last_error_phase, last_error_code, last_error_at_seconds
             ) VALUES (?1, 1, ?2, 'quiescing', 'internal', ?2)",
            rusqlite::params![failed_root.id.as_str(), now.as_unix_seconds()],
        )
        .unwrap();
    drop(connection);
    migrate_to_v38(&store).unwrap();
    assert_eq!(root_epoch(&store, &failed_root.id), 1);
    assert_eq!(root_epoch(&store, &clean_root.id), 0);
    assert_eq!(
        query_i64(
            &store,
            "SELECT root_revocation_epoch FROM scan_snapshot WHERE id = 'legacy-failed-scan'",
            [],
        ),
        0
    );
}

#[test]
fn v38_migration_rejects_unknown_receipt_phase_without_schema_side_effects() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.initialize_empty_schema(schema_v37::VERSION).unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_500_750);
    let root = synthetic_root(&store, "v38-unknown-migration-phase", now);
    insert_failed_deletion_receipt(&store, &root, now);
    force_receipt_phase(&store, &root.id, "future");
    assert!(migrate_to_v38(&store).is_err());
    assert_eq!(
        query_i64(
            &store,
            "SELECT COUNT(*) FROM pragma_table_info('source_root') WHERE name = 'revocation_epoch'",
            [],
        ),
        0
    );
}

#[test]
fn v38_scan_commit_and_deletion_epoch_fail_closed_atomically() {
    let fixture = V38Fixture::new();
    let store = &fixture.store;
    let now = fixture.now;
    let root = fixture.root("runtime");
    let first = fixture.start(&root, "first");
    fixture.assert_commit_valid(&root, &first.0.id);

    let standalone = fixture.root("standalone");
    let standalone_id = ImportTaskId::from_non_secret_parts(&["v38-root-fence", "standalone"]);
    store
        .begin_scan(
            &standalone.id,
            standalone_id.as_str(),
            ScanTrigger::Manual,
            now,
        )
        .unwrap();
    fixture.assert_commit_rejected(&standalone, &standalone_id);
    fixture.assert_commit_rejected(&standalone, &first.0.id);

    let partial = fixture.root("partial-binding");
    let partial_scan = fixture.start(&partial, "partial-binding");
    fixture.execute(
        "DELETE FROM import_scan_scope WHERE import_task_id = ?1",
        [partial_scan.0.id.as_str()],
    );
    fixture.assert_commit_rejected(&partial, &partial_scan.0.id);
    store.upsert_import_scan_scope(&partial_scan.1).unwrap();
    fixture.execute(
        "DELETE FROM scan_snapshot WHERE id = ?1",
        [partial_scan.0.id.as_str()],
    );
    fixture.assert_commit_rejected(&partial, &partial_scan.0.id);

    store.begin_source_root_deletion(&root.id, now).unwrap();
    assert_eq!(root_epoch(store, &root.id), 1);
    fixture.assert_commit_rejected(&root, &first.0.id);

    let retry = queued_scan(
        "v38-coalesced",
        &root.canonical_path,
        UnixTimestamp::from_unix_seconds(now.as_unix_seconds() + 1),
    );
    let before = total_changes(store);
    assert!(matches!(
        fixture
            .coordinate(&root, ScanTrigger::Watcher, &retry, retry.0.queued_at)
            .unwrap(),
        SourceRootScanCoordination::Coalesced(_)
    ));
    assert_eq!(total_changes(store) - before, 0);
    store
        .begin_source_root_deletion_attempt(&root.id, now)
        .unwrap();
    fixture.execute(
        "UPDATE source_root_deletion SET phase = 'failed' WHERE root_id = ?1",
        [root.id.as_str()],
    );
    fixture.assert_commit_rejected(&root, &first.0.id);

    let malformed = fixture.root("malformed");
    for value in [
        rusqlite::types::Value::Real(1.5),
        rusqlite::types::Value::Text("not-an-integer".to_string()),
    ] {
        execute_ignoring_checks(
            store,
            "UPDATE source_root SET revocation_epoch = ?2 WHERE id = ?1",
            rusqlite::params![malformed.id.as_str(), value],
        );
        let before = total_changes(store);
        assert!(fence::admit_scan(&store.connection.borrow(), &malformed.id).is_err());
        assert_eq!(total_changes(store) - before, 0);
    }

    let saturated = fixture.root("saturated");
    fixture.execute(
        "UPDATE source_root SET revocation_epoch = ?2 WHERE id = ?1",
        rusqlite::params![saturated.id.as_str(), schema_v38::MAX_ROOT_REVOCATION_EPOCH],
    );
    let before = total_changes(store);
    assert!(store
        .begin_source_root_deletion(&saturated.id, now)
        .is_err());
    assert_eq!(total_changes(store) - before, 0);
    assert_eq!(
        root_epoch(store, &saturated.id),
        schema_v38::MAX_ROOT_REVOCATION_EPOCH
    );
    assert!(store.source_root_deletion(&saturated.id).unwrap().is_none());

    let rollback = fixture.root("rollback");
    fixture.execute_batch(
        "CREATE TEMP TRIGGER fail_v38_receipt
         BEFORE INSERT ON source_root_deletion
         BEGIN SELECT RAISE(ABORT, 'synthetic receipt failure'); END;",
    );
    assert!(store.begin_source_root_deletion(&rollback.id, now).is_err());
    fixture.execute_batch("DROP TRIGGER temp.fail_v38_receipt;");
    assert_eq!(root_epoch(store, &rollback.id), 0);
    assert!(store.source_root_deletion(&rollback.id).unwrap().is_none());

    let late = fixture.root("late-failure");
    fixture.execute_batch(
        "CREATE TEMP TRIGGER drift_v38_scan_epoch
         AFTER INSERT ON scan_snapshot
         BEGIN
           UPDATE source_root
           SET revocation_epoch = revocation_epoch + 1
           WHERE id = NEW.root_id;
         END;",
    );
    let late_scan = queued_scan("v38-late", &late.canonical_path, now);
    let before = total_changes(store);
    assert!(fixture
        .coordinate(&late, ScanTrigger::Manual, &late_scan, now)
        .is_err());
    assert!(total_changes(store) - before > 0);
    fixture.execute_batch("DROP TRIGGER temp.drift_v38_scan_epoch;");
    assert_eq!(root_epoch(store, &late.id), 0);
    assert!(store.import_task_by_id(&late_scan.0.id).unwrap().is_none());
    assert!(store
        .import_scan_scope_by_task_id(&late_scan.0.id)
        .unwrap()
        .is_none());
    assert!(store.latest_scan_snapshot(&late.id).unwrap().is_none());

    force_receipt_phase(store, &root.id, "future");
    let before = total_changes(store);
    let witness = deletion_witness(store, &root.id);
    let error = store.begin_source_root_deletion(&root.id, now).unwrap_err();
    assert_eq!(error.class(), MetaStoreErrorClass::InvalidValue);
    assert_eq!(total_changes(store) - before, 0);
    assert_eq!(deletion_witness(store, &root.id), witness);
    let before = total_changes(store);
    let error = store
        .register_source_root(
            &root.canonical_path,
            &root.requested_path,
            &root.display_label,
            now,
        )
        .unwrap_err();
    assert_eq!(error.class(), MetaStoreErrorClass::InvalidValue);
    assert_eq!(total_changes(store) - before, 0);
}

struct V38Fixture {
    store: EphemeralMetaStore,
    now: UnixTimestamp,
    contract: ImportProcessingContract,
}

impl V38Fixture {
    fn new() -> Self {
        let store = EphemeralMetaStore::open_in_memory().unwrap();
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_800_500_800);
        let contract =
            ImportProcessingContract::new("v38-parser", "v38-ocr", "v38-derived", CLASSIFIER_EPOCH)
                .unwrap();
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap();
        Self {
            store,
            now,
            contract,
        }
    }

    fn root(&self, label: &str) -> SourceRoot {
        synthetic_root(&self.store, &format!("v38-{label}"), self.now)
    }

    fn start(&self, root: &SourceRoot, label: &str) -> (ImportTask, ImportScanScope) {
        let scan = queued_scan(&format!("v38-{label}"), &root.canonical_path, self.now);
        assert!(matches!(
            self.coordinate(root, ScanTrigger::Manual, &scan, self.now)
                .unwrap(),
            SourceRootScanCoordination::Started { .. }
        ));
        scan
    }

    fn coordinate(
        &self,
        root: &SourceRoot,
        trigger: ScanTrigger,
        scan: &(ImportTask, ImportScanScope),
        now: UnixTimestamp,
    ) -> crate::Result<SourceRootScanCoordination> {
        self.store.coordinate_source_root_scan(
            &root.id,
            trigger,
            &scan.0,
            &scan.1,
            &self.contract,
            now,
        )
    }

    fn execute<P: rusqlite::Params>(&self, sql: &str, params: P) {
        self.store.connection.borrow().execute(sql, params).unwrap();
    }

    fn execute_batch(&self, sql: &str) {
        self.store.connection.borrow().execute_batch(sql).unwrap();
    }

    fn assert_commit_valid(&self, root: &SourceRoot, task_id: &ImportTaskId) {
        fence::validate_scan_commit(&self.store.connection.borrow(), &root.id, task_id).unwrap();
    }

    fn assert_commit_rejected(&self, root: &SourceRoot, task_id: &ImportTaskId) {
        assert!(
            fence::validate_scan_commit(&self.store.connection.borrow(), &root.id, task_id)
                .is_err()
        );
    }
}

fn queued_scan(
    label: &str,
    canonical_root_path: &str,
    now: UnixTimestamp,
) -> (ImportTask, ImportScanScope) {
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["v38-root-fence", label]),
        root_path: canonical_root_path.to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let scope = ImportScanScope {
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
        updated_at: now,
    };
    (task, scope)
}

fn synthetic_root(store: &EphemeralMetaStore, label: &str, now: UnixTimestamp) -> SourceRoot {
    let path = format!("/synthetic/{label}");
    store
        .register_source_root(&path, &path, label, now)
        .unwrap()
}

fn migrate_to_v38(store: &EphemeralMetaStore) -> crate::Result<()> {
    let mut connection = store.connection.borrow_mut();
    let transaction = connection.transaction().unwrap();
    apply_v37_to_v38(&transaction)?;
    transaction.commit().unwrap();
    Ok(())
}

fn insert_failed_deletion_receipt(
    store: &EphemeralMetaStore,
    root: &SourceRoot,
    now: UnixTimestamp,
) {
    store
        .connection
        .borrow()
        .execute(
            "INSERT INTO source_root_deletion (
                root_id, canonical_path, phase, affected_documents, removed_documents,
                started_at_seconds, updated_at_seconds, checkpoint_protocol_version
             ) VALUES (?1, ?2, 'failed', 0, 0, ?3, ?3, ?4)",
            rusqlite::params![
                root.id.as_str(),
                root.canonical_path,
                now.as_unix_seconds(),
                schema_v37::SNAPSHOT_INVARIANT_V2,
            ],
        )
        .unwrap();
}

fn force_receipt_phase(store: &EphemeralMetaStore, root_id: &SourceRootId, phase: &str) {
    execute_ignoring_checks(
        store,
        "UPDATE source_root_deletion SET phase = ?2 WHERE root_id = ?1",
        rusqlite::params![root_id.as_str(), phase],
    );
}

fn execute_ignoring_checks<P: rusqlite::Params>(store: &EphemeralMetaStore, sql: &str, params: P) {
    let connection = store.connection.borrow();
    connection
        .execute_batch("PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    connection.execute(sql, params).unwrap();
    connection
        .execute_batch("PRAGMA ignore_check_constraints = OFF;")
        .unwrap();
}

fn query_i64<P: rusqlite::Params>(store: &EphemeralMetaStore, sql: &str, params: P) -> i64 {
    store
        .connection
        .borrow()
        .query_row(sql, params, |row| row.get(0))
        .unwrap()
}

fn root_epoch(store: &EphemeralMetaStore, root_id: &SourceRootId) -> i64 {
    query_i64(
        store,
        "SELECT revocation_epoch FROM source_root WHERE id = ?1",
        [root_id.as_str()],
    )
}

fn total_changes(store: &EphemeralMetaStore) -> i64 {
    query_i64(store, "SELECT total_changes()", [])
}

fn deletion_witness(
    store: &EphemeralMetaStore,
    root_id: &SourceRootId,
) -> Vec<rusqlite::types::Value> {
    store
        .connection
        .borrow()
        .query_row(
            "SELECT root.revocation_epoch, deletion.*, evidence.*,
                    (SELECT COUNT(*) FROM source_root_deletion_document WHERE root_id = root.id)
             FROM source_root AS root
             JOIN source_root_deletion AS deletion ON deletion.root_id = root.id
             JOIN source_root_deletion_attempt_evidence AS evidence
               ON evidence.root_id = root.id
            WHERE root.id = ?1",
            [root_id.as_str()],
            |row| {
                (0..row.as_ref().column_count())
                    .map(|column| row.get(column))
                    .collect()
            },
        )
        .unwrap()
}
