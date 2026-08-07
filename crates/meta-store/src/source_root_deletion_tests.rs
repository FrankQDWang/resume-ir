use rusqlite::params;

use super::{
    checkpoint, read_source_root_deletion_completion_residuals,
    SourceRootDeletionCompletionResiduals,
};
use crate::{
    schema_v37, ContentDigest, Document, DocumentId, DocumentStatus, EphemeralMetaStore,
    FileExtension, MetaStoreErrorClass, ScanTrigger, SourceRevision, SourceRoot,
    SourceRootDeletion, SourceRootDeletionPhase, SourceRootId, UnixTimestamp,
};

fn synthetic_document(label: &str, now: UnixTimestamp) -> (Document, SourceRevision) {
    let source = format!("synthetic source-root deletion {label}");
    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&["source-root-deletion", label]),
        source_uri: format!("synthetic://source-root-deletion/{label}.pdf"),
        normalized_path: format!("synthetic/source-root-deletion/{label}.pdf"),
        file_name: format!("{label}.pdf"),
        extension: FileExtension::Pdf,
        byte_size: source.len() as u64,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::FieldsExtracted,
    };
    let revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(source.as_bytes()),
        source.len() as u64,
    );
    document.content_hash = Some(revision.content_hash.as_str().to_string());
    (document, revision)
}

