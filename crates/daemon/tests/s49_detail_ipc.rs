use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use index_fulltext::{publish_snapshot, IndexDocument};
use index_vector::{VectorModelContract, VectorSnapshotStore};
use meta_store::{
    ActiveSearchProjection, ClassificationStatus, ContentDigest, DataDirectoryOwnerAcquisition,
    DataDirectoryOwnerLease, Document, DocumentId, DocumentStatus, EntityMention, EntityMentionId,
    EntityType, FileExtension, FullTextSnapshotDescriptor, IdentityInsertOutcome,
    MigrationRebuildPublicationAttemptAcquire, OwnedMetaStore, ProjectedDocumentSnapshot,
    ReasonCode, ResumeVersion, ResumeVersionClassification, ResumeVersionId, ReviewDisposition,
    SearchProjectionDigest, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationOutcome, SearchPublicationValidation, SearchSelection, SourceRevision,
    TerminalDocumentUpdate, UnixTimestamp, VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};
use process_containment::ContainedChild;
use tempfile::TempDir;

mod support;

const IPC_ENDPOINT_TIMEOUT: Duration = Duration::from_secs(30);
const HYDRATE_PAGE_BYTES: usize = 32 * 1024;
const MAX_BODY_PAGE_BYTES: usize = 32 * 1024;
const DETAIL_FIELD_LIMIT: usize = 256;

