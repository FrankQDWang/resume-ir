use super::*;

struct CountingPublicationVectorizer {
    calls: Arc<AtomicUsize>,
}

impl SearchPublicationVectorizer for CountingPublicationVectorizer {
    fn model_id(&self) -> &str {
        "synthetic-no-op-publication-v1"
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
        self.calls.fetch_add(1, Ordering::SeqCst);
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

#[test]
fn publication_boundary_discards_removal_without_an_active_projection() {
    let temp = TestDir::new("import-pipeline-idempotent-removal-boundary");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let store = create_test_store(&data_dir);
    initialize_ready_empty_search(
        &data_dir,
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_194),
    );
    let first_head = ready_search_head(&store);
    let mut pending_documents = Vec::new();
    let mut pending_removals = PendingProjectionRemovals::default();
    pending_removals
        .schedule(
            DocumentId::from_non_secret_parts(&["never-projected"]),
            SearchProjectionRemovalReason::PermanentClassificationExclusion,
            None,
        )
        .unwrap();

    assert!(!flush_pending_searchable_documents(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_195),
        &mut ImportSummary::default(),
        &mut pending_documents,
        &mut pending_removals,
        None,
        CurrentImportCacheMode::Retain,
        &|| Ok(()),
        &|_| {},
        Instant::now(),
        H2_INDEX_WRITER_HEAP_BYTES,
        &SearchPublicationVectorization::default(),
    )
    .unwrap());
    assert_eq!(ready_search_head(&store), first_head);
}

#[test]
fn publication_boundary_discards_an_exact_searchable_replacement() {
    let temp = TestDir::new("import-pipeline-idempotent-replacement-boundary");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let store = create_test_store(&data_dir);
    initialize_ready_empty_search(
        &data_dir,
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_194),
    );
    let mut first_pending = vec![test_pending_searchable_document("exact-replacement")];
    assert!(flush_pending_searchable_documents(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_195),
        &mut ImportSummary::default(),
        &mut first_pending,
        &mut PendingProjectionRemovals::default(),
        None,
        CurrentImportCacheMode::Retain,
        &|| Ok(()),
        &|_| {},
        Instant::now(),
        H2_INDEX_WRITER_HEAP_BYTES,
        &SearchPublicationVectorization::default(),
    )
    .unwrap());
    let first_head = ready_search_head(&store);
    let mut second_pending = vec![test_pending_searchable_document("exact-replacement")];
    let mut second_summary = ImportSummary::default();

    assert!(!flush_pending_searchable_documents(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_196),
        &mut second_summary,
        &mut second_pending,
        &mut PendingProjectionRemovals::default(),
        None,
        CurrentImportCacheMode::Retain,
        &|| Ok(()),
        &|_| {},
        Instant::now(),
        H2_INDEX_WRITER_HEAP_BYTES,
        &SearchPublicationVectorization::default(),
    )
    .unwrap());
    assert_eq!(second_summary.searchable_documents, 1);
    assert_eq!(ready_search_head(&store), first_head);
}

#[test]
fn publication_boundary_counts_exact_replacement_alongside_a_real_delta() {
    let temp = TestDir::new("import-pipeline-mixed-replacement-boundary");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let store = create_test_store(&data_dir);
    initialize_ready_empty_search(
        &data_dir,
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_194),
    );
    let mut first_pending = vec![test_pending_searchable_document("exact-replacement")];
    assert!(flush_pending_searchable_documents(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_195),
        &mut ImportSummary::default(),
        &mut first_pending,
        &mut PendingProjectionRemovals::default(),
        None,
        CurrentImportCacheMode::Retain,
        &|| Ok(()),
        &|_| {},
        Instant::now(),
        H2_INDEX_WRITER_HEAP_BYTES,
        &SearchPublicationVectorization::default(),
    )
    .unwrap());
    let first_head = ready_search_head(&store);
    let mut mixed_pending = vec![
        test_pending_searchable_document("exact-replacement"),
        test_pending_searchable_document("real-delta"),
    ];
    let mut mixed_summary = ImportSummary::default();

    assert!(flush_pending_searchable_documents(
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_196),
        &mut mixed_summary,
        &mut mixed_pending,
        &mut PendingProjectionRemovals::default(),
        None,
        CurrentImportCacheMode::Retain,
        &|| Ok(()),
        &|_| {},
        Instant::now(),
        H2_INDEX_WRITER_HEAP_BYTES,
        &SearchPublicationVectorization::default(),
    )
    .unwrap());
    assert_eq!(mixed_summary.searchable_documents, 2);
    let second_head = ready_search_head(&store);
    assert_ne!(second_head.generation, first_head.generation);
    assert_eq!(
        second_head.visible_epoch,
        first_head.visible_epoch.checked_add(1).unwrap()
    );
}