fn deletion_snapshot_tuples(
    store: &EphemeralMetaStore,
    root_id: &SourceRootId,
) -> Vec<(String, String)> {
    store
        .connection
        .borrow()
        .prepare(
            "SELECT document_id, content_hash
             FROM source_root_deletion_document
             WHERE root_id = ?1
             ORDER BY document_id, content_hash",
        )
        .unwrap()
        .query_map([root_id.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap()
}

fn total_changes(store: &EphemeralMetaStore) -> i64 {
    store
        .connection
        .borrow()
        .query_row("SELECT total_changes()", [], |row| row.get(0))
        .unwrap()
}

fn register_scanned_root(
    store: &EphemeralMetaStore,
    path: &str,
    label: &str,
    scan_id: &str,
    now: UnixTimestamp,
) -> SourceRoot {
    let root = store.register_source_root(path, path, label, now).unwrap();
    store
        .begin_scan(&root.id, scan_id, ScanTrigger::Manual, now)
        .unwrap();
    root
}

fn persist_synthetic_document(
    store: &EphemeralMetaStore,
    label: &str,
    now: UnixTimestamp,
) -> (Document, SourceRevision) {
    let (document, revision) = synthetic_document(label, now);
    store.upsert_document(&document).unwrap();
    store.insert_source_revision(&revision).unwrap();
    (document, revision)
}

fn observe(
    store: &EphemeralMetaStore,
    root_id: &SourceRootId,
    document: &Document,
    revision: &SourceRevision,
    scan_id: &str,
    now: UnixTimestamp,
) {
    store
        .observe_source_occurrence(
            root_id,
            &document.file_name,
            &document.id,
            &revision.id,
            scan_id,
            now,
        )
        .unwrap();
}

fn make_quiescing_receipt_legacy(
    store: &EphemeralMetaStore,
    root_id: &SourceRootId,
    now: UnixTimestamp,
    stale_canonical_path: &str,
) {
    store.begin_source_root_deletion(root_id, now).unwrap();
    store
        .set_source_root_deletion_phase(root_id, SourceRootDeletionPhase::Quiescing, now)
        .unwrap();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE source_root_deletion
             SET checkpoint_protocol_version = ?2, canonical_path = ?3
             WHERE root_id = ?1 AND phase = 'quiescing'",
            params![
                root_id.as_str(),
                schema_v37::LEGACY_OR_UNATTESTED,
                stale_canonical_path
            ],
        )
        .unwrap();
}

#[derive(Debug, Eq, PartialEq)]
struct DeletionTransactionWitness {
    receipt: SourceRootDeletion,
    canonical_path: String,
    checkpoint_protocol_version: i64,
    snapshot: Vec<(String, String)>,
}

fn deletion_transaction_witness(
    store: &EphemeralMetaStore,
    root_id: &SourceRootId,
) -> DeletionTransactionWitness {
    let receipt = store.source_root_deletion(root_id).unwrap().unwrap();
    let connection = store.connection.borrow();
    let (canonical_path, checkpoint_protocol_version) = connection
        .query_row(
            "SELECT canonical_path, checkpoint_protocol_version
             FROM source_root_deletion WHERE root_id = ?1",
            [root_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    drop(connection);
    DeletionTransactionWitness {
        receipt,
        canonical_path,
        checkpoint_protocol_version,
        snapshot: deletion_snapshot_tuples(store, root_id),
    }
}

#[test]
fn requested_to_quiescing_generic_transition_reconciles_snapshot_atomically() {
    const ROOT_A: &str = "/synthetic/requested-checkpoint-a";
    const ROOT_B: &str = "/synthetic/requested-checkpoint-b";
    const SCAN_A: &str = "requested-checkpoint-scan-a";
    const SCAN_B: &str = "requested-checkpoint-scan-b";
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let requested_at = UnixTimestamp::from_unix_seconds(1_800_299_000);
    let attempted_at = UnixTimestamp::from_unix_seconds(1_800_299_001);
    let quiescing_at = UnixTimestamp::from_unix_seconds(1_800_299_002);
    let root_a = store
        .register_source_root(ROOT_A, ROOT_A, "Requested checkpoint A", requested_at)
        .unwrap();
    let root_b = store
        .register_source_root(ROOT_B, ROOT_B, "Requested checkpoint B", requested_at)
        .unwrap();
    store
        .begin_scan(&root_a.id, SCAN_A, ScanTrigger::Manual, requested_at)
        .unwrap();
    store
        .begin_scan(&root_b.id, SCAN_B, ScanTrigger::Manual, requested_at)
        .unwrap();

    let (shared, shared_revision) = synthetic_document("shared", requested_at);
    store.upsert_document(&shared).unwrap();
    store.insert_source_revision(&shared_revision).unwrap();
    store
        .observe_source_occurrence(
            &root_a.id,
            "shared.pdf",
            &shared.id,
            &shared_revision.id,
            SCAN_A,
            requested_at,
        )
        .unwrap();
    let requested = store
        .begin_source_root_deletion(&root_a.id, requested_at)
        .unwrap();
    assert_eq!(
        store.source_root_deletion_document_ids(&root_a.id).unwrap(),
        vec![shared.id.clone()]
    );
    let attempted = store
        .begin_source_root_deletion_attempt(&root_a.id, attempted_at)
        .unwrap();

    store
        .observe_source_occurrence(
            &root_b.id,
            "shared.pdf",
            &shared.id,
            &shared_revision.id,
            SCAN_B,
            attempted_at,
        )
        .unwrap();
    let (exclusive, exclusive_revision) = synthetic_document("exclusive", attempted_at);
    store.upsert_document(&exclusive).unwrap();
    store.insert_source_revision(&exclusive_revision).unwrap();
    store
        .observe_source_occurrence(
            &root_a.id,
            "exclusive.pdf",
            &exclusive.id,
            &exclusive_revision.id,
            SCAN_A,
            attempted_at,
        )
        .unwrap();

    store
        .set_source_root_deletion_phase(
            &root_a.id,
            SourceRootDeletionPhase::Quiescing,
            quiescing_at,
        )
        .unwrap();

    let quiescing = store.source_root_deletion(&root_a.id).unwrap().unwrap();
    assert_eq!(quiescing.root_id, requested.root_id);
    assert_eq!(quiescing.started_at, requested.started_at);
    assert_eq!(quiescing.attempt_count, attempted.attempt_count);
    assert_eq!(quiescing.last_attempt_at, attempted.last_attempt_at);
    assert_eq!(quiescing.phase, SourceRootDeletionPhase::Quiescing);
    assert_eq!(quiescing.affected_documents, 1);
    assert_eq!(
        store.source_root_deletion_document_ids(&root_a.id).unwrap(),
        vec![exclusive.id.clone()]
    );
    assert!(store.document_by_id(&shared.id).unwrap().is_some());
    assert!(store.document_by_id(&exclusive.id).unwrap().is_some());
    assert!(store.source_root(&root_a.id).unwrap().is_some());
    assert!(store.source_root(&root_b.id).unwrap().is_some());
    assert_eq!(
        deletion_transaction_witness(&store, &root_a.id).checkpoint_protocol_version,
        schema_v37::SNAPSHOT_INVARIANT_V2
    );
}

#[test]
fn legacy_quiescing_attempt_reconciles_once_without_stable_retry_amplification() {
    const ROOT_A: &str = "/synthetic/legacy-quiescing-a";
    const ROOT_B: &str = "/synthetic/legacy-quiescing-b";
    const ROOT_C: &str = "/synthetic/legacy-quiescing-metadata-only";
    const SCAN_A: &str = "legacy-scan-a";
    const SCAN_B: &str = "legacy-scan-b";
    const SCAN_C: &str = "legacy-scan-c";
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let requested_at = UnixTimestamp::from_unix_seconds(1_800_303_000);
    let reconciled_at = UnixTimestamp::from_unix_seconds(1_800_303_010);
    let stable_at = UnixTimestamp::from_unix_seconds(1_800_303_011);
    let root_a = register_scanned_root(&store, ROOT_A, "Legacy Quiescing A", SCAN_A, requested_at);
    let root_b = register_scanned_root(&store, ROOT_B, "Legacy Quiescing B", SCAN_B, requested_at);
    let root_c = register_scanned_root(&store, ROOT_C, "Metadata-only", SCAN_C, requested_at);

    let (shared, shared_revision) =
        persist_synthetic_document(&store, "legacy-shared", requested_at);
    observe(
        &store,
        &root_a.id,
        &shared,
        &shared_revision,
        SCAN_A,
        requested_at,
    );
    make_quiescing_receipt_legacy(
        &store,
        &root_a.id,
        requested_at,
        "/synthetic/stale-authority",
    );

    observe(
        &store,
        &root_b.id,
        &shared,
        &shared_revision,
        SCAN_B,
        requested_at,
    );
    let (exclusive, exclusive_revision) =
        persist_synthetic_document(&store, "legacy-exclusive", requested_at);
    let second_revision = SourceRevision::for_content(
        exclusive.id.clone(),
        ContentDigest::from_bytes(b"synthetic second retained hash"),
        b"synthetic second retained hash".len() as u64,
    );
    store.insert_source_revision(&second_revision).unwrap();
    observe(
        &store,
        &root_a.id,
        &exclusive,
        &exclusive_revision,
        SCAN_A,
        requested_at,
    );

    checkpoint::reset_snapshot_census_count();
    let started = store
        .begin_source_root_deletion_attempt(&root_a.id, reconciled_at)
        .unwrap();
    assert_eq!(checkpoint::snapshot_census_count(), 1);
    assert_eq!(started.phase, SourceRootDeletionPhase::Quiescing);
    assert_eq!(started.affected_documents, 1);
    assert_eq!(started.attempt_count, 1);
    assert_eq!(
        deletion_transaction_witness(&store, &root_a.id).canonical_path,
        ROOT_A
    );
    assert_eq!(
        deletion_transaction_witness(&store, &root_a.id).checkpoint_protocol_version,
        schema_v37::SNAPSHOT_INVARIANT_V2
    );
    assert_eq!(
        deletion_snapshot_tuples(&store, &root_a.id),
        vec![
            (
                exclusive.id.as_str().to_string(),
                exclusive_revision.content_hash.as_str().to_string(),
            ),
            (
                exclusive.id.as_str().to_string(),
                second_revision.content_hash.as_str().to_string(),
            ),
        ]
    );
    assert!(store.document_by_id(&shared.id).unwrap().is_some());
    assert!(store.document_by_id(&exclusive.id).unwrap().is_some());

    let before_stable = total_changes(&store);
    let stable = store
        .begin_source_root_deletion_attempt(&root_a.id, stable_at)
        .unwrap();
    assert_eq!(total_changes(&store) - before_stable, 1);
    assert_eq!(checkpoint::snapshot_census_count(), 1);
    assert_eq!(stable.updated_at, started.updated_at);
    assert_eq!(stable.attempt_count, 2);

    let plan = store
        .connection
        .borrow()
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            checkpoint::ATTEMPT_ADMISSION_SQL
        ))
        .unwrap()
        .query_map([root_a.id.as_str()], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let plan = plan.join("\n");
    assert!(
        plan.contains("source_root_deletion") && plan.contains("root_id=?"),
        "{plan}"
    );
    assert!(!plan.contains("source_occurrence"), "{plan}");
    assert!(!plan.contains("source_revision"), "{plan}");
    assert!(!plan.contains("source_root_deletion_document"), "{plan}");

    let (metadata_document, metadata_revision) =
        persist_synthetic_document(&store, "legacy-metadata-only", requested_at);
    observe(
        &store,
        &root_c.id,
        &metadata_document,
        &metadata_revision,
        SCAN_C,
        requested_at,
    );
    make_quiescing_receipt_legacy(
        &store,
        &root_c.id,
        requested_at,
        "/synthetic/stale-metadata-only",
    );
    let census_before_metadata = checkpoint::snapshot_census_count();
    let metadata_before = total_changes(&store);
    store
        .begin_source_root_deletion_attempt(&root_c.id, reconciled_at)
        .unwrap();
    assert_eq!(total_changes(&store) - metadata_before, 2);
    assert_eq!(
        checkpoint::snapshot_census_count(),
        census_before_metadata + 1
    );
    let census_after_metadata = checkpoint::snapshot_census_count();

    store
        .set_source_root_deletion_phase(&root_a.id, SourceRootDeletionPhase::Publishing, stable_at)
        .unwrap();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE source_root_deletion SET checkpoint_protocol_version = ?2
             WHERE root_id = ?1",
            params![root_a.id.as_str(), schema_v37::LEGACY_OR_UNATTESTED],
        )
        .unwrap();
    let later_before = total_changes(&store);
    store
        .begin_source_root_deletion_attempt(&root_a.id, stable_at)
        .unwrap();
    assert_eq!(total_changes(&store) - later_before, 1);
    assert_eq!(checkpoint::snapshot_census_count(), census_after_metadata);
    assert_eq!(
        deletion_transaction_witness(&store, &root_a.id).checkpoint_protocol_version,
        schema_v37::LEGACY_OR_UNATTESTED
    );
}

#[test]
fn legacy_quiescing_attempt_rolls_back_evidence_and_snapshot_on_receipt_failure() {
    const ROOT: &str = "/synthetic/legacy-quiescing-rollback";
    const SCAN: &str = "legacy-rollback-scan";
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let requested_at = UnixTimestamp::from_unix_seconds(1_800_304_000);
    let attempted_at = UnixTimestamp::from_unix_seconds(1_800_304_001);
    let root = register_scanned_root(&store, ROOT, "Legacy rollback", SCAN, requested_at);
    let (document, revision) = persist_synthetic_document(&store, "legacy-rollback", requested_at);
    observe(&store, &root.id, &document, &revision, SCAN, requested_at);
    make_quiescing_receipt_legacy(&store, &root.id, requested_at, "/synthetic/stale-rollback");
    let later_revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(b"synthetic rollback later hash"),
        b"synthetic rollback later hash".len() as u64,
    );
    store.insert_source_revision(&later_revision).unwrap();
    let before = deletion_transaction_witness(&store, &root.id);

    store
        .connection
        .borrow()
        .execute_batch(
            "CREATE TEMP TRIGGER fail_legacy_checkpoint_receipt_update
             BEFORE UPDATE OF checkpoint_protocol_version ON source_root_deletion
             WHEN OLD.checkpoint_protocol_version = 1
              AND NEW.checkpoint_protocol_version = 2
             BEGIN
               SELECT RAISE(ABORT, 'synthetic receipt update failure');
             END;",
        )
        .unwrap();
    let result = store.begin_source_root_deletion_attempt(&root.id, attempted_at);
    store
        .connection
        .borrow()
        .execute_batch("DROP TRIGGER fail_legacy_checkpoint_receipt_update;")
        .unwrap();

    assert_eq!(result.unwrap_err().class(), MetaStoreErrorClass::Storage);
    assert_eq!(deletion_transaction_witness(&store, &root.id), before);
    store
        .begin_source_root_deletion_attempt(&root.id, attempted_at)
        .unwrap();
    assert_eq!(deletion_snapshot_tuples(&store, &root.id).len(), 2);
    assert_eq!(
        deletion_transaction_witness(&store, &root.id).checkpoint_protocol_version,
        schema_v37::SNAPSHOT_INVARIANT_V2
    );
}

