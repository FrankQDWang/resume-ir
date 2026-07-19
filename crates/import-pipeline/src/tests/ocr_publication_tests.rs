use std::fs;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use meta_store::{
    IngestJobStatus, MetaStoreErrorClass, OcrAttemptFailure, OcrAttemptFailureOutcome,
    OwnedMetaStore, UnixTimestamp,
};

use super::*;

struct CompetingPublicationVectorizer {
    store: Mutex<OwnedMetaStore>,
    contention_observed: AtomicBool,
}

impl SearchPublicationVectorizer for CompetingPublicationVectorizer {
    fn model_id(&self) -> &str {
        "synthetic-publication-v1"
    }

    fn dimension(&self) -> usize {
        2
    }

    fn max_batch_inputs(&self) -> usize {
        4
    }

    fn max_text_bytes(&self) -> usize {
        65_536
    }

    fn embed_batch(
        &self,
        inputs: &[SearchPublicationEmbeddingInput],
        _is_cancelled: &dyn Fn() -> bool,
    ) -> std::result::Result<Vec<SearchPublicationEmbeddingOutput>, SearchPublicationEmbeddingFailure>
    {
        if !self.contention_observed.swap(true, Ordering::SeqCst) {
            let contender_store = self.store.lock().unwrap().open_sibling().unwrap();
            let error = std::thread::spawn(move || {
                contender_store
                    .try_acquire_search_publication_session()
                    .unwrap_err()
            })
            .join()
            .unwrap();
            assert_eq!(
                error.class(),
                MetaStoreErrorClass::MigrationOwnershipRequired
            );
        }
        Ok(inputs
            .iter()
            .map(|input| {
                SearchPublicationEmbeddingOutput::new(
                    input.id(),
                    self.model_id(),
                    vec![1.0, input.text().len() as f32],
                )
            })
            .collect())
    }
}

fn assert_ocr_publication_facts_absent(
    store: &OwnedMetaStore,
    document: &Document,
    claimed: &meta_store::ClaimedOcrJob,
    expected_job_status: IngestJobStatus,
) {
    assert_eq!(
        store.document_by_id(&document.id).unwrap().unwrap().status,
        DocumentStatus::OcrRequired
    );
    assert!(store
        .resume_versions_for_document(&document.id)
        .unwrap()
        .is_empty());
    assert_eq!(
        store
            .active_search_projection_for_document(&document.id)
            .unwrap(),
        None
    );
    let job = store.ingest_job_by_id(&claimed.job.id).unwrap().unwrap();
    assert_eq!(job.status, expected_job_status);
    assert_eq!(job.resume_version_id, None);
}

fn transition_failed_ocr_publication_to_retryable(
    store: &OwnedMetaStore,
    claimed: &meta_store::ClaimedOcrJob,
    now: UnixTimestamp,
) {
    assert_eq!(
        store
            .finish_ocr_attempt_failure(claimed, OcrAttemptFailure::Retryable, now)
            .unwrap(),
        OcrAttemptFailureOutcome::Retryable
    );
}

#[test]
fn ocr_fulltext_failure_keeps_all_derived_facts_outside_the_store() {
    let temp = TestDir::new("ocr-publication-fulltext-failure");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let store = create_test_store(&data_dir);
    initialize_ready_empty_search(
        &data_dir,
        &store,
        UnixTimestamp::from_unix_seconds(1_700_001_000),
    );
    let head_before = ready_search_head(&store);
    let document = test_document("ocr-fulltext-failure", DocumentStatus::OcrRequired);
    let claimed = claim_ocr_document(
        &store,
        &document,
        UnixTimestamp::from_unix_seconds(1_700_001_001),
    );
    fs::rename(
        data_dir.join("search-index"),
        data_dir.join("search-index-valid"),
    )
    .unwrap();
    fs::write(data_dir.join("search-index"), b"not-a-directory").unwrap();

    let error = index_claimed_ocr_text(
        &data_dir,
        &store,
        &claimed,
        &synthetic_resume_text("Fulltext Failure", "Rust Search"),
        Some(0.9),
        Some(1),
        UnixTimestamp::from_unix_seconds(1_700_001_002),
        &SearchPublicationVectorization::default(),
    )
    .unwrap_err();

    assert_eq!(error.class(), ImportPipelineErrorClass::FullText);
    assert_eq!(ready_search_head(&store), head_before);
    assert_ocr_publication_facts_absent(&store, &document, &claimed, IngestJobStatus::Running);
    transition_failed_ocr_publication_to_retryable(
        &store,
        &claimed,
        UnixTimestamp::from_unix_seconds(1_700_001_003),
    );
    assert_ocr_publication_facts_absent(
        &store,
        &document,
        &claimed,
        IngestJobStatus::FailedRetryable,
    );
}