#[test]
fn detail_and_hydrate_read_one_exact_selection_across_unrelated_publications() {
    let fixture = Fixture::create("current");
    let expected_body = fixture.current_body.clone();
    let mut daemon = Daemon::start(&fixture.data_dir);
    let token = daemon.token.clone();

    let detail = daemon.post(
        "/details",
        Some(&token),
        detail_request("detail-current", &fixture.current_selection),
    );
    assert_status(&detail, "HTTP/1.1 200 OK");
    assert_private_values_absent(&detail, &daemon.token, &fixture.data_dir);
    let payload = response_json(&detail);
    assert_eq!(payload["schema_version"], "resume-ir.detail-response.v3");
    assert_eq!(payload["request_id"], "detail-current");
    assert_selection(&payload["selection"], &fixture.current_selection);
    assert_eq!(payload["document"]["source_byte_size"], expected_body.len());
    assert_eq!(payload["document"]["parse_version"], "parser-v1");
    assert_eq!(payload["document"]["schema_version"], "schema-v27");
    assert_eq!(payload["document"]["field_limit"], DETAIL_FIELD_LIMIT);
    assert_eq!(payload["document"]["fields_truncated"], false);
    assert_eq!(
        payload["document"]["fields"].as_array().unwrap().len(),
        DETAIL_FIELD_LIMIT
    );
    assert!(payload["document"]["snippet"]
        .as_str()
        .unwrap()
        .contains("Java"));
    assert!(payload["document"].get("visibility").is_none());
    assert!(payload["document"].get("doc_id").is_none());
    assert!(payload["document"].get("version_id").is_none());

    let mut offset = 0;
    let mut hydrated = String::new();
    loop {
        let response = daemon.post(
            "/details/hydrate",
            Some(&token),
            hydrate_request(
                &format!("hydrate-{offset}"),
                &fixture.current_selection,
                offset,
                HYDRATE_PAGE_BYTES,
            ),
        );
        assert_status(&response, "HTTP/1.1 200 OK");
        let payload = response_json(&response);
        assert_eq!(
            payload["schema_version"],
            "resume-ir.detail-hydrate-response.v3"
        );
        assert_eq!(payload["request_id"], format!("hydrate-{offset}"));
        assert_selection(&payload["selection"], &fixture.current_selection);
        assert!(payload["document"].get("display_path").is_none());
        let page = &payload["document"]["body_page"];
        assert_eq!(page["offset_bytes"], offset);
        assert_eq!(page["total_bytes"], expected_body.len());
        hydrated.push_str(page["text"].as_str().unwrap());
        let next = page["next_offset_bytes"].as_u64().unwrap() as usize;
        if page["complete"] == true {
            break;
        }
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(hydrated, expected_body);
    daemon.wait_success();
}

#[test]
fn hydrate_never_mixes_pages_after_the_selected_document_is_republished() {
    let fixture = Fixture::create("hydrate-switch");
    let mut daemon = Daemon::start(&fixture.data_dir);
    let token = daemon.token.clone();

    let first = daemon.post(
        "/details/hydrate",
        Some(&token),
        hydrate_request("hydrate-before-switch", &fixture.current_selection, 0, 4),
    );
    assert_status(&first, "HTTP/1.1 200 OK");
    let first_payload = response_json(&first);
    assert_eq!(first_payload["document"]["body_page"]["text"], "A简");
    let next_offset = first_payload["document"]["body_page"]["next_offset_bytes"]
        .as_u64()
        .unwrap() as usize;
    daemon.wait_success();

    fixture.replace_current_version();
    let mut daemon = Daemon::start(&fixture.data_dir);
    let token = daemon.token.clone();
    let interrupted = daemon.post(
        "/details/hydrate",
        Some(&token),
        hydrate_request(
            "hydrate-after-switch",
            &fixture.current_selection,
            next_offset,
            HYDRATE_PAGE_BYTES,
        ),
    );
    assert_error(
        &interrupted,
        "HTTP/1.1 409 Conflict",
        "STALE_SELECTION",
        Some("hydrate-after-switch"),
    );
    assert!(!interrupted.contains("REPLACEMENT_BODY_MUST_NOT_MIX_WITH_OLD_PAGE"));

    daemon.wait_success();
}

#[test]
fn detail_distinguishes_stale_from_unpublished_or_invalid_selections() {
    let fixture = Fixture::create("selection-errors");
    let mut daemon = Daemon::start(&fixture.data_dir);
    let token = daemon.token.clone();

    let stale = daemon.post(
        "/details",
        Some(&token),
        detail_request("detail-stale", &fixture.stale_selection),
    );
    assert_status(&stale, "HTTP/1.1 409 Conflict");
    let stale_payload = response_json(&stale);
    assert_eq!(stale_payload["schema_version"], "resume-ir.error.v2");
    assert_eq!(stale_payload["request_id"], "detail-stale");
    assert_eq!(stale_payload["error"]["code"], "STALE_SELECTION");
    assert_eq!(stale_payload["error"]["action"], "refresh_search");
    assert!(!stale.contains(fixture.stale_selection.document_id.as_str()));
    assert!(!stale.contains(fixture.stale_selection.resume_version_id.as_str()));

    let unpublished = daemon.post(
        "/details",
        Some(&token),
        detail_request("detail-unpublished", &fixture.unpublished_selection),
    );
    assert_not_found_without_selection(
        &unpublished,
        "detail-unpublished",
        &fixture.unpublished_selection,
    );

    let mismatched = SearchSelection {
        document_id: fixture.current_selection.document_id.clone(),
        resume_version_id: fixture.unpublished_selection.resume_version_id.clone(),
        visible_epoch: fixture.current_selection.visible_epoch,
    };
    let invalid_pair = daemon.post(
        "/details",
        Some(&token),
        detail_request("detail-invalid-pair", &mismatched),
    );
    assert_not_found_without_selection(&invalid_pair, "detail-invalid-pair", &mismatched);

    let missing = SearchSelection {
        document_id: DocumentId::from_non_secret_parts(&["s807", "missing-document"]),
        resume_version_id: ResumeVersionId::from_non_secret_parts(&["s807", "missing-version"]),
        visible_epoch: 1,
    };
    let missing_response = daemon.post(
        "/details/hydrate",
        Some(&token),
        hydrate_request("hydrate-missing", &missing, 0, HYDRATE_PAGE_BYTES),
    );
    assert_not_found_without_selection(&missing_response, "hydrate-missing", &missing);

    daemon.wait_success();
}

#[test]
fn detail_contract_rejects_legacy_shape_unbounded_ids_and_oversized_pages() {
    let fixture = Fixture::create("contract-errors");
    let mut daemon = Daemon::start(&fixture.data_dir);
    let token = daemon.token.clone();

    let unauthorized = daemon.post(
        "/details",
        None,
        detail_request("unauthorized", &fixture.current_selection),
    );
    assert_error(
        &unauthorized,
        "HTTP/1.1 401 Unauthorized",
        "UNAUTHORIZED",
        None,
    );
    assert!(!unauthorized.contains("unauthorized"));
    assert!(!unauthorized.contains(fixture.current_selection.document_id.as_str()));

    let mut wrong_schema_request = detail_request("wrong-schema", &fixture.current_selection);
    wrong_schema_request["schema_version"] = serde_json::json!("unexpected-schema");
    let wrong_schema = daemon.post("/details", Some(&token), wrong_schema_request);
    assert_error(
        &wrong_schema,
        "HTTP/1.1 400 Bad Request",
        "BAD_REQUEST",
        Some("wrong-schema"),
    );

    let unbounded = daemon.post(
        "/details",
        Some(&token),
        detail_request(&"x".repeat(129), &fixture.current_selection),
    );
    assert_error(&unbounded, "HTTP/1.1 400 Bad Request", "BAD_REQUEST", None);

    let oversized = daemon.post(
        "/details/hydrate",
        Some(&token),
        hydrate_request(
            "hydrate-oversized",
            &fixture.current_selection,
            0,
            MAX_BODY_PAGE_BYTES + 1,
        ),
    );
    assert_error(
        &oversized,
        "HTTP/1.1 413 Payload Too Large",
        "RESPONSE_TOO_LARGE",
        Some("hydrate-oversized"),
    );

    let invalid_boundary = daemon.post(
        "/details/hydrate",
        Some(&token),
        hydrate_request(
            "hydrate-invalid-boundary",
            &fixture.current_selection,
            2,
            HYDRATE_PAGE_BYTES,
        ),
    );
    assert_error(
        &invalid_boundary,
        "HTTP/1.1 400 Bad Request",
        "BAD_REQUEST",
        Some("hydrate-invalid-boundary"),
    );

    daemon.wait_success();
}

fn detail_request(request_id: &str, selection: &SearchSelection) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "resume-ir.detail-request.v3",
        "request_id": request_id,
        "selection": selection_json(selection),
    })
}