#[test]
fn completion_residual_owner_reads_one_typed_row_and_preserves_failed_completion() {
    const ROOT: &str = "/synthetic/completion-residual-owner";
    const SCAN_ID: &str = "completion-residual-owner-scan";
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_300_000);
    let root = store
        .register_source_root(ROOT, ROOT, "Completion residual owner", now)
        .unwrap();
    let source = b"synthetic completion residual source";
    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&["completion-residual-owner"]),
        source_uri: "synthetic://completion-residual-owner/document.pdf".to_string(),
        normalized_path: "synthetic/completion-residual-owner/document.pdf".to_string(),
        file_name: "document.pdf".to_string(),
        extension: FileExtension::Pdf,
        byte_size: source.len() as u64,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::FieldsExtracted,
    };
    let revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(source),
        source.len() as u64,
    );
    document.content_hash = Some(revision.content_hash.as_str().to_string());
    store.upsert_document(&document).unwrap();
    store.insert_source_revision(&revision).unwrap();
    store
        .begin_scan(&root.id, SCAN_ID, ScanTrigger::Manual, now)
        .unwrap();
    store
        .observe_source_occurrence(
            &root.id,
            "document.pdf",
            &document.id,
            &revision.id,
            SCAN_ID,
            now,
        )
        .unwrap();
    {
        let connection = store.connection.borrow();
        connection
            .execute(
                "INSERT INTO pdf_reprocess_job (
                    source_revision_id, root_id, relative_path, parser_contract,
                    state, attempts, scheduled_task_id, queued_at_seconds,
                    updated_at_seconds, completed_at_seconds
                 ) VALUES (?1, ?2, 'document.pdf', 'synthetic_parser_v1',
                           'queued', 0, NULL, ?3, ?3, NULL)",
                params![
                    revision.id.as_str(),
                    root.id.as_str(),
                    now.as_unix_seconds()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO import_task (
                    id, root_path, status, queued_at_seconds,
                    started_at_seconds, finished_at_seconds, updated_at_seconds
                 ) VALUES ('completion-residual-task', ?1, 'queued', ?2, NULL, NULL, ?2)",
                params![ROOT, now.as_unix_seconds()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO authorized_import_root (
                    canonical_root_path, requested_root_path, root_kind,
                    root_preset, scan_profile, scan_budget_kind,
                    scan_budget_limit, paused, updated_at_seconds
                 ) VALUES (?1, ?1, 'explicit', NULL, 'explicit', NULL, NULL, 0, ?2)",
                params![ROOT, now.as_unix_seconds()],
            )
            .unwrap();
    }
    store.begin_source_root_deletion(&root.id, now).unwrap();
    for phase in [
        SourceRootDeletionPhase::Quiescing,
        SourceRootDeletionPhase::Publishing,
        SourceRootDeletionPhase::Purging,
        SourceRootDeletionPhase::Verifying,
    ] {
        store
            .set_source_root_deletion_phase(&root.id, phase, now)
            .unwrap();
    }

    let expected = SourceRootDeletionCompletionResiduals {
        source_occurrences: 1,
        scan_snapshots: 1,
        pdf_reprocess_jobs: 1,
        import_tasks: 1,
        authorized_import_roots: 1,
        documents: 1,
        active_search_projections: 0,
        ocr_page_cache: 0,
        total: 6,
    };
    assert_eq!(
        read_source_root_deletion_completion_residuals(&store.connection.borrow(), &root.id)
            .unwrap(),
        expected
    );
    assert_eq!(
        store
            .complete_source_root_deletion(&root.id, now)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::StorageInvariant
    );
    assert!(store.source_root(&root.id).unwrap().is_some());
    assert_eq!(
        store.source_root_deletion(&root.id).unwrap().unwrap().phase,
        SourceRootDeletionPhase::Verifying
    );
    assert_eq!(
        read_source_root_deletion_completion_residuals(&store.connection.borrow(), &root.id)
            .unwrap(),
        expected
    );

    {
        let connection = store.connection.borrow();
        connection
            .execute(
                "DELETE FROM pdf_reprocess_job WHERE root_id = ?1",
                params![root.id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM source_occurrence WHERE root_id = ?1",
                params![root.id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM scan_snapshot WHERE root_id = ?1",
                params![root.id.as_str()],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM import_task WHERE root_path = ?1",
                params![ROOT],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM authorized_import_root WHERE canonical_root_path = ?1",
                params![ROOT],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM document WHERE id = ?1",
                params![document.id.as_str()],
            )
            .unwrap();
    }
    assert_eq!(
        read_source_root_deletion_completion_residuals(&store.connection.borrow(), &root.id)
            .unwrap(),
        SourceRootDeletionCompletionResiduals::default()
    );
    let completed = store.complete_source_root_deletion(&root.id, now).unwrap();
    assert_eq!(completed.phase, SourceRootDeletionPhase::Complete);
    assert!(store.source_root(&root.id).unwrap().is_none());
}

#[test]
fn completion_residual_counts_unreferenced_ocr_page_cache_and_spares_shared_hash() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_300_100);
    let doomed_root = register_scanned_root(
        &store,
        "/synthetic/ocr-residual-doomed",
        "OCR residual doomed",
        "ocr-residual-doomed-scan",
        now,
    );
    let sibling_root = register_scanned_root(
        &store,
        "/synthetic/ocr-residual-sibling",
        "OCR residual sibling",
        "ocr-residual-sibling-scan",
        now,
    );
    let (exclusive, exclusive_revision) = persist_synthetic_document(&store, "ocr-exclusive", now);
    observe(
        &store,
        &doomed_root.id,
        &exclusive,
        &exclusive_revision,
        "ocr-residual-doomed-scan",
        now,
    );

    // Distinct documents with identical content bytes: doomed snapshot owns one;
    // sibling retains the other so OCR for that hash must be spared.
    let shared_bytes = b"synthetic shared ocr residual bytes";
    let mut doomed_shared = Document {
        id: DocumentId::from_non_secret_parts(&["source-root-deletion", "ocr-shared-doomed"]),
        source_uri: "synthetic://source-root-deletion/ocr-shared-doomed.pdf".to_string(),
        normalized_path: "/synthetic/ocr-residual-doomed/shared.pdf".to_string(),
        file_name: "shared.pdf".to_string(),
        extension: FileExtension::Pdf,
        byte_size: shared_bytes.len() as u64,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::FieldsExtracted,
    };
    let doomed_shared_revision = SourceRevision::for_content(
        doomed_shared.id.clone(),
        ContentDigest::from_bytes(shared_bytes),
        shared_bytes.len() as u64,
    );
    doomed_shared.content_hash = Some(doomed_shared_revision.content_hash.as_str().to_string());
    store.upsert_document(&doomed_shared).unwrap();
    store
        .insert_source_revision(&doomed_shared_revision)
        .unwrap();
    observe(
        &store,
        &doomed_root.id,
        &doomed_shared,
        &doomed_shared_revision,
        "ocr-residual-doomed-scan",
        now,
    );

    let mut sibling_shared = Document {
        id: DocumentId::from_non_secret_parts(&["source-root-deletion", "ocr-shared-sibling"]),
        source_uri: "synthetic://source-root-deletion/ocr-shared-sibling.pdf".to_string(),
        normalized_path: "/synthetic/ocr-residual-sibling/shared.pdf".to_string(),
        file_name: "shared.pdf".to_string(),
        extension: FileExtension::Pdf,
        byte_size: shared_bytes.len() as u64,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::FieldsExtracted,
    };
    let sibling_shared_revision = SourceRevision::for_content(
        sibling_shared.id.clone(),
        ContentDigest::from_bytes(shared_bytes),
        shared_bytes.len() as u64,
    );
    sibling_shared.content_hash = Some(sibling_shared_revision.content_hash.as_str().to_string());
    store.upsert_document(&sibling_shared).unwrap();
    store
        .insert_source_revision(&sibling_shared_revision)
        .unwrap();
    observe(
        &store,
        &sibling_root.id,
        &sibling_shared,
        &sibling_shared_revision,
        "ocr-residual-sibling-scan",
        now,
    );

    let exclusive_hash = exclusive.content_hash.clone().unwrap();
    let shared_hash = doomed_shared.content_hash.clone().unwrap();
    assert_eq!(shared_hash, sibling_shared.content_hash.clone().unwrap());
    {
        let connection = store.connection.borrow();
        for hash in [&exclusive_hash, &shared_hash] {
            connection
                .execute(
                    "INSERT INTO ocr_page_cache (
                        file_content_hash, page_no, render_dpi, ocr_lang, ocr_profile,
                        text, confidence, engine_profile, duration_ms, status, error_kind,
                        updated_at_seconds
                     ) VALUES (?1, 1, 300, 'eng', 'balanced', 'synthetic', 0.9, 'fixture', 1,
                               'succeeded', NULL, ?2)",
                    params![hash, now.as_unix_seconds()],
                )
                .unwrap();
        }
    }

    store
        .begin_source_root_deletion(&doomed_root.id, now)
        .unwrap();
    for phase in [
        SourceRootDeletionPhase::Quiescing,
        SourceRootDeletionPhase::Publishing,
        SourceRootDeletionPhase::Purging,
    ] {
        store
            .set_source_root_deletion_phase(&doomed_root.id, phase, now)
            .unwrap();
    }
    store.purge_source_root_data(&doomed_root.id, now).unwrap();

    let residuals =
        read_source_root_deletion_completion_residuals(&store.connection.borrow(), &doomed_root.id)
            .unwrap();
    assert_eq!(residuals.ocr_page_cache, 1);
    assert_eq!(residuals.active_search_projections, 0);
    assert!(residuals.total >= 1);
    assert_eq!(
        store
            .complete_source_root_deletion(&doomed_root.id, now)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::StorageInvariant
    );

    store
        .purge_ocr_page_cache_by_content_hashes(&[exclusive_hash])
        .unwrap();
    assert_eq!(
        store
            .ocr_page_cache_entries_for_content_hashes(&[shared_hash.clone()])
            .unwrap()
            .len(),
        1
    );
    loop {
        let report = store
            .purge_source_root_deleted_documents(&doomed_root.id)
            .unwrap();
        if report.remaining_tombstones == 0 {
            break;
        }
    }
    let residuals =
        read_source_root_deletion_completion_residuals(&store.connection.borrow(), &doomed_root.id)
            .unwrap();
    assert_eq!(residuals.ocr_page_cache, 0);
    assert_eq!(residuals.documents, 0);
    assert!(residuals.is_empty());
    let completed = store
        .complete_source_root_deletion(&doomed_root.id, now)
        .unwrap();
    assert_eq!(completed.phase, SourceRootDeletionPhase::Complete);
    assert_eq!(
        store
            .ocr_page_cache_entries_for_content_hashes(&[shared_hash])
            .unwrap()
            .len(),
        1
    );
    assert!(store.document_by_id(&sibling_shared.id).unwrap().is_some());
}

