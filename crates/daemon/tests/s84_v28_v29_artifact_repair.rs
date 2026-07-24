use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use import_pipeline::{current_import_processing_contract, ImportOptions};
use index_fulltext::{publish_snapshot, IndexDocument};
use index_vector::{VectorModelContract, VectorSnapshotStore};
use meta_store::{
    migration_test_support::{seed_v28_legacy_artifact_repair_fixture, V28ArtifactRepairHead},
    ActiveSearchProjection, ClassificationStatus, ContentDigest, DataDirectoryOwnerAcquisition,
    DataDirectoryOwnerLease, Document, DocumentId, DocumentStatus, FileExtension,
    FullTextSnapshotDescriptor, ImmutableIngestStage, MetaStoreErrorClass,
    MigrationRebuildPublicationAttemptAcquire, ProjectedDocumentSnapshot, ReadMetaStore,
    ReasonCode, ResumeVersion, ResumeVersionClassification, ResumeVersionId, ReviewDisposition,
    SearchProjectionDigest, SearchPublicationCommit, SearchPublicationDraft,
    SearchPublicationOutcome, SearchPublicationValidation, SearchSelection,
    SearchSelectionResolution, SourceRevision, TerminalDocumentUpdate, UnixTimestamp,
    VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};
use process_containment::ContainedChild;
use serde_json::json;
use sha2::{Digest, Sha256};
use tempfile::tempdir;

const LAUNCH_ID: &str = "8484848484848484848484848484848484848484848484848484848484848484";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[test]
fn supervised_daemon_blocks_v28_without_changing_existing_bytes() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    seed_v28_legacy_artifact_repair_fixture(&data_dir, V28ArtifactRepairHead::Ready).unwrap();
    let protected_before = snapshot_existing_files(&data_dir);
    let open_error = ReadMetaStore::open_data_dir(&data_dir).unwrap_err();
    assert_eq!(
        open_error.class(),
        MetaStoreErrorClass::UnsupportedStoreSchema
    );

    let mut daemon = DesktopDaemon::spawn(&data_dir);
    let generation = wait_for_generation(&mut daemon.child, &data_dir);
    assert_eq!(generation.launch_id, LAUNCH_ID);

    let status = wait_for_core_state(&mut daemon.child, &generation, "blocked");
    assert_status_contract(&status.body);
    assert_eq!(status.body["process_state"], "ready");
    assert_eq!(status.body["status"], "blocked");
    assert_eq!(status.body["core"]["state"], "blocked");
    assert_eq!(status.body["core"]["reason"], "unsupported_store_schema");
    assert_eq!(status.body["indexed_documents"], serde_json::Value::Null);
    assert_eq!(status.body["visible_epoch"], serde_json::Value::Null);
    assert_eq!(status.body["error"]["code"], "SERVICE_BLOCKED");
    assert_eq!(status.body["error"]["action"], "repair_required");

    let search = request(
        &generation.search_endpoint,
        &generation.token,
        "POST",
        Some(json!({
            "schema_version": "resume-ir.ipc-request.v3",
            "request_id": "v28-must-not-open",
            "client_capability": "codex_validation",
            "deadline_ms": 1_000,
            "payload": {"query": "synthetic", "mode": "fulltext", "top_k": 1}
        })),
    );
    assert_eq!(search.status_code, 503, "{}", search.raw);
    assert_eq!(search.body["schema_version"], "resume-ir.error.v2");
    assert_eq!(search.body["error"]["code"], "SERVICE_BLOCKED");
    assert_eq!(search.body["error"]["action"], "repair_required");
    assert_eq!(search.body["error"]["capability"], serde_json::Value::Null);
    assert_eq!(search.body["error"]["reason"], "unsupported_store_schema");
    assert_eq!(
        object_keys(&search.body),
        BTreeSet::from(["schema_version", "status", "request_id", "error"])
    );
    assert_eq!(
        object_keys(&search.body["error"]),
        BTreeSet::from(["code", "action", "capability", "reason"])
    );

    let current = read_generation(&data_dir).unwrap();
    assert_eq!(current.launch_id, generation.launch_id);
    assert_eq!(current.instance_id, generation.instance_id);
    daemon.finish();

    assert_eq!(snapshot_existing_files(&data_dir), protected_before);
    assert!(!data_dir.join("ipc.endpoints.json").exists());
    assert!(!data_dir.join("ipc.auth").exists());
}