#[test]
fn publication_boundary_normalizes_after_waiting_for_the_owner_session() {
    let temp = TestDir::new("import-pipeline-owner-bound-normalization");
    let data_dir = temp.path().join("data");
    fs::create_dir_all(&data_dir).unwrap();
    let store = create_test_store(&data_dir);
    initialize_ready_empty_search(
        &data_dir,
        &store,
        UnixTimestamp::from_unix_seconds(1_700_000_194),
    );
    let holder_store = store.open_sibling().unwrap();
    let waiter_store = store.open_sibling().unwrap();
    let (holder_ready_tx, holder_ready_rx) = mpsc::sync_channel(1);
    let (release_holder_tx, release_holder_rx) = mpsc::sync_channel(1);
    let (waiter_ready_tx, waiter_ready_rx) = mpsc::sync_channel(1);

    thread::scope(|scope| {
        let holder = scope.spawn(move || {
            let publication_session = holder_store.wait_for_search_publication_session().unwrap();
            let target_index_document =
                stage_test_index_document(&holder_store, "concurrent-target");
            holder_ready_tx.send(()).unwrap();
            release_holder_rx.recv().unwrap();
            let prepared = write_incremental_search_artifacts_for_test(
                &publication_session,
                UnixTimestamp::from_unix_seconds(1_700_000_195),
                CLASSIFIER_EPOCH,
                vec![target_index_document],
                &BTreeSet::new(),
                0,
                0,
                None,
                CurrentImportCacheMode::Retain,
                None,
                None,
                None,
                H2_INDEX_WRITER_HEAP_BYTES,
                &SearchPublicationVectorization::default(),
            )
            .unwrap();
            commit_prepared_search_publication_for_test(
                UnixTimestamp::from_unix_seconds(1_700_000_195),
                prepared,
                &[terminal_searchable_document(
                    &holder_store,
                    "concurrent-target",
                    UnixTimestamp::from_unix_seconds(1_700_000_195),
                )],
            )
            .unwrap()
            .release();
        });
        holder_ready_rx.recv().unwrap();

        let waiter = scope.spawn(move || {
            let mut pending_documents = vec![test_pending_searchable_document("real-delta")];
            let mut pending_removals = PendingProjectionRemovals::default();
            pending_removals
                .schedule(
                    DocumentId::from_non_secret_parts(&["concurrent-target"]),
                    SearchProjectionRemovalReason::PermanentClassificationExclusion,
                    Some(test_document("concurrent-target", DocumentStatus::Excluded)),
                )
                .unwrap();
            let published = flush_pending_searchable_documents(
                &waiter_store,
                UnixTimestamp::from_unix_seconds(1_700_000_196),
                &mut ImportSummary::default(),
                &mut pending_documents,
                &mut pending_removals,
                None,
                CurrentImportCacheMode::Retain,
                &|| Ok(()),
                &|phase| {
                    if phase == ImportCancelCheckPhase::IndexPublication {
                        waiter_ready_tx.send(()).unwrap();
                    }
                },
                Instant::now(),
                H2_INDEX_WRITER_HEAP_BYTES,
                &SearchPublicationVectorization::default(),
            )
            .unwrap();
            assert!(published);
        });
        waiter_ready_rx.recv().unwrap();
        release_holder_tx.send(()).unwrap();
        holder.join().unwrap();
        waiter.join().unwrap();
    });

    assert!(store
        .active_search_projection_for_document(&DocumentId::from_non_secret_parts(&[
            "concurrent-target"
        ]))
        .unwrap()
        .is_none());
    assert!(store
        .active_search_projection_for_document(&DocumentId::from_non_secret_parts(&["real-delta"]))
        .unwrap()
        .is_some());
}

#[test]
fn metadata_only_searchable_change_still_publishes() {
    let temp = TestDir::new("import-pipeline-metadata-only-publication");
    let data_dir = temp.path().join("data");
    let root = temp.path().join("resumes");
    let source = root.join("resume.txt");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(
        &source,
        synthetic_resume_text("Synthetic Candidate", "Rust Search"),
    )
    .unwrap();
    let store = create_test_store(&data_dir);
    let first_now = UnixTimestamp::from_unix_seconds(1_700_000_196);
    let first_task = import_task("metadata-only-first", root.to_str().unwrap(), first_now);
    insert_test_import_task(&store, &first_task, &ImportOptions::default());
    import_root_with_options(
        &data_dir,
        &store,
        &first_task,
        &root,
        first_now,
        ImportOptions::default(),
    )
    .unwrap();
    let first_head = ready_search_head(&store);
    let first_document = store.visible_documents().unwrap().remove(0);
    let first_version_count = store
        .resume_versions_for_document(&first_document.id)
        .unwrap()
        .len();
    let changed_mtime = fs::metadata(&source)
        .unwrap()
        .modified()
        .unwrap()
        .checked_add(Duration::from_secs(5))
        .unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(changed_mtime))
        .unwrap();

    let second_now = UnixTimestamp::from_unix_seconds(1_700_000_197);
    let second_task = import_task("metadata-only-second", root.to_str().unwrap(), second_now);
    insert_test_import_task(&store, &second_task, &ImportOptions::default());
    import_root_with_options(
        &data_dir,
        &store,
        &second_task,
        &root,
        second_now,
        ImportOptions::default(),
    )
    .unwrap();
    let second_head = ready_search_head(&store);
    let second_document = store.visible_documents().unwrap().remove(0);

    assert_eq!(second_document.id, first_document.id);
    assert_ne!(second_document.mtime, first_document.mtime);
    assert_eq!(
        store
            .resume_versions_for_document(&second_document.id)
            .unwrap()
            .len(),
        first_version_count
    );
    assert_ne!(second_head.generation, first_head.generation);
    assert_eq!(
        second_head.visible_epoch,
        first_head.visible_epoch.checked_add(1).unwrap()
    );
}