fn hydrate_request(
    request_id: &str,
    selection: &SearchSelection,
    offset: usize,
    limit: usize,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "resume-ir.detail-hydrate-request.v3",
        "request_id": request_id,
        "selection": selection_json(selection),
        "body_offset_bytes": offset,
        "body_limit_bytes": limit,
    })
}

fn selection_json(selection: &SearchSelection) -> serde_json::Value {
    serde_json::json!({
        "doc_id": selection.document_id.as_str(),
        "version_id": selection.resume_version_id.as_str(),
        "visible_epoch": selection.visible_epoch,
    })
}

fn assert_selection(actual: &serde_json::Value, expected: &SearchSelection) {
    assert_eq!(actual, &selection_json(expected));
}

fn assert_not_found_without_selection(
    response: &str,
    request_id: &str,
    selection: &SearchSelection,
) {
    assert_error(
        response,
        "HTTP/1.1 404 Not Found",
        "NOT_FOUND",
        Some(request_id),
    );
    assert!(!response.contains(selection.document_id.as_str()));
    assert!(!response.contains(selection.resume_version_id.as_str()));
}

fn assert_error(response: &str, status: &str, code: &str, request_id: Option<&str>) {
    assert_status(response, status);
    let payload = response_json(response);
    assert_eq!(payload["schema_version"], "resume-ir.error.v2");
    assert_eq!(payload["status"], "error");
    assert_eq!(payload["error"]["code"], code);
    assert_eq!(
        payload.get("request_id").and_then(|value| value.as_str()),
        request_id
    );
}

fn assert_status(response: &str, status: &str) {
    assert!(response.contains(status), "response:\n{response}");
}

fn assert_private_values_absent(response: &str, token: &str, data_dir: &Path) {
    assert!(!response.contains(token));
    assert!(!response.contains(path_str(data_dir)));
    assert!(!response.contains("candidate@example.test"));
    assert!(!response.contains("155-555-0199"));
    assert!(!response.contains("PRIVATE_TRAILING_MARKER_SHOULD_NOT_APPEAR"));
    assert!(!response.contains("MUTABLE_STAGING_METADATA_MUST_NOT_APPEAR"));
}

fn response_json(response: &str) -> serde_json::Value {
    serde_json::from_str(response.split("\r\n\r\n").nth(1).unwrap_or_default()).unwrap()
}

struct Fixture {
    _data_dir_guard: TempDir,
    data_dir: PathBuf,
    current_selection: SearchSelection,
    stale_selection: SearchSelection,
    unpublished_selection: SearchSelection,
    current_body: String,
    active_generation: String,
    visible_epoch: u64,
    active_projections: Vec<ActiveSearchProjection>,
}

