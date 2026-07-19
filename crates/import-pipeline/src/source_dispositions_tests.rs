use meta_store::{
    ClassificationStatus, ContentDigest, Document, DocumentId, DocumentStatus, FileExtension,
    ImportProcessingContract, ImportRootKind, ImportScanProfile, ImportScanScope,
    ImportSourceDispositionKind, ImportTask, ImportTaskId, ImportTaskSourceDisposition,
    ImportTaskStatus, OwnedMetaStore, ReasonCode, ResumeVersion, ResumeVersionClassification,
    ResumeVersionId, ReviewDisposition, SourceRevision, UnixTimestamp,
};
use resume_classifier::LinearPromotionPolicy;
use tempfile::tempdir;

use super::*;
use crate::{current_import_processing_contract, persist_source_revision_failure, ImportOptions};

#[test]
fn disposition_buffer_flushes_at_the_bounded_batch_limit() {
    let directory = tempdir().unwrap();
    let store = owned_store(directory.path());
    let now = UnixTimestamp::from_unix_seconds(1_700_000_080);
    let (task, contract) = running_task(&store, "bounded-disposition-batches", now);
    let mut batches = ImportDispositionBatches::new(task.id.clone(), contract.id().clone());

    for source_ordinal in 0..meta_store::IMPORT_SOURCE_DISPOSITION_BATCH_LIMIT {
        let source_label = format!("bounded-source-{source_ordinal}");
        let mut document = test_document(&source_label, DocumentStatus::FailedPermanent, now);
        let source_revision = SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(source_label.as_bytes()),
            source_label.len() as u64,
        );
        document.content_hash = Some(source_revision.content_hash.as_str().to_string());
        document.byte_size = source_revision.byte_size;
        persist_source_revision_failure(
            &store,
            &document,
            &source_revision,
            now,
            &LinearPromotionPolicy::default(),
        )
        .unwrap();
        batches.record(
            ImportTaskSourceDisposition {
                source_ordinal: u64::try_from(source_ordinal).unwrap(),
                document_id: document.id,
                source_revision_id: source_revision.id,
                resume_version_id: None,
                kind: ImportSourceDispositionKind::Failed,
            },
            DispositionStaging::Ready,
        );
        batches.flush_ready_if_full(&store).unwrap();
    }

    assert!(batches.ready.is_empty());
    assert!(batches.pending_searchable.is_empty());
    let mut final_scope = import_scan_scope(&task, now);
    final_scope.files_discovered =
        u64::try_from(meta_store::IMPORT_SOURCE_DISPOSITION_BATCH_LIMIT).unwrap();
    final_scope.failed_documents = final_scope.files_discovered;
    let completion = store
        .complete_import_task(&task.id, contract.id(), &final_scope, now)
        .unwrap();
    assert_eq!(
        completion.source_disposition_count,
        u64::try_from(meta_store::IMPORT_SOURCE_DISPOSITION_BATCH_LIMIT).unwrap()
    );
}