#[test]
fn supervised_daemon_preserves_exact_v29_business_head_epoch_and_artifact_digests() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let projection = seed_current_v29_publication(&data_dir);
    let before = preserved_v29_summary(&data_dir, &projection);
    assert_eq!(before.generation, "s84-current-v29-generation");
    assert_eq!(before.visible_epoch, 1);
    assert!(before.selection_is_current);
    assert_eq!(before.active_projection, projection);
    assert_eq!(before.selection.document_id, projection.document_id);
    assert_eq!(
        before.selection.resume_version_id,
        projection.resume_version_id
    );

    let mut daemon = DesktopDaemon::spawn(&data_dir);
    let generation = wait_for_generation(&mut daemon.child, &data_dir);
    let status = wait_for_core_state(&mut daemon.child, &generation, "ready");
    assert_status_contract(&status.body);
    assert_eq!(status.body["process_state"], "ready");
    assert_eq!(status.body["status"], "ok");
    assert_eq!(
        status.body["core"],
        json!({"state": "ready", "reason": null})
    );
    assert_eq!(status.body["visible_epoch"], before.visible_epoch);
    assert_eq!(
        status.body["capabilities"]["keyword_search"]["state"],
        "available"
    );
    assert_eq!(status.body["capabilities"]["detail"]["state"], "available");
    assert_eq!(
        status.body["capabilities"]["semantic_search"]["state"],
        "unavailable"
    );
    assert_eq!(
        status.body["capabilities"]["hybrid_search"]["state"],
        "degraded"
    );

    let search = request(
        &generation.search_endpoint,
        &generation.token,
        "POST",
        Some(json!({
            "schema_version": "resume-ir.ipc-request.v3",
            "request_id": "v29-preserved-search",
            "client_capability": "codex_validation",
            "deadline_ms": 5_000,
            "payload": {"query": "PreservationToken", "mode": "fulltext", "top_k": 1}
        })),
    );
    assert_eq!(search.status_code, 200, "{}", search.raw);
    assert_eq!(
        search.body["schema_version"],
        "resume-ir.search-response.v3"
    );
    assert_eq!(search.body["visible_epoch"], before.visible_epoch);
    assert_eq!(search.body["result_count"], 1);
    assert_eq!(
        search.body["results"][0]["selection"]["doc_id"],
        projection.document_id.as_str()
    );
    assert_eq!(
        search.body["results"][0]["selection"]["version_id"],
        projection.resume_version_id.as_str()
    );
    assert_eq!(
        search.body["results"][0]["selection"]["visible_epoch"],
        before.visible_epoch
    );
    daemon.finish();

    assert_eq!(preserved_v29_summary(&data_dir, &projection), before);
    assert!(!data_dir.join("ipc.endpoints.json").exists());
    assert!(!data_dir.join("ipc.auth").exists());
}