impl Fixture {
    fn create(label: &str) -> Self {
        let data_dir_guard = tempfile::Builder::new()
            .prefix(&format!("resume-ir-s49-{label}-"))
            .tempdir()
            .unwrap();
        let data_dir = data_dir_guard.path().to_path_buf();
        let owner = acquire_data_directory_owner(&data_dir);
        let store = owner.open_store().unwrap();

        let current_path = "/synthetic/local/resumes/candidate-detail.pdf".to_string();
        let current_body = format!(
            "A简历🙂 Java platform engineer candidate@example.test 155-555-0199 {current_path} led payment routing with Rust and Kubernetes. {} PRIVATE_TRAILING_MARKER_SHOULD_NOT_APPEAR",
            "skill evidence ".repeat(6_000)
        );
        let current = seed_version(
            &store,
            "current",
            &current_path,
            &current_body,
            EXTRA_DETAIL_FIELDS,
        );
        publish(
            &data_dir,
            &store,
            "detail-generation-1",
            None,
            0,
            std::slice::from_ref(&current.projection()),
        );
        let current_selection = current.selection(1);

        let unrelated = seed_version(
            &store,
            "unrelated",
            "/synthetic/local/resumes/unrelated.txt",
            "Unrelated synthetic resume",
            0,
        );
        publish(
            &data_dir,
            &store,
            "detail-generation-2",
            Some("detail-generation-1"),
            1,
            &[current.projection(), unrelated.projection()],
        );

        let stale = seed_version(
            &store,
            "swapped-old",
            "/synthetic/local/resumes/swapped.pdf",
            "OLD_VERSION_SHOULD_NOT_APPEAR",
            0,
        );
        publish(
            &data_dir,
            &store,
            "detail-generation-3",
            Some("detail-generation-2"),
            2,
            &[
                current.projection(),
                unrelated.projection(),
                stale.projection(),
            ],
        );
        let stale_selection = stale.selection(3);
        let replacement = seed_version_for_document(
            &store,
            &stale.document,
            "swapped-new",
            "NEW_VERSION_MUST_NOT_BE_RETURNED_FOR_OLD_SELECTION",
            0,
        );
        let active_projections = vec![
            current.projection(),
            unrelated.projection(),
            replacement.projection(),
        ];
        publish(
            &data_dir,
            &store,
            "detail-generation-4",
            Some("detail-generation-3"),
            3,
            &active_projections,
        );

        let unpublished = seed_version(
            &store,
            "unpublished",
            "/synthetic/local/resumes/unpublished.pdf",
            "UNPUBLISHED_BODY_SHOULD_NOT_APPEAR",
            0,
        );
        let unpublished_selection = unpublished.selection(4);

        let mut staging_current = current.document.clone();
        staging_current.normalized_path =
            "/synthetic/MUTABLE_STAGING_METADATA_MUST_NOT_APPEAR.pdf".to_string();
        staging_current.file_name = "MUTABLE_STAGING_METADATA_MUST_NOT_APPEAR.pdf".to_string();
        staging_current.byte_size = 1;
        staging_current.status = DocumentStatus::Searchable;
        store.upsert_document(&staging_current).unwrap();

        drop(store);
        Self {
            _data_dir_guard: data_dir_guard,
            data_dir,
            current_selection,
            stale_selection,
            unpublished_selection,
            current_body,
            active_generation: "detail-generation-4".to_string(),
            visible_epoch: 4,
            active_projections,
        }
    }

    fn replace_current_version(&self) {
        let owner = acquire_data_directory_owner(&self.data_dir);
        let store = owner.open_store().unwrap();
        let mut projections = self.active_projections.clone();
        let document = store
            .document_by_id(&self.current_selection.document_id)
            .unwrap()
            .unwrap();
        let replacement = seed_version_for_document(
            &store,
            &document,
            "current-replacement",
            "REPLACEMENT_BODY_MUST_NOT_MIX_WITH_OLD_PAGE",
            0,
        );
        let current = projections
            .iter_mut()
            .find(|projection| projection.document_id == self.current_selection.document_id)
            .unwrap();
        current.resume_version_id = replacement.version.id;
        publish(
            &self.data_dir,
            &store,
            "detail-generation-after-hydrate",
            Some(&self.active_generation),
            self.visible_epoch,
            &projections,
        );
    }
}