#[test]
fn unchanged_mixed_root_keeps_search_publication_stable() {
    let temp = TestDir::new("import-pipeline-zero-change-mixed-root");
    let data_dir = temp.path().join("data");
    let root = temp.path().join("mixed");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("resume.txt"),
        synthetic_resume_text("Synthetic Candidate", "Rust Search"),
    )
    .unwrap();
    fs::write(
        root.join("invoice.txt"),
        "INVOICE\nInvoice number 7\nSubtotal 10\nPayment terms net 30",
    )
    .unwrap();

    let embed_calls = Arc::new(AtomicUsize::new(0));
    let options = ImportOptions {
        parse_workers: ImportParseWorkers::new(1),
        search_vectorization: SearchPublicationVectorization::enabled(Arc::new(
            CountingPublicationVectorizer {
                calls: Arc::clone(&embed_calls),
            },
        )),
        ..ImportOptions::default()
    };
    let store = create_test_store(&data_dir);
    let first_now = UnixTimestamp::from_unix_seconds(1_700_000_196);
    let first_task = import_task("zero-change-mixed-first", root.to_str().unwrap(), first_now);
    let contract = insert_test_import_task(&store, &first_task, &options);
    let first_summary = import_root_with_options(
        &data_dir,
        &store,
        &first_task,
        &root,
        first_now,
        options.clone(),
    )
    .unwrap();
    let first_head = ready_search_head(&store);
    let first_documents = store.visible_documents().unwrap();
    let first_classification_counts = store
        .classification_counts_for_processing_contract(&contract)
        .unwrap();
    let first_source_counts = (
        first_summary.files_discovered,
        first_summary.searchable_documents,
        first_summary.ocr_required_documents,
        first_summary.failed_documents,
        first_summary.deleted_documents,
    );
    let first_ready_publications = store.recent_ready_search_publications(8).unwrap();
    let first_embed_calls = embed_calls.load(Ordering::SeqCst);

    assert_eq!(first_summary.files_discovered, 2);
    assert_eq!(first_summary.searchable_documents, 1);
    assert_eq!(first_documents.len(), 2);
    assert_eq!(first_classification_counts.resume_candidate, 1);
    assert_eq!(
        first_classification_counts.non_resume + first_classification_counts.needs_review,
        1
    );
    assert!(first_embed_calls > 0);

    let second_now = UnixTimestamp::from_unix_seconds(1_700_000_197);
    let second_task = import_task(
        "zero-change-mixed-second",
        root.to_str().unwrap(),
        second_now,
    );
    insert_test_import_task(&store, &second_task, &options);
    let second_summary =
        import_root_with_options(&data_dir, &store, &second_task, &root, second_now, options)
            .unwrap();
    let second_head = ready_search_head(&store);
    let second_documents = store.visible_documents().unwrap();
    let second_classification_counts = store
        .classification_counts_for_processing_contract(&contract)
        .unwrap();
    let second_source_counts = (
        second_summary.files_discovered,
        second_summary.searchable_documents,
        second_summary.ocr_required_documents,
        second_summary.failed_documents,
        second_summary.deleted_documents,
    );

    assert_eq!(second_summary.files_discovered, 2);
    assert_eq!(second_summary.searchable_documents, 1);
    assert_eq!(second_summary.ocr_required_documents, 0);
    assert_eq!(second_summary.ocr_jobs_queued, 0);
    assert_eq!(second_summary.failed_documents, 0);
    assert_eq!(second_summary.deleted_documents, 0);
    assert_eq!(second_head.generation, first_head.generation);
    assert_eq!(second_head.visible_epoch, first_head.visible_epoch);
    assert_eq!(
        store.recent_ready_search_publications(8).unwrap(),
        first_ready_publications
    );
    assert_eq!(embed_calls.load(Ordering::SeqCst), first_embed_calls);
    assert_eq!(second_documents, first_documents);
    assert_eq!(second_classification_counts, first_classification_counts);
    assert_eq!(second_source_counts, first_source_counts);
}