fn seed_current_v29_publication(data_dir: &Path) -> ActiveSearchProjection {
    const GENERATION: &str = "s84-current-v29-generation";
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let store = owner.open_store().unwrap();
    let now = UnixTimestamp::from_unix_seconds(1_800_084_000);
    let source = b"synthetic PreservationToken v29 resume";
    let mut document = Document {
        id: DocumentId::from_non_secret_parts(&["s84", "current-v29"]),
        source_uri: "synthetic://s84/current-v29.txt".to_string(),
        normalized_path: "synthetic/s84/current-v29.txt".to_string(),
        file_name: "current-v29.txt".to_string(),
        extension: FileExtension::Txt,
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
    let contract = current_import_processing_contract(&ImportOptions::default()).unwrap();
    let normalized_text = "synthetic PreservationToken v29 resume";
    let normalized_text_hash = ContentDigest::from_bytes(normalized_text.as_bytes());
    document.text_hash = Some(normalized_text_hash.as_str().to_string());
    let version = ResumeVersion {
        id: ResumeVersionId::from_content_identity(
            &document.id,
            &revision.id,
            &normalized_text_hash,
            contract.primary_parse_version(),
            contract.derived_schema_version(),
        ),
        document_id: document.id.clone(),
        source_revision_id: revision.id.clone(),
        normalized_text_hash,
        parse_version: contract.primary_parse_version().to_string(),
        schema_version: contract.derived_schema_version().to_string(),
        language_set: vec!["en".to_string()],
        page_count: Some(1),
        raw_text: Some(normalized_text.to_string()),
        clean_text: Some(normalized_text.to_string()),
        quality_score: Some(0.95),
    };
    let classification = ResumeVersionClassification {
        resume_version_id: version.id.clone(),
        status: ClassificationStatus::ResumeCandidate,
        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
        classified_at: now,
        review_disposition: ReviewDisposition::NotRequired,
    };
    store
        .stage_immutable_ingest(ImmutableIngestStage::ClassifiedResume {
            document: &document,
            source_revision: &revision,
            version: &version,
            classification: &classification,
            mentions: &[],
            email_hash: None,
            phone_hash: None,
        })
        .unwrap();
    store
        .activate_migration_rebuild_contract(&contract, now)
        .unwrap();
    let barrier = store
        .acquire_migration_rebuild_barrier_token(contract.id())
        .unwrap()
        .unwrap();
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_migration_rebuild_publication_attempt(&barrier, now)
            .unwrap(),
        MigrationRebuildPublicationAttemptAcquire::Started(_)
            | MigrationRebuildPublicationAttemptAcquire::InProgress
    ));
    let projection = ActiveSearchProjection {
        document_id: document.id.clone(),
        resume_version_id: version.id.clone(),
    };
    let digest = SearchProjectionDigest::from_pairs([(
        projection.document_id.as_str(),
        projection.resume_version_id.as_str(),
    )])
    .unwrap();
    assert_eq!(
        session
            .begin_search_publication(&SearchPublicationDraft {
                generation: GENERATION.to_string(),
                base_generation: None,
                expected_visible_epoch: 0,
                classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                projection_digest: digest.clone(),
                now,
            })
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    let fulltext_artifact = publish_snapshot(
        &data_dir.join("search-index"),
        GENERATION,
        [IndexDocument {
            doc_id: projection.document_id.to_string(),
            resume_version_id: projection.resume_version_id.to_string(),
            file_name: document.file_name.clone(),
            clean_text: normalized_text.to_string(),
            sections: Vec::new(),
        }],
    )
    .unwrap();
    let vector_artifact =
        VectorSnapshotStore::new(data_dir.join("vector-index"), VectorModelContract::Disabled)
            .unwrap()
            .publish_generation(GENERATION, [projection.clone()], Vec::new())
            .unwrap();
    let fulltext = FullTextSnapshotDescriptor::new(
        GENERATION.to_string(),
        1,
        fulltext_artifact.projection_digest().clone(),
        fulltext_artifact.logical_content_digest().clone(),
    );
    let vector = VectorSnapshotDescriptor::disabled(
        GENERATION.to_string(),
        1,
        vector_artifact.projection_digest().clone(),
        vector_artifact.coverage_digest().clone(),
        vector_artifact.logical_content_digest().clone(),
    );
    session
        .validate_search_publication(&SearchPublicationValidation {
            generation: GENERATION,
            fulltext: &fulltext,
            vector: &vector,
            now,
        })
        .unwrap();
    let terminal = TerminalDocumentUpdate {
        document_id: document.id.clone(),
        expected_status: DocumentStatus::FieldsExtracted,
        expected_is_deleted: false,
        expected_content_hash: revision.content_hash,
        terminal_status: DocumentStatus::Searchable,
        terminal_is_deleted: false,
    };
    document.status = DocumentStatus::Searchable;
    assert_eq!(
        session
            .commit_migration_rebuild_search_publication(
                &SearchPublicationCommit {
                    generation: GENERATION,
                    terminal_documents: &[terminal],
                    projections: std::slice::from_ref(&projection),
                    projected_documents: &[ProjectedDocumentSnapshot::Replacement {
                        projection: projection.clone(),
                        document,
                    }],
                    vector_coverage: &[],
                    now,
                },
                &barrier,
            )
            .unwrap(),
        SearchPublicationOutcome::Applied
    );
    drop(session);
    drop(store);
    drop(owner);
    projection
}

#[derive(Debug, PartialEq)]
struct PreservedV29Summary {
    document: Document,
    source_revision: SourceRevision,
    resume_version: ResumeVersion,
    classification: ResumeVersionClassification,
    active_projection: ActiveSearchProjection,
    active_document: Document,
    generation: String,
    visible_epoch: u64,
    selection: SearchSelection,
    selection_is_current: bool,
    projection_digest: SearchProjectionDigest,
    fulltext_artifact_digest: ContentDigest,
    vector_artifact_digest: ContentDigest,
}