#[test]
fn completion_residual_counts_active_search_projection_for_snapshot_docs() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_300_200);
    let root = register_scanned_root(
        &store,
        "/synthetic/projection-residual",
        "Projection residual",
        "projection-residual-scan",
        now,
    );
    let (mut document, revision) = persist_synthetic_document(&store, "projection-residual", now);
    document.status = DocumentStatus::Searchable;
    document.updated_at = now;
    store.upsert_document(&document).unwrap();
    observe(
        &store,
        &root.id,
        &document,
        &revision,
        "projection-residual-scan",
        now,
    );
    let content_hash = document.content_hash.clone().unwrap();
    let version_id = format!("sha256:{}", "a".repeat(64));
    let generation = "projection-residual-gen";
    {
        let connection = store.connection.borrow();
        connection
            .execute(
                "INSERT INTO resume_version (
                    id, document_id, source_revision_id, normalized_text_hash,
                    parse_version, schema_version, language_set_json, page_count,
                    raw_text, clean_text, quality_score
                 ) VALUES (?1, ?2, ?3, ?4, 'parser-v1', 'schema-v1', '[]', 1,
                           'synthetic', 'synthetic', 0.9)",
                params![
                    version_id,
                    document.id.as_str(),
                    revision.id.as_str(),
                    content_hash
                ],
            )
            .unwrap();
        // Residual proof only needs a stuck projection row for a snapshot document;
        // bypass guarded-publication insert triggers that require a live commit CAS.
        connection
            .execute_batch(
                "PRAGMA foreign_keys=OFF;
                 DROP TRIGGER IF EXISTS active_projection_requires_validated_publication;
                 DROP TRIGGER IF EXISTS active_search_projection_exact_version_metadata;",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO active_search_projection (
                    document_id, resume_version_id, generation,
                    source_uri, normalized_path, file_name, extension,
                    byte_size, mtime_seconds, content_hash, text_hash,
                    is_deleted, created_at_seconds, updated_at_seconds, status
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pdf', ?7, ?8, ?9, NULL,
                           0, ?8, ?8, 'searchable')",
                params![
                    document.id.as_str(),
                    version_id,
                    generation,
                    document.source_uri,
                    document.normalized_path,
                    document.file_name,
                    document.byte_size as i64,
                    now.as_unix_seconds(),
                    content_hash
                ],
            )
            .unwrap();
    }

    store.begin_source_root_deletion(&root.id, now).unwrap();
    for phase in [
        SourceRootDeletionPhase::Quiescing,
        SourceRootDeletionPhase::Publishing,
        SourceRootDeletionPhase::Purging,
    ] {
        store
            .set_source_root_deletion_phase(&root.id, phase, now)
            .unwrap();
    }
    store.purge_source_root_data(&root.id, now).unwrap();
    let residuals =
        read_source_root_deletion_completion_residuals(&store.connection.borrow(), &root.id)
            .unwrap();
    assert_eq!(residuals.active_search_projections, 1);
    assert!(residuals.total >= 1);
    assert_eq!(
        store
            .complete_source_root_deletion(&root.id, now)
            .unwrap_err()
            .class(),
        MetaStoreErrorClass::StorageInvariant
    );
}