const EXTRA_DETAIL_FIELDS: usize = DETAIL_FIELD_LIMIT;

struct SeededVersion {
    document: Document,
    version: ResumeVersion,
}

impl SeededVersion {
    fn projection(&self) -> ActiveSearchProjection {
        ActiveSearchProjection {
            document_id: self.document.id.clone(),
            resume_version_id: self.version.id.clone(),
        }
    }

    fn selection(&self, visible_epoch: u64) -> SearchSelection {
        SearchSelection {
            document_id: self.document.id.clone(),
            resume_version_id: self.version.id.clone(),
            visible_epoch,
        }
    }
}

fn seed_version(
    store: &OwnedMetaStore,
    label: &str,
    path: &str,
    text: &str,
    mention_count: usize,
) -> SeededVersion {
    let now = UnixTimestamp::from_unix_seconds(1_800_049_000);
    let document = Document {
        id: DocumentId::from_non_secret_parts(&["s807", label]),
        source_uri: format!("file://{path}"),
        normalized_path: path.to_string(),
        file_name: format!("{label}-candidate@example.test.pdf"),
        extension: FileExtension::Pdf,
        byte_size: text.len() as u64,
        mtime: now,
        content_hash: None,
        text_hash: None,
        is_deleted: false,
        created_at: now,
        updated_at: now,
        status: DocumentStatus::Searchable,
    };
    store.upsert_document(&document).unwrap();
    seed_version_for_document(store, &document, label, text, mention_count)
}

fn seed_version_for_document(
    store: &OwnedMetaStore,
    document: &Document,
    label: &str,
    text: &str,
    mention_count: usize,
) -> SeededVersion {
    let revision = SourceRevision::for_content(
        document.id.clone(),
        ContentDigest::from_bytes(format!("source-{label}").as_bytes()),
        text.len() as u64,
    );
    let normalized_text_hash = ContentDigest::from_bytes(text.as_bytes());
    let mut staged_document = document.clone();
    staged_document.byte_size = revision.byte_size;
    staged_document.content_hash = Some(revision.content_hash.as_str().to_string());
    staged_document.text_hash = Some(normalized_text_hash.as_str().to_string());
    staged_document.status = DocumentStatus::FieldsExtracted;
    store.upsert_document(&staged_document).unwrap();
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &revision.id,
            &normalized_text_hash,
            "parser-v1",
            "schema-v27",
        ),
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: "parser-v1".to_string(),
        schema_version: "schema-v27".to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some(text.to_string()),
        clean_text: Some(text.to_string()),
        quality_score: Some(0.9),
    };
    assert!(matches!(
        store.insert_source_revision(&revision).unwrap(),
        IdentityInsertOutcome::Inserted | IdentityInsertOutcome::AlreadyPresent
    ));
    assert!(matches!(
        store.insert_resume_version(&version).unwrap(),
        IdentityInsertOutcome::Inserted | IdentityInsertOutcome::AlreadyPresent
    ));
    assert!(matches!(
        store
            .insert_resume_version_classification(&ResumeVersionClassification {
                resume_version_id: version.id.clone(),
                status: ClassificationStatus::ResumeCandidate,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
                classified_at: UnixTimestamp::from_unix_seconds(1_800_049_001),
                review_disposition: ReviewDisposition::NotRequired,
            })
            .unwrap(),
        IdentityInsertOutcome::Inserted | IdentityInsertOutcome::AlreadyPresent
    ));
    let mentions = (0..mention_count)
        .map(|index| entity_mention(&version.id, index))
        .collect::<Vec<_>>();
    store
        .insert_entity_mentions(&version.id, &mentions)
        .unwrap();
    SeededVersion {
        document: staged_document,
        version,
    }
}

fn entity_mention(version_id: &ResumeVersionId, index: usize) -> EntityMention {
    let value = if index == 0 {
        "candidate@example.test".to_string()
    } else if index == 1 {
        "155-555-0199".to_string()
    } else {
        format!("SyntheticSkill{index:03}")
    };
    EntityMention {
        id: EntityMentionId::from_non_secret_parts(&[
            "s807",
            version_id.as_str(),
            &format!("field-{index:03}"),
        ]),
        resume_version_id: version_id.clone(),
        section_id: None,
        entity_type: if index == 0 {
            EntityType::Email
        } else if index == 1 {
            EntityType::Phone
        } else {
            EntityType::Skill
        },
        raw_value: value.clone(),
        normalized_value: Some(value),
        span_start: Some(0),
        span_end: Some(1),
        confidence: 0.9,
        extractor: "s807-test".to_string(),
    }
}