#[test]
fn searchable_disposition_waits_until_searchable_staging_completes() {
    let directory = tempdir().unwrap();
    let store = owned_store(directory.path());
    let now = UnixTimestamp::from_unix_seconds(1_700_000_081);
    let (task, contract) = running_task(&store, "searchable-disposition-order", now);
    let document_id = DocumentId::from_non_secret_parts(&["searchable-disposition-order-doc"]);
    let source_revision = SourceRevision::for_content(
        document_id.clone(),
        ContentDigest::from_bytes(b"searchable source"),
        17,
    );
    let mut document = test_document(
        "searchable-disposition-order-doc",
        DocumentStatus::Searchable,
        now,
    );
    document.content_hash = Some(source_revision.content_hash.as_str().to_string());
    document.byte_size = source_revision.byte_size;
    let normalized_text_hash = ContentDigest::from_bytes(b"searchable text");
    let version_id = ResumeVersionId::from_content_identity(
        &document_id,
        &source_revision.id,
        &normalized_text_hash,
        contract.primary_parse_version(),
        contract.derived_schema_version(),
    );
    let version = ResumeVersion {
        id: version_id.clone(),
        document_id: document_id.clone(),
        source_revision_id: source_revision.id.clone(),
        normalized_text_hash,
        parse_version: contract.primary_parse_version().to_string(),
        schema_version: contract.derived_schema_version().to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: None,
        clean_text: Some("searchable text".to_string()),
        quality_score: Some(0.9),
    };
    let classification = ResumeVersionClassification {
        resume_version_id: version.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        classifier_epoch: contract.classifier_epoch().to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        classified_at: now,
        review_disposition: ReviewDisposition::NotRequired,
    };
    crate::immutable_ingest::stage(
        &store,
        crate::immutable_ingest::StagedResume {
            document: &document,
            source_revision: &source_revision,
            derived: crate::immutable_ingest::StagedDerivedData::ClassifiedVersion {
                version: &version,
                classification: &classification,
                mentions: &[],
                email_hash: None,
                phone_hash: None,
            },
        },
    )
    .unwrap();
    let disposition = ImportTaskSourceDisposition {
        source_ordinal: 0,
        document_id: document_id.clone(),
        source_revision_id: source_revision.id.clone(),
        resume_version_id: Some(version_id),
        kind: ImportSourceDispositionKind::Searchable,
    };
    let mut batches = ImportDispositionBatches::new(task.id.clone(), contract.id().clone());
    batches.record(
        disposition.clone(),
        DispositionStaging::SearchableFactsPending,
    );

    batches
        .searchable_staging_completed(SearchableStagingState::Pending, &store)
        .unwrap();
    assert!(batches.ready.is_empty());
    assert_eq!(batches.pending_searchable.len(), 1);

    batches
        .searchable_staging_completed(SearchableStagingState::Completed, &store)
        .unwrap();
    assert!(batches.ready.is_empty());
    assert!(batches.pending_searchable.is_empty());
    let replay = store
        .stage_import_task_source_dispositions(&task.id, contract.id(), &[disposition])
        .unwrap();
    assert_eq!(replay.inserted, 0);
    assert_eq!(replay.already_present, 1);
}

fn owned_store(data_dir: &std::path::Path) -> OwnedMetaStore {
    let owner = match meta_store::DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        meta_store::DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        meta_store::DataDirectoryOwnerAcquisition::Contended => {
            panic!("synthetic data dir is owned")
        }
    };
    owner.open_store().unwrap()
}

fn running_task(
    store: &OwnedMetaStore,
    label: &str,
    now: UnixTimestamp,
) -> (ImportTask, ImportProcessingContract) {
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&[label]),
        root_path: "/fixture/root".to_string(),
        status: ImportTaskStatus::Queued,
        queued_at: now,
        started_at: None,
        finished_at: None,
        updated_at: now,
    };
    let contract = current_import_processing_contract(&ImportOptions::default()).unwrap();
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    store
        .insert_import_task_with_scan_scope(
            &ImportTask {
                id: ImportTaskId::from_non_secret_parts(&[label, "root-seed"]),
                ..task.clone()
            },
            &ImportScanScope {
                import_task_id: ImportTaskId::from_non_secret_parts(&[label, "root-seed"]),
                ..import_scan_scope(&task, now)
            },
            &contract,
        )
        .unwrap();
    let seed_id = ImportTaskId::from_non_secret_parts(&[label, "root-seed"]);
    store.cancel_import_task(&seed_id, now).unwrap();
    store
        .enqueue_full_corpus_migration_rebuild_root(&task.root_path, &task.id, &contract, now)
        .unwrap();
    let task = store
        .claim_observed_import_task_for_worker(&task, now)
        .unwrap()
        .expect("new test import task must be claimable");
    (task, contract)
}

fn import_scan_scope(task: &ImportTask, now: UnixTimestamp) -> ImportScanScope {
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
        updated_at: now,
    }
}

fn test_document(label: &str, status: DocumentStatus, now: UnixTimestamp) -> Document {
    Document {
        id: DocumentId::from_non_secret_parts(&[label]),
        source_uri: format!("file:///fixture/{label}.txt"),
        normalized_path: format!("/fixture/{label}.txt"),
        file_name: format!("{label}.txt"),
        extension: FileExtension::Txt,
        byte_size: 0,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status,
    }
}