#[test]
fn deletion_attempt_evidence_rejects_unproduced_error_codes() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_301_000);
    let root = store
        .register_source_root(
            "/synthetic/error-vocabulary",
            "/synthetic/error-vocabulary",
            "Synthetic error vocabulary",
            now,
        )
        .unwrap();
    store.begin_source_root_deletion(&root.id, now).unwrap();
    store
        .begin_source_root_deletion_attempt(&root.id, now)
        .unwrap();

    for retired_code in ["receipt_unavailable", "service_unavailable"] {
        let result = store.connection.borrow().execute(
            "UPDATE source_root_deletion_attempt_evidence
             SET last_error_phase = 'requested',
                 last_error_code = ?2,
                 last_error_at_seconds = ?3
             WHERE root_id = ?1",
            rusqlite::params![root.id.as_str(), retired_code, now.as_unix_seconds()],
        );
        assert!(
            result.is_err(),
            "retired error code {retired_code} persisted"
        );
    }
}

#[test]
fn saturated_attempt_count_does_not_block_the_next_deletion_attempt() {
    let store = EphemeralMetaStore::open_in_memory().unwrap();
    store.run_migrations().unwrap();
    let requested_at = UnixTimestamp::from_unix_seconds(1_800_302_000);
    let next_attempt_at = UnixTimestamp::from_unix_seconds(1_800_302_001);
    let root = store
        .register_source_root(
            "/synthetic/saturated-attempt",
            "/synthetic/saturated-attempt",
            "Synthetic saturated attempt",
            requested_at,
        )
        .unwrap();
    store
        .begin_source_root_deletion(&root.id, requested_at)
        .unwrap();
    store
        .connection
        .borrow()
        .execute(
            "UPDATE source_root_deletion_attempt_evidence
             SET attempt_count = 9007199254740991,
                 last_attempt_at_seconds = ?2,
                 last_error_phase = 'requested',
                 last_error_code = 'internal',
                 last_error_at_seconds = ?2
             WHERE root_id = ?1",
            rusqlite::params![root.id.as_str(), requested_at.as_unix_seconds()],
        )
        .unwrap();

    let saturated = store
        .begin_source_root_deletion_attempt(&root.id, next_attempt_at)
        .unwrap();

    assert_eq!(saturated.attempt_count, 9_007_199_254_740_991);
    assert_eq!(saturated.last_attempt_at, Some(next_attempt_at));
    assert_eq!(saturated.last_error_phase, None);
    assert_eq!(saturated.last_error_code, None);
    assert_eq!(saturated.last_error_at, None);
}