fn publish(
    data_dir: &Path,
    store: &OwnedMetaStore,
    generation: &str,
    expected_generation: Option<&str>,
    expected_epoch: u64,
    projections: &[ActiveSearchProjection],
) {
    let now = UnixTimestamp::from_unix_seconds(1_800_050_000 + expected_epoch as i64);
    let projection_digest =
        SearchProjectionDigest::from_pairs(projections.iter().map(|projection| {
            (
                projection.document_id.as_str(),
                projection.resume_version_id.as_str(),
            )
        }))
        .unwrap();
    let publication = SearchPublicationDraft {
        generation: generation.to_string(),
        base_generation: expected_generation.map(str::to_string),
        expected_visible_epoch: expected_epoch,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        projection_digest: projection_digest.clone(),
        now,
    };
    let migration_barrier = expected_generation.is_none().then(|| {
        let contract = support::activate_default_processing_contract(store, now);
        store
            .acquire_migration_rebuild_barrier_token(contract.id())
            .unwrap()
            .expect("initial publication requires a closed migration rebuild barrier")
    });
    let mut publication_session = store.wait_for_search_publication_session().unwrap();
    if let Some(barrier) = migration_barrier.as_ref() {
        assert!(matches!(
            publication_session
                .acquire_migration_rebuild_publication_attempt(barrier, now)
                .unwrap(),
            MigrationRebuildPublicationAttemptAcquire::Started(_)
        ));
    }
    assert_eq!(
        publication_session
            .begin_search_publication(&publication)
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let index_documents = projections
        .iter()
        .map(|projection| {
            let document = store
                .document_by_id(&projection.document_id)
                .unwrap()
                .unwrap();
            let version = store
                .resume_version_by_id(&projection.resume_version_id)
                .unwrap()
                .unwrap();
            IndexDocument {
                doc_id: projection.document_id.to_string(),
                resume_version_id: projection.resume_version_id.to_string(),
                file_name: document.file_name,
                clean_text: version.clean_text.unwrap(),
                sections: Vec::new(),
            }
        })
        .collect::<Vec<_>>();
    let fulltext_artifact =
        publish_snapshot(&data_dir.join("search-index"), generation, index_documents).unwrap();
    let vector_artifact =
        VectorSnapshotStore::new(data_dir.join("vector-index"), VectorModelContract::Disabled)
            .unwrap()
            .publish_generation(generation, projections.iter().cloned(), Vec::new())
            .unwrap();
    let fulltext = FullTextSnapshotDescriptor::new(
        generation.to_string(),
        u64::try_from(fulltext_artifact.document_count()).unwrap(),
        fulltext_artifact.projection_digest().clone(),
        fulltext_artifact.logical_content_digest().clone(),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        generation.to_string(),
        u64::try_from(vector_artifact.projection_count()).unwrap(),
        vector_artifact.projection_digest().clone(),
        vector_artifact.coverage_digest().clone(),
        vector_artifact.logical_content_digest().clone(),
    );
    publication_session
        .validate_search_publication(&SearchPublicationValidation {
            generation,
            fulltext: &fulltext,
            vector: &vector,
            now,
        })
        .unwrap();
    let terminal_documents = projections
        .iter()
        .filter_map(|projection| {
            let document = store
                .document_by_id(&projection.document_id)
                .unwrap()
                .unwrap();
            (document.status != DocumentStatus::Searchable).then(|| {
                let version = store
                    .resume_version_by_id(&projection.resume_version_id)
                    .unwrap()
                    .unwrap();
                let revision = store
                    .source_revision_by_id(&version.source_revision_id)
                    .unwrap()
                    .unwrap();
                TerminalDocumentUpdate {
                    document_id: projection.document_id.clone(),
                    expected_status: document.status,
                    expected_is_deleted: document.is_deleted,
                    expected_content_hash: revision.content_hash,
                    terminal_status: DocumentStatus::Searchable,
                    terminal_is_deleted: false,
                }
            })
        })
        .collect::<Vec<_>>();
    let projected_documents = projections
        .iter()
        .map(|projection| {
            let mut document = store
                .document_by_id(&projection.document_id)
                .unwrap()
                .unwrap();
            if let Some(terminal) = terminal_documents
                .iter()
                .find(|terminal| terminal.document_id == projection.document_id)
            {
                document.status = terminal.terminal_status;
                document.is_deleted = terminal.terminal_is_deleted;
                document.updated_at = now;
            }
            match store
                .active_search_projection_for_document(&projection.document_id)
                .unwrap()
            {
                Some(active) if active == *projection => {
                    let active_document =
                        store.active_search_document(projection).unwrap().unwrap();
                    if active_document == document {
                        ProjectedDocumentSnapshot::RetainedUnchanged {
                            projection: projection.clone(),
                        }
                    } else {
                        ProjectedDocumentSnapshot::MetadataChanged {
                            projection: projection.clone(),
                            document,
                        }
                    }
                }
                Some(_) | None => ProjectedDocumentSnapshot::Replacement {
                    projection: projection.clone(),
                    document,
                },
            }
        })
        .collect::<Vec<_>>();
    let commit = SearchPublicationCommit {
        generation,
        terminal_documents: &terminal_documents,
        projections,
        projected_documents: &projected_documents,
        vector_coverage: &[],
        now,
    };
    let outcome = match migration_barrier.as_ref() {
        Some(barrier) => publication_session
            .commit_migration_rebuild_search_publication(&commit, barrier)
            .unwrap(),
        None => publication_session
            .commit_search_publication(&commit)
            .unwrap(),
    };
    assert_eq!(outcome, SearchPublicationOutcome::Applied);
}

fn acquire_data_directory_owner(data_dir: &Path) -> DataDirectoryOwnerLease {
    match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data directory is owned"),
    }
}