fn preserved_v29_summary(
    data_dir: &Path,
    projection: &ActiveSearchProjection,
) -> PreservedV29Summary {
    let store = ReadMetaStore::open_data_dir(data_dir).unwrap();
    assert_eq!(store.schema_version().unwrap(), 29);
    let state = store.search_projection_state().unwrap();
    let publication = state.publication.as_deref().unwrap();
    let selection = SearchSelection {
        document_id: projection.document_id.clone(),
        resume_version_id: projection.resume_version_id.clone(),
        visible_epoch: state.visible_epoch,
    };
    let selection_is_current = store
        .with_search_metadata_snapshot(|snapshot| {
            Ok::<_, ()>(matches!(
                snapshot.resolve_search_selection(&selection).unwrap(),
                SearchSelectionResolution::Current { selection: resolved } if resolved == selection
            ))
        })
        .unwrap();
    let resume_version = store
        .resume_version_by_id(&projection.resume_version_id)
        .unwrap()
        .unwrap();
    PreservedV29Summary {
        document: store
            .document_by_id(&projection.document_id)
            .unwrap()
            .unwrap(),
        source_revision: store
            .source_revision_by_id(&resume_version.source_revision_id)
            .unwrap()
            .unwrap(),
        resume_version,
        classification: store
            .resume_version_classification(&projection.resume_version_id, CLASSIFIER_EPOCH)
            .unwrap()
            .unwrap(),
        active_projection: store
            .active_search_projection_for_document(&projection.document_id)
            .unwrap()
            .unwrap(),
        active_document: store.active_search_document(projection).unwrap().unwrap(),
        generation: state.generation.clone().unwrap(),
        visible_epoch: state.visible_epoch,
        selection,
        selection_is_current,
        projection_digest: publication.projection_digest.clone(),
        fulltext_artifact_digest: publication
            .fulltext
            .as_ref()
            .unwrap()
            .logical_content_digest()
            .clone(),
        vector_artifact_digest: publication
            .vector
            .as_ref()
            .unwrap()
            .logical_content_digest()
            .clone(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Generation {
    launch_id: String,
    instance_id: String,
    token: String,
    status_endpoint: String,
    search_endpoint: String,
}

fn wait_for_generation(child: &mut ContainedChild, data_dir: &Path) -> Generation {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Some(generation) = read_generation(data_dir) {
            return generation;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("daemon exited before publishing control plane: {status}");
        }
        assert!(Instant::now() < deadline, "control plane was not published");
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_core_state(
    child: &mut ContainedChild,
    generation: &Generation,
    expected_state: &str,
) -> Response {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let response = request(&generation.status_endpoint, &generation.token, "GET", None);
        assert_eq!(response.status_code, 200, "{}", response.raw);
        assert_eq!(response.body["schema_version"], "daemon.status.v3");
        assert_eq!(response.body["process_state"], "ready");
        let state = response.body["core"]["state"].as_str().unwrap();
        if state == expected_state {
            return response;
        }
        assert_eq!(
            state, "initializing",
            "unexpected core transition: {}",
            response.raw
        );
        assert_eq!(response.body["core"]["reason"], "metadata_initializing");
        if let Some(status) = child.try_wait().unwrap() {
            panic!("daemon exited before core became {expected_state}: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "core did not become {expected_state}"
        );
        thread::sleep(POLL_INTERVAL);
    }
}

fn assert_status_contract(body: &serde_json::Value) {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../apps/desktop/src-tauri/tests/fixtures/daemon-status-v3-ready.json"
    )))
    .unwrap();
    assert_eq!(object_keys(body), object_keys(&fixture));
    assert_eq!(
        object_keys(&body["core"]),
        BTreeSet::from(["state", "reason"])
    );
    assert_eq!(
        object_keys(&body["optional_runtimes"]),
        BTreeSet::from(["embedding", "ocr", "classifier"])
    );
    for runtime in ["embedding", "ocr", "classifier"] {
        assert_eq!(
            object_keys(&body["optional_runtimes"][runtime]),
            BTreeSet::from(["state", "reason"])
        );
    }
    assert_eq!(
        object_keys(&body["capabilities"]),
        BTreeSet::from([
            "keyword_search",
            "detail",
            "semantic_search",
            "hybrid_search",
            "text_import",
            "ocr_import",
            "index_publication",
        ])
    );
    for capability in body["capabilities"].as_object().unwrap().values() {
        assert_eq!(object_keys(capability), BTreeSet::from(["state", "reason"]));
    }
    assert_eq!(object_keys(&body["ipc"]), object_keys(&fixture["ipc"]));
    if body["query_latency"].is_object() {
        assert_eq!(
            object_keys(&body["query_latency"]),
            object_keys(&fixture["query_latency"])
        );
    }
    if !body["error"].is_null() {
        assert_eq!(
            object_keys(&body["error"]),
            BTreeSet::from(["code", "action", "capability", "reason"])
        );
    }
}

fn read_generation(data_dir: &Path) -> Option<Generation> {
    let endpoints: serde_json::Value =
        serde_json::from_slice(&fs::read(data_dir.join("ipc.endpoints.json")).ok()?).ok()?;
    let auth: serde_json::Value =
        serde_json::from_slice(&fs::read(data_dir.join("ipc.auth")).ok()?).ok()?;
    if endpoints["schema_version"] != "resume-ir.daemon-ipc.v3"
        || auth["schema_version"] != "resume-ir.daemon-auth.v3"
        || endpoints["launch_id"] != auth["launch_id"]
        || endpoints["instance_id"] != auth["instance_id"]
    {
        return None;
    }
    assert_eq!(
        object_keys(&endpoints),
        BTreeSet::from([
            "schema_version",
            "launch_id",
            "instance_id",
            "owner_mode",
            "status",
            "diagnostics",
            "imports",
            "import_cancel",
            "import_control",
            "import_progress",
            "search",
            "search_batch",
            "details",
            "delete",
        ])
    );
    assert_eq!(
        object_keys(&auth),
        BTreeSet::from(["schema_version", "launch_id", "instance_id", "token"])
    );
    assert_eq!(endpoints["owner_mode"], "desktop_supervised");
    assert_eq!(endpoints["launch_id"], LAUNCH_ID);
    assert_eq!(endpoints["instance_id"].as_str()?.len(), 64);
    assert_eq!(auth["token"].as_str()?.len(), 64);
    Some(Generation {
        launch_id: endpoints["launch_id"].as_str()?.to_string(),
        instance_id: endpoints["instance_id"].as_str()?.to_string(),
        token: auth["token"].as_str()?.to_string(),
        status_endpoint: endpoints["status"].as_str()?.to_string(),
        search_endpoint: endpoints["search"].as_str()?.to_string(),
    })
}

fn object_keys(value: &serde_json::Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

struct Response {
    status_code: u16,
    body: serde_json::Value,
    raw: String,
}

fn request(endpoint: &str, token: &str, method: &str, body: Option<serde_json::Value>) -> Response {
    let (address, path) = endpoint
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap();
    let body = body.map(|body| body.to_string()).unwrap_or_default();
    let content_headers = if body.is_empty() {
        String::new()
    } else {
        format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        )
    };
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "{method} /{path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\n{content_headers}Connection: close\r\n\r\n{body}"
    )
    .unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let status_code = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap();
    let body = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
    Response {
        status_code,
        body,
        raw,
    }
}

