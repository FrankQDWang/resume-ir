use rusqlite::params;

use super::{
    read_source_root_deletion_completion_residuals, SourceRootDeletionCompletionResiduals,
};
use crate::{
    ContentDigest, Document, DocumentId, DocumentStatus, EphemeralMetaStore, FileExtension,
    MetaStoreErrorClass, ScanTrigger, SourceRevision, SourceRootDeletionPhase, UnixTimestamp,
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