struct Daemon {
    child: Option<ContainedChild>,
    parent_lifecycle: Option<ChildStdin>,
    stderr: Option<ChildStderr>,
    endpoint: String,
    token: String,
}

impl Daemon {
    fn start(data_dir: &Path) -> Self {
        let launch_id = random_launch_id();
        let mut command = Command::new(env!("CARGO_BIN_EXE_resume-daemon"));
        command
            .args([
                "--data-dir",
                path_str(data_dir),
                "run",
                "--foreground",
                "--parent-lifecycle-stdin",
                "--launch-id",
                &launch_id,
                "--ipc-listen",
                "127.0.0.1:0",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = ContainedChild::spawn(&mut command).expect("start contained resume daemon");
        let parent_lifecycle = child.take_stdin().expect("daemon parent lifecycle");
        let stdout = child.take_stdout().expect("daemon stdout");
        let mut stderr = child.take_stderr().expect("daemon stderr");
        let mut stdout = BufReader::new(stdout);
        let endpoint = read_ipc_endpoint(&mut child, &mut stderr, &mut stdout);
        let token = read_ipc_auth_token(data_dir);
        Self {
            child: Some(child),
            parent_lifecycle: Some(parent_lifecycle),
            stderr: Some(stderr),
            endpoint,
            token,
        }
    }

    fn post(&mut self, path: &str, token: Option<&str>, payload: serde_json::Value) -> String {
        http_post_command(&self.endpoint, path, token, payload)
    }

    fn wait_success(mut self) {
        assert!(
            self.child
                .as_mut()
                .unwrap()
                .try_wait()
                .expect("poll daemon before parent shutdown")
                .is_none(),
            "daemon exited before its parent lifecycle closed"
        );
        drop(self.parent_lifecycle.take());
        let status = self.child.as_mut().unwrap().wait().expect("wait daemon");
        let mut stderr = String::new();
        self.stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .expect("read daemon stderr");
        assert!(status.success(), "stderr:\n{stderr}");
        assert!(stderr.is_empty());
        self.child.take();
    }
}

fn random_launch_id() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).expect("generate daemon test launch identifier");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn http_post_command(
    endpoint: &str,
    path: &str,
    token: Option<&str>,
    payload: serde_json::Value,
) -> String {
    let rest = endpoint.strip_prefix("http://").unwrap();
    let (addr, _) = rest.split_once('/').unwrap();
    let body = payload.to_string();
    let authorization = token
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n{authorization}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(addr).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    read_http_response(&mut stream).unwrap()
}

fn read_http_response(reader: &mut impl Read) -> io::Result<String> {
    const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

    let mut response = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(frame_len) = complete_http_frame_len(&response)? {
            if response.len() != frame_len {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "HTTP response contains bytes after the declared body",
                ));
            }
            return String::from_utf8(response).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "HTTP response is not UTF-8")
            });
        }
        if response.len() == MAX_RESPONSE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP response exceeds the test client limit",
            ));
        }
        let remaining = MAX_RESPONSE_BYTES - response.len();
        let chunk_limit = remaining.min(chunk.len());
        let read = reader.read(&mut chunk[..chunk_limit])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "HTTP response ended before the declared body was complete",
            ));
        }
        response.extend_from_slice(&chunk[..read]);
    }
}

