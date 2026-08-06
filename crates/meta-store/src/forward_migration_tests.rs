use super::*;
use crate::{
    schema_v37, schema_v38, EphemeralMetaStore, ImportProcessingContract, ImportRootKind,
    ImportScanProfile, ImportScanScope, ImportTask, ImportTaskId, ImportTaskStatus, ScanTrigger,
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
fn v38_persists_root_and_scan_revocation_epochs() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();

    assert_eq!(store.schema_version().unwrap(), 38);
    let connection = store.connection.borrow();
    for (table, column) in [
        ("source_root", "revocation_epoch"),
        ("scan_snapshot", "root_revocation_epoch"),
    ] {
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
                    rusqlite::params![table, column],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "missing {table}.{column}"
        );
    }
}

#[test]
fn v38_migration_conservatively_revokes_failed_receipt_history() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.initialize_empty_schema(schema_v37::VERSION).unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_500_700);
    let failed_root = synthetic_root(&store, "v38-failed-history", now);
    let clean_root = synthetic_root(&store, "v38-no-history", now);
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
            "INSERT INTO source_root_deletion (
                root_id, canonical_path, phase, affected_documents, removed_documents,
                started_at_seconds, updated_at_seconds, checkpoint_protocol_version
             ) VALUES (?1, ?2, 'failed', 0, 0, ?3, ?3, ?4)",
            rusqlite::params![
                failed_root.id.as_str(),
                failed_root.canonical_path,
                now.as_unix_seconds(),
                schema_v37::SNAPSHOT_INVARIANT_V2,
            ],
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

    let mut connection = store.connection.borrow_mut();
    let transaction = connection.transaction().unwrap();
    apply_v37_to_v38(&transaction).unwrap();
    transaction.commit().unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT revocation_epoch FROM source_root WHERE id = ?1",
                [failed_root.id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT revocation_epoch FROM source_root WHERE id = ?1",
                [clean_root.id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT root_revocation_epoch FROM scan_snapshot WHERE id = 'legacy-failed-scan'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert!(
        crate::source_root_commit_fence::validate_scan_commit(&connection, &failed_root.id, 0,)
            .is_err()
    );
}

#[test]
fn v38_scan_commit_and_deletion_epoch_fail_closed_atomically() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_500_800);
    let contract =
        ImportProcessingContract::new("v38-parser", "v38-ocr", "v38-derived", CLASSIFIER_EPOCH)
            .unwrap();
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    let root = synthetic_root(&store, "v38-runtime", now);
    let first = queued_scan("v38-first", &root.canonical_path, now);
    assert!(matches!(
        store
            .coordinate_source_root_scan(
                &root.id,
                ScanTrigger::Manual,
                &first.0,
                &first.1,
                &contract,
                now,
            )
            .unwrap(),
        SourceRootScanCoordination::Started { .. }
    ));
    assert_eq!(scan_epoch(&store, first.0.id.as_str()), 0);
    store.begin_source_root_deletion(&root.id, now).unwrap();
    assert_eq!(root_epoch(&store, &root.id), 1);
    assert!(crate::source_root_commit_fence::validate_scan_commit(
        &store.connection.borrow(),
        &root.id,
        0,
    )
    .is_err());
    assert!(crate::source_root_commit_fence::validate_scan_commit(
        &store.connection.borrow(),
        &root.id,
        1,
    )
    .is_err());

    let retry = queued_scan(
        "v38-coalesced",
        &root.canonical_path,
        UnixTimestamp::from_unix_seconds(now.as_unix_seconds() + 1),
    );
    let before = total_changes(&store);
    assert!(matches!(
        store
            .coordinate_source_root_scan(
                &root.id,
                ScanTrigger::Watcher,
                &retry.0,
                &retry.1,
                &contract,
                retry.0.queued_at,
            )
            .unwrap(),
        SourceRootScanCoordination::Coalesced(_)
    ));
    assert_eq!(total_changes(&store) - before, 0);

    let malformed = synthetic_root(&store, "v38-malformed", now);
    store
        .connection
        .borrow()
        .execute_batch("PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE source_root SET revocation_epoch = 1.5 WHERE id = ?1",
            [malformed.id.as_str()],
        )
        .unwrap();
    store
        .connection
        .borrow()
        .execute_batch("PRAGMA ignore_check_constraints = OFF;")
        .unwrap();
    let before = total_changes(&store);
    assert!(
        crate::source_root_commit_fence::admit_scan(&store.connection.borrow(), &malformed.id,)
            .is_err()
    );
    assert_eq!(total_changes(&store) - before, 0);

    let saturated = synthetic_root(&store, "v38-saturated", now);
    store
        .connection
        .borrow()
        .execute(
            "UPDATE source_root SET revocation_epoch = ?2 WHERE id = ?1",
            rusqlite::params![saturated.id.as_str(), schema_v38::MAX_ROOT_REVOCATION_EPOCH],
        )
        .unwrap();
    let before = total_changes(&store);
    assert!(store
        .begin_source_root_deletion(&saturated.id, now)
        .is_err());
    assert_eq!(total_changes(&store) - before, 0);
    assert_eq!(
        root_epoch(&store, &saturated.id),
        schema_v38::MAX_ROOT_REVOCATION_EPOCH
    );
    assert!(store.source_root_deletion(&saturated.id).unwrap().is_none());

    let rollback = synthetic_root(&store, "v38-rollback", now);
    store
        .connection
        .borrow()
        .execute_batch(
            "CREATE TEMP TRIGGER fail_v38_receipt
             BEFORE INSERT ON source_root_deletion
             BEGIN SELECT RAISE(ABORT, 'synthetic receipt failure'); END;",
        )
        .unwrap();
    assert!(store.begin_source_root_deletion(&rollback.id, now).is_err());
    store
        .connection
        .borrow()
        .execute_batch("DROP TRIGGER temp.fail_v38_receipt;")
        .unwrap();
    assert_eq!(root_epoch(&store, &rollback.id), 0);
    assert!(store.source_root_deletion(&rollback.id).unwrap().is_none());

    let late = synthetic_root(&store, "v38-late-failure", now);
    store
        .connection
        .borrow()
        .execute_batch(
            "CREATE TEMP TRIGGER drift_v38_scan_epoch
             AFTER INSERT ON scan_snapshot
             BEGIN
               UPDATE source_root
               SET revocation_epoch = revocation_epoch + 1
               WHERE id = NEW.root_id;
             END;",
        )
        .unwrap();
    let late_scan = queued_scan("v38-late", &late.canonical_path, now);
    let before = total_changes(&store);
    assert!(store
        .coordinate_source_root_scan(
            &late.id,
            ScanTrigger::Manual,
            &late_scan.0,
            &late_scan.1,
            &contract,
            now,
        )
        .is_err());
    assert!(total_changes(&store) - before > 0);
    store
        .connection
        .borrow()
        .execute_batch("DROP TRIGGER temp.drift_v38_scan_epoch;")
        .unwrap();
    assert_eq!(root_epoch(&store, &late.id), 0);
    let persisted = store
        .connection
        .borrow()
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM import_task WHERE root_path = ?1),
                (SELECT COUNT(*) FROM import_scan_scope WHERE canonical_root_path = ?1),
                (SELECT COUNT(*) FROM scan_snapshot WHERE root_id = ?2)",
            rusqlite::params![late.canonical_path, late.id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(persisted, (0, 0, 0));

    store
        .connection
        .borrow()
        .execute_batch("PRAGMA ignore_check_constraints = ON;")
        .unwrap();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE source_root_deletion SET phase = 'future' WHERE root_id = ?1",
            [root.id.as_str()],
        )
        .unwrap();
    store
        .connection
        .borrow()
        .execute_batch("PRAGMA ignore_check_constraints = OFF;")
        .unwrap();
    let before = total_changes(&store);
    assert!(
        crate::source_root_commit_fence::admit_scan(&store.connection.borrow(), &root.id,).is_err()
    );
    assert_eq!(total_changes(&store) - before, 0);
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

fn synthetic_root(
    store: &EphemeralMetaStore,
    label: &str,
    now: UnixTimestamp,
) -> crate::SourceRoot {
    let path = format!("/synthetic/{label}");
    store
        .register_source_root(&path, &path, label, now)
        .unwrap()
}

fn root_epoch(store: &EphemeralMetaStore, root_id: &crate::SourceRootId) -> i64 {
    store
        .connection
        .borrow()
        .query_row(
            "SELECT revocation_epoch FROM source_root WHERE id = ?1",
            [root_id.as_str()],
            |row| row.get(0),
        )
        .unwrap()
}

fn scan_epoch(store: &EphemeralMetaStore, scan_id: &str) -> i64 {
    store
        .connection
        .borrow()
        .query_row(
            "SELECT root_revocation_epoch FROM scan_snapshot WHERE id = ?1",
            [scan_id],
            |row| row.get(0),
        )
        .unwrap()
}

fn total_changes(store: &EphemeralMetaStore) -> i64 {
    store
        .connection
        .borrow()
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap()
}
