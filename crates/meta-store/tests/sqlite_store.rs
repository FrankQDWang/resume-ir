use meta_store::{DocumentRecord, IngestJobStatus, MetaStore, ResumeVersionRecord, RetryableJob};

#[test]
fn migrations_are_idempotent_and_create_schema_v1_tables() {
    let store = MetaStore::open_in_memory().expect("open in-memory store");

    store.apply_migrations().expect("first migration");
    store.apply_migrations().expect("second migration");

    assert_eq!(store.schema_version().expect("schema version"), 1);
    assert!(store.table_exists("document").expect("document table"));
    assert!(
        store
            .table_exists("resume_version")
            .expect("resume_version table")
    );
    assert!(store.table_exists("ingest_job").expect("ingest_job table"));
    assert!(
        store
            .table_exists("index_state")
            .expect("index_state table")
    );
}

#[test]
fn deleted_documents_are_hidden_from_default_queries() {
    let store = MetaStore::open_in_memory().expect("open in-memory store");
    store.apply_migrations().expect("migrate");

    store
        .upsert_document(&DocumentRecord {
            doc_id: "doc_visible".to_owned(),
            source_uri: "file:///visible.pdf".to_owned(),
            normalized_path: "/fixtures/visible.pdf".to_owned(),
            file_name: "visible.pdf".to_owned(),
            extension: "pdf".to_owned(),
            byte_size: 42,
            mtime_unix_ms: 1_700_000_000_000,
            is_deleted: false,
        })
        .expect("insert visible doc");
    store
        .upsert_document(&DocumentRecord {
            doc_id: "doc_deleted".to_owned(),
            source_uri: "file:///deleted.pdf".to_owned(),
            normalized_path: "/fixtures/deleted.pdf".to_owned(),
            file_name: "deleted.pdf".to_owned(),
            extension: "pdf".to_owned(),
            byte_size: 42,
            mtime_unix_ms: 1_700_000_000_000,
            is_deleted: true,
        })
        .expect("insert deleted doc");

    let visible = store.list_visible_documents().expect("visible docs");

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].doc_id, "doc_visible");
}

#[test]
fn retryable_job_query_recovers_interrupted_work() {
    let store = MetaStore::open_in_memory().expect("open in-memory store");
    store.apply_migrations().expect("migrate");
    store
        .upsert_document(&DocumentRecord {
            doc_id: "doc_jobs".to_owned(),
            source_uri: "file:///jobs.pdf".to_owned(),
            normalized_path: "/fixtures/jobs.pdf".to_owned(),
            file_name: "jobs.pdf".to_owned(),
            extension: "pdf".to_owned(),
            byte_size: 42,
            mtime_unix_ms: 1_700_000_000_000,
            is_deleted: false,
        })
        .expect("insert doc");

    let queued = store
        .create_ingest_job("doc_jobs", 3)
        .expect("create queued job");
    let running = store
        .create_ingest_job("doc_jobs", 3)
        .expect("create running job");
    let permanent = store
        .create_ingest_job("doc_jobs", 3)
        .expect("create permanent job");

    store
        .update_job_status(&running, IngestJobStatus::Running)
        .expect("mark running");
    store
        .update_job_status(&permanent, IngestJobStatus::FailedPermanent)
        .expect("mark permanent");

    let retryable = store.list_retryable_jobs(10).expect("retryable jobs");
    let retryable_ids: Vec<&str> = retryable.iter().map(RetryableJob::job_id).collect();

    assert_eq!(retryable_ids, vec![queued.as_str(), running.as_str()]);
}

#[test]
fn resume_versions_can_be_recorded_for_documents() {
    let store = MetaStore::open_in_memory().expect("open in-memory store");
    store.apply_migrations().expect("migrate");
    store
        .upsert_document(&DocumentRecord {
            doc_id: "doc_versioned".to_owned(),
            source_uri: "file:///versioned.docx".to_owned(),
            normalized_path: "/fixtures/versioned.docx".to_owned(),
            file_name: "versioned.docx".to_owned(),
            extension: "docx".to_owned(),
            byte_size: 128,
            mtime_unix_ms: 1_700_000_000_000,
            is_deleted: false,
        })
        .expect("insert doc");

    store
        .upsert_resume_version(&ResumeVersionRecord {
            version_id: "ver_1".to_owned(),
            doc_id: "doc_versioned".to_owned(),
            parse_version: "parser-v1".to_owned(),
            schema_version: "schema-v1".to_owned(),
            visibility: "searchable".to_owned(),
        })
        .expect("insert version");

    assert_eq!(
        store
            .count_resume_versions("doc_versioned")
            .expect("version count"),
        1
    );
}