fn complete_http_frame_len(response: &[u8]) -> io::Result<Option<usize>> {
    let Some(header_offset) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Ok(None);
    };
    let header_len = header_offset + 4;
    let header = std::str::from_utf8(&response[..header_offset]).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP response header is not UTF-8",
        )
    })?;
    let content_length = header
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim())
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP response is missing Content-Length",
            )
        })?
        .parse::<usize>()
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP response has an invalid Content-Length",
            )
        })?;
    let frame_len = header_len.checked_add(content_length).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP response length overflows usize",
        )
    })?;
    Ok((response.len() >= frame_len).then_some(frame_len))
}

#[test]
fn http_response_reader_accepts_a_complete_frame_before_transport_reset() {
    struct CompleteFrameThenReset {
        frame: &'static [u8],
        offset: usize,
    }

    impl Read for CompleteFrameThenReset {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.offset == self.frame.len() {
                return Err(io::Error::from(io::ErrorKind::ConnectionReset));
            }
            let remaining = &self.frame[self.offset..];
            let copied = remaining.len().min(buffer.len());
            buffer[..copied].copy_from_slice(&remaining[..copied]);
            self.offset += copied;
            Ok(copied)
        }
    }

    let frame = b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    let mut reader = CompleteFrameThenReset { frame, offset: 0 };

    assert_eq!(
        read_http_response(&mut reader).unwrap(),
        String::from_utf8(frame.to_vec()).unwrap()
    );
}

#[test]
fn http_response_reader_rejects_a_partial_frame_before_transport_reset() {
    struct PartialFrameThenReset {
        delivered: bool,
    }

    impl Read for PartialFrameThenReset {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.delivered {
                return Err(io::Error::from(io::ErrorKind::ConnectionReset));
            }
            let partial = b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n{}";
            buffer[..partial.len()].copy_from_slice(partial);
            self.delivered = true;
            Ok(partial.len())
        }
    }

    assert_eq!(
        read_http_response(&mut PartialFrameThenReset { delivered: false })
            .unwrap_err()
            .kind(),
        io::ErrorKind::ConnectionReset
    );
}

fn read_ipc_endpoint(
    child: &mut ContainedChild,
    stderr: &mut ChildStderr,
    stdout: &mut BufReader<impl Read>,
) -> String {
    let deadline = Instant::now() + IPC_ENDPOINT_TIMEOUT;
    let mut line = String::new();
    let mut endpoint = None;
    while Instant::now() < deadline {
        line.clear();
        let bytes = stdout.read_line(&mut line).unwrap();
        if bytes == 0 {
            if let Ok(Some(status)) = child.try_wait() {
                let mut stderr_body = String::new();
                let _ = stderr.read_to_string(&mut stderr_body);
                panic!("daemon exited before endpoint: {status}\nstderr:\n{stderr_body}");
            }
            continue;
        }
        if let Some(value) = line.trim().strip_prefix("ipc status endpoint: ") {
            endpoint = Some(value.to_string());
        }
        if line.trim() == "resume-daemon foreground ready" {
            return endpoint.expect("ready line follows endpoint publication");
        }
    }
    child.terminate();
    panic!("daemon did not print ipc status endpoint");
}

fn read_ipc_auth_token(data_dir: &Path) -> String {
    let body = fs::read_to_string(data_dir.join("ipc.auth")).unwrap();
    let auth: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(auth["schema_version"], "resume-ir.daemon-auth.v3");
    assert_eq!(auth["launch_id"].as_str().map(str::len), Some(64));
    auth["token"].as_str().unwrap().to_string()
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}