struct DesktopDaemon {
    child: ContainedChild,
    parent_stdin: Option<ChildStdin>,
    stderr: Option<ChildStderr>,
}

impl DesktopDaemon {
    fn spawn(data_dir: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_resume-daemon"));
        command
            .args([
                "--data-dir",
                data_dir.to_str().unwrap(),
                "run",
                "--foreground",
                "--parent-lifecycle-stdin",
                "--launch-id",
                LAUNCH_ID,
                "--expected-ipc-protocol",
                "resume-ir.daemon-ipc.v3",
                "--ipc-listen",
                "127.0.0.1:0",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = ContainedChild::spawn(&mut command).unwrap();
        let parent_stdin = child.take_stdin();
        let stderr = child.take_stderr();
        Self {
            child,
            parent_stdin,
            stderr,
        }
    }

    fn finish(mut self) {
        drop(self.parent_stdin.take());
        let status = self.child.wait().unwrap();
        let mut stderr = Vec::new();
        self.stderr
            .take()
            .unwrap()
            .read_to_end(&mut stderr)
            .unwrap();
        assert!(
            status.success(),
            "daemon shutdown failed: {}",
            String::from_utf8_lossy(&stderr)
        );
        assert!(stderr.is_empty());
    }
}

fn snapshot_existing_files(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut paths = Vec::new();
    collect_regular_files(root, root, &mut paths);
    snapshot_selected_files(root, paths.iter())
}

fn snapshot_selected_files<'a>(
    root: &Path,
    paths: impl IntoIterator<Item = &'a PathBuf>,
) -> BTreeMap<PathBuf, String> {
    paths
        .into_iter()
        .map(|relative| {
            let bytes = fs::read(root.join(relative)).unwrap();
            (relative.clone(), format!("{:x}", Sha256::digest(bytes)))
        })
        .collect()
}

fn collect_regular_files(root: &Path, current: &Path, paths: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let metadata = fs::symlink_metadata(entry.path()).unwrap();
        if metadata.file_type().is_dir() {
            collect_regular_files(root, &entry.path(), paths);
        } else if metadata.file_type().is_file() {
            paths.push(entry.path().strip_prefix(root).unwrap().to_path_buf());
        }
    }
}