#[test]
fn ocr_vector_failure_keeps_claim_and_search_head_retryable() {
    let temp = TestDir::new("ocr-publication-vector-failure");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let store = create_test_store(&data_dir);
    initialize_ready_empty_search(
        &data_dir,
        &store,
        UnixTimestamp::from_unix_seconds(1_700_001_010),
    );
    let head_before = ready_search_head(&store);
    let document = test_document("ocr-vector-failure", DocumentStatus::OcrRequired);
    let claimed = claim_ocr_document(
        &store,
        &document,
        UnixTimestamp::from_unix_seconds(1_700_001_011),
    );
    let failing =
        SearchPublicationVectorization::enabled(Arc::new(TestPublicationVectorizer { fail: true }));

    let error = index_claimed_ocr_text(
        &data_dir,
        &store,
        &claimed,
        &synthetic_resume_text("Vector Failure", "Rust Search"),
        Some(0.9),
        Some(1),
        UnixTimestamp::from_unix_seconds(1_700_001_012),
        &failing,
    )
    .unwrap_err();

    assert_eq!(error.class(), ImportPipelineErrorClass::EmbeddingRuntime);
    assert!(error.is_retryable());
    assert_eq!(ready_search_head(&store), head_before);
    assert!(store
        .interrupted_search_publications(256)
        .unwrap()
        .is_empty());
    assert_ocr_publication_facts_absent(&store, &document, &claimed, IngestJobStatus::Running);
    transition_failed_ocr_publication_to_retryable(
        &store,
        &claimed,
        UnixTimestamp::from_unix_seconds(1_700_001_013),
    );
    assert_ocr_publication_facts_absent(
        &store,
        &document,
        &claimed,
        IngestJobStatus::FailedRetryable,
    );
}

#[test]
fn ocr_publication_session_excludes_competing_writer_and_commits_the_claim() {
    let temp = TestDir::new("ocr-publication-session-exclusion");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let store = create_test_store(&data_dir);
    initialize_ready_empty_search(
        &data_dir,
        &store,
        UnixTimestamp::from_unix_seconds(1_700_001_020),
    );
    let head_before = ready_search_head(&store);
    let document = test_document("ocr-session-exclusion", DocumentStatus::OcrRequired);
    let claimed = claim_ocr_document(
        &store,
        &document,
        UnixTimestamp::from_unix_seconds(1_700_001_021),
    );
    let competing_vectorizer = Arc::new(CompetingPublicationVectorizer {
        store: Mutex::new(store.open_sibling().unwrap()),
        contention_observed: AtomicBool::new(false),
    });
    let vectorization = SearchPublicationVectorization::enabled(competing_vectorizer.clone());

    let outcome = index_claimed_ocr_text(
        &data_dir,
        &store,
        &claimed,
        &synthetic_resume_text("Session Exclusion", "Rust Search"),
        Some(0.9),
        Some(1),
        UnixTimestamp::from_unix_seconds(1_700_001_022),
        &vectorization,
    )
    .unwrap();

    let OcrTextIndexOutcome::Committed(summary) = outcome else {
        panic!("current OCR claim must commit under its publication session");
    };
    assert!(summary.searchable);
    assert!(competing_vectorizer
        .contention_observed
        .load(Ordering::SeqCst));
    assert_ne!(ready_search_head(&store).generation, head_before.generation);
    assert!(store
        .interrupted_search_publications(256)
        .unwrap()
        .is_empty());
    assert!(active_resume_version(&store, &document).is_some());
    assert_eq!(
        store.document_by_id(&document.id).unwrap().unwrap().status,
        DocumentStatus::Searchable
    );
    assert_eq!(
        store
            .ingest_job_by_id(&claimed.job.id)
            .unwrap()
            .unwrap()
            .status,
        IngestJobStatus::Completed
    );
}
