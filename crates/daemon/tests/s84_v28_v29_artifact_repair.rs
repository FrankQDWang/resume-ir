use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{ChildStderr, ChildStdin, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use index_fulltext::{FullTextIndex, SearchQuery, SnapshotReadLease, FULLTEXT_SNAPSHOT_SCHEMA_V3};
use index_vector::{VectorModelContract, VectorSnapshotRoot, VECTOR_SNAPSHOT_SCHEMA_V4};
use meta_store::{
    migration_test_support::{
        seed_v28_legacy_artifact_repair_fixture, V28ArtifactRepairHead,
        V28LegacyArtifactRepairFixtureFacts,
    },
    ArtifactRepairAttemptAcquire, ArtifactRepairAttemptErrorKind, ArtifactRepairAttemptPhase,
    ArtifactRepairAttemptState, ArtifactRepairContext, ArtifactRepairKey,
    ArtifactRepairVectorContext, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    MetaStoreErrorClass, ReadMetaStore, SearchProjectionDigest, SearchProjectionServiceState,
    SearchProjectionState, SearchRepairReason, UnixTimestamp, VectorSnapshotMode, CLASSIFIER_EPOCH,
};
use process_containment::ContainedChild;
use tempfile::tempdir;

#[path = "support/publication_gate.rs"]
mod publication_gate;

use publication_gate::{PublicationArtifact, PublicationGate};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const REPAIR_TIMEOUT: Duration = Duration::from_secs(70);
const SEARCH_TERM: &str = "artifact";

#[test]
fn desktop_combined_daemon_repairs_v28_artifacts_without_restarting() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let fixture =
        seed_v28_legacy_artifact_repair_fixture(&data_dir, V28ArtifactRepairHead::Ready).unwrap();
    assert!(!data_dir.join("ipc.endpoints.json").exists());
    assert!(!data_dir.join("ipc.auth").exists());
    let gate = PublicationGate::acquire(&data_dir, PublicationArtifact::FullText).unwrap();
    assert!(gate.is_held());
    assert_eq!(format!("{gate:?}"), "PublicationGate(<redacted>)");

    let mut daemon = DesktopDaemon::spawn(&data_dir);
    let generation = wait_for_fresh_ipc_generation(&mut daemon, &data_dir, None);
    let child_id = daemon.id();
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let attempt = wait_for_attempt(&mut daemon, &store, "first repair retry", |attempt| {
        attempt.phase == ArtifactRepairAttemptPhase::RetryWait
            && attempt.attempt_count == 1
            && attempt.last_error_kind
                == Some(ArtifactRepairAttemptErrorKind::FullTextPublicationBusy)
    });
    assert!(attempt.next_retry_at.is_some());
    assert_repairing_v29_contract(&store, &fixture);

    let status = request_json(&generation.status_endpoint, &generation.token, "GET", None);
    assert_eq!(status.status_code, 200, "{}", status.raw);
    assert_eq!(status.body["schema_version"], "daemon.status.v2");
    assert_eq!(status.body["process_state"], "ready");
    assert_eq!(status.body["status"], "repairing");
    assert_eq!(status.body["service_state"], "repairing");
    assert_eq!(status.body["services"]["metadata"], "ready");
    assert_eq!(status.body["services"]["query"], "repairing");
    assert_eq!(status.body["repair_reason"], "artifact_unavailable");
    assert_eq!(status.body["repair_progress"]["phase"], "retry_wait");
    assert_eq!(status.body["repair_progress"]["attempt"], 1);
    assert_eq!(status.body["repair_progress"]["max_attempts"], 5);
    assert_eq!(status.body["error"]["code"], "REPAIRING");

    let repairing_search = search(&generation, "s84-repairing-search");
    assert_eq!(
        repairing_search.status_code, 503,
        "{}",
        repairing_search.raw
    );
    assert_eq!(
        repairing_search.body["schema_version"],
        "resume-ir.error.v1"
    );
    assert_eq!(repairing_search.body["request_id"], "s84-repairing-search");
    assert_eq!(repairing_search.body["error"]["code"], "REPAIRING");
    assert_eq!(repairing_search.body["error"]["action"], "wait_for_repair");
    assert!(gate.is_held());

    gate.release().unwrap();
    let ready = wait_for_ready_projection(&mut daemon, &store, &fixture);
    assert_eq!(daemon.id(), child_id);
    assert_generation_is_current(&mut daemon, &data_dir, child_id, &generation.instance_id);
    assert_ready_artifacts(&data_dir, &ready, &fixture);
    assert_eq!(store.artifact_repair_context().unwrap(), None);
    assert_eq!(store.artifact_repair_attempt_state().unwrap(), None);

    let ready_status = request_json(&generation.status_endpoint, &generation.token, "GET", None);
    assert_eq!(ready_status.status_code, 200, "{}", ready_status.raw);
    assert_eq!(ready_status.body["status"], "ok");
    assert_eq!(ready_status.body["service_state"], "ready");
    assert_eq!(ready_status.body["services"]["metadata"], "ready");
    assert_eq!(ready_status.body["services"]["query"], "ready");
    assert_eq!(ready_status.body["repair_reason"], serde_json::Value::Null);
    assert_eq!(
        ready_status.body["repair_progress"],
        serde_json::Value::Null
    );

    let ready_search = search(&generation, "s84-ready-search");
    assert_exact_ready_search(&ready_search, &ready, &fixture);
    assert_generation_is_current(&mut daemon, &data_dir, child_id, &generation.instance_id);
    daemon.finish_clean(&data_dir);
}

#[test]
fn desktop_combined_daemon_retries_vector_publication_busy_and_converges_without_restarting() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let fixture =
        seed_v28_legacy_artifact_repair_fixture(&data_dir, V28ArtifactRepairHead::Ready).unwrap();
    let gate = PublicationGate::acquire(&data_dir, PublicationArtifact::Vector).unwrap();

    let mut daemon = DesktopDaemon::spawn(&data_dir);
    let generation = wait_for_fresh_ipc_generation(&mut daemon, &data_dir, None);
    let child_id = daemon.id();
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let attempt = wait_for_attempt(&mut daemon, &store, "vector repair retry", |attempt| {
        attempt.phase == ArtifactRepairAttemptPhase::RetryWait
            && attempt.attempt_count == 1
            && attempt.last_error_kind
                == Some(ArtifactRepairAttemptErrorKind::VectorPublicationBusy)
    });
    assert!(attempt.next_retry_at.is_some());
    assert_repairing_v29_contract(&store, &fixture);
    assert_no_search_artifact_candidates(&data_dir);
    assert_eq!(daemon.id(), child_id);

    let status = request_json(&generation.status_endpoint, &generation.token, "GET", None);
    assert_eq!(status.status_code, 200, "{}", status.raw);
    assert_eq!(status.body["process_state"], "ready");
    assert_eq!(status.body["service_state"], "repairing");
    assert_eq!(status.body["repair_progress"]["phase"], "retry_wait");
    assert_eq!(status.body["repair_progress"]["attempt"], 1);
    assert_eq!(
        status.body["repair_progress"]["last_error_kind"],
        "vector_publication_busy"
    );
    assert_eq!(status.body["error"]["action"], "wait_for_repair");
    assert!(gate.is_held());

    let second_attempt = wait_for_attempt(
        &mut daemon,
        &store,
        "second resident vector repair retry",
        |attempt| {
            attempt.phase == ArtifactRepairAttemptPhase::RetryWait
                && attempt.attempt_count == 2
                && attempt.last_error_kind
                    == Some(ArtifactRepairAttemptErrorKind::VectorPublicationBusy)
        },
    );
    assert!(second_attempt.next_retry_at.is_some());
    assert_no_search_artifact_candidates(&data_dir);
    assert_generation_is_current(&mut daemon, &data_dir, child_id, &generation.instance_id);

    gate.release().unwrap();
    let ready = wait_for_ready_projection(&mut daemon, &store, &fixture);
    assert_generation_is_current(&mut daemon, &data_dir, child_id, &generation.instance_id);
    assert_ready_artifacts(&data_dir, &ready, &fixture);
    daemon.finish_clean(&data_dir);
}

#[test]
fn desktop_combined_daemon_exhausts_publication_busy_as_repair_required_without_exiting() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    seed_v28_legacy_artifact_repair_fixture(&data_dir, V28ArtifactRepairHead::Ready).unwrap();
    let gate = PublicationGate::acquire(&data_dir, PublicationArtifact::FullText).unwrap();

    let mut daemon = DesktopDaemon::spawn(&data_dir);
    let generation = wait_for_fresh_ipc_generation(&mut daemon, &data_dir, None);
    let child_id = daemon.id();
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let blocked = poll_until(&mut daemon, REPAIR_TIMEOUT, "fifth repair block", || {
        // This assertion spans the complete bounded backoff window while the
        // daemon is committing repair state. A transient read observation is
        // request-scoped; convergence is the invariant under test. Keep
        // polling the same resident daemon and require the exact terminal
        // state by the deadline.
        let projection = store.search_projection_state().ok()?;
        let attempt = store.artifact_repair_attempt_state().ok()??;
        (projection.service_state == SearchProjectionServiceState::RepairBlocked
            && attempt.attempt_count == 5
            && attempt.last_error_kind
                == Some(ArtifactRepairAttemptErrorKind::FullTextPublicationBusy))
        .then_some((projection, attempt))
    });
    assert_eq!(
        blocked.0.repair_reason,
        Some(SearchRepairReason::RuntimeInvariant)
    );
    assert!(gate.is_held());
    assert_no_search_artifact_candidates(&data_dir);
    assert_generation_is_current(&mut daemon, &data_dir, child_id, &generation.instance_id);

    let status = request_json(&generation.status_endpoint, &generation.token, "GET", None);
    assert_eq!(status.status_code, 200, "{}", status.raw);
    assert_eq!(status.body["process_state"], "ready");
    assert_eq!(status.body["service_state"], "degraded");
    assert_eq!(status.body["repair_progress"]["phase"], "blocked");
    assert_eq!(status.body["repair_progress"]["attempt"], 5);
    assert_eq!(status.body["repair_progress"]["max_attempts"], 5);
    assert_eq!(
        status.body["repair_progress"]["retry_after_ms"],
        serde_json::Value::Null
    );
    assert_eq!(
        status.body["repair_progress"]["last_error_kind"],
        "fulltext_publication_busy"
    );
    assert_eq!(status.body["error"]["code"], "QUERY_SERVICE_UNAVAILABLE");
    assert_eq!(status.body["error"]["action"], "repair_required");

    let search = search(&generation, "s84-blocked-search");
    assert_eq!(search.status_code, 503, "{}", search.raw);
    assert_eq!(search.body["error"]["code"], "QUERY_SERVICE_UNAVAILABLE");
    assert_eq!(search.body["error"]["action"], "repair_required");
    assert_generation_is_current(&mut daemon, &data_dir, child_id, &generation.instance_id);
    daemon.finish_clean(&data_dir);
    assert!(gate.is_held());
}

#[test]
fn daemon_normalizes_a_preexisting_orphaned_repair_before_attempt_two() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    let fixture =
        seed_v28_legacy_artifact_repair_fixture(&data_dir, V28ArtifactRepairHead::Ready).unwrap();
    seed_orphaned_running_attempt(&data_dir);
    let gate = PublicationGate::acquire(&data_dir, PublicationArtifact::FullText).unwrap();

    let mut second = DesktopDaemon::spawn(&data_dir);
    let second_generation = wait_for_fresh_ipc_generation(&mut second, &data_dir, None);
    let second_child_id = second.id();
    let second_store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let interrupted = wait_for_attempt(
        &mut second,
        &second_store,
        "orphaned repair normalization",
        |attempt| {
            attempt.phase == ArtifactRepairAttemptPhase::RetryWait
                && attempt.attempt_count == 1
                && attempt.last_error_kind == Some(ArtifactRepairAttemptErrorKind::Interrupted)
        },
    );
    assert!(interrupted.next_retry_at.is_some());
    assert!(gate.is_held());
    let second_attempt = wait_for_attempt(
        &mut second,
        &second_store,
        "second repair attempt",
        |attempt| {
            attempt.phase == ArtifactRepairAttemptPhase::RetryWait
                && attempt.attempt_count == 2
                && attempt.last_error_kind
                    == Some(ArtifactRepairAttemptErrorKind::FullTextPublicationBusy)
        },
    );
    assert!(second_attempt.next_retry_at.is_some());
    assert!(gate.is_held());

    gate.release().unwrap();
    let ready = wait_for_ready_projection(&mut second, &second_store, &fixture);
    assert_eq!(second.id(), second_child_id);
    assert_generation_is_current(
        &mut second,
        &data_dir,
        second_child_id,
        &second_generation.instance_id,
    );
    assert_eq!(second_store.artifact_repair_context().unwrap(), None);
    assert_eq!(second_store.artifact_repair_attempt_state().unwrap(), None);
    let status = request_json(
        &second_generation.status_endpoint,
        &second_generation.token,
        "GET",
        None,
    );
    assert_eq!(status.status_code, 200, "{}", status.raw);
    assert_eq!(status.body["service_state"], "ready");
    assert_eq!(status.body["repair_reason"], serde_json::Value::Null);
    assert_ne!(ready.generation.as_deref(), Some(fixture.generation()));
    second.finish_clean(&data_dir);
}

#[test]
fn desktop_daemon_cancels_cleanly_while_publication_lock_stays_contended() {
    let workspace = tempdir().unwrap();
    let data_dir = workspace.path().join("data");
    seed_v28_legacy_artifact_repair_fixture(&data_dir, V28ArtifactRepairHead::Ready).unwrap();
    let gate = PublicationGate::acquire(&data_dir, PublicationArtifact::FullText).unwrap();
    let mut daemon = DesktopDaemon::spawn(&data_dir);
    let _generation = wait_for_fresh_ipc_generation(&mut daemon, &data_dir, None);
    let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();

    wait_for_attempt(&mut daemon, &store, "contended repair retry", |attempt| {
        attempt.phase == ArtifactRepairAttemptPhase::RetryWait
            && attempt.attempt_count == 1
            && attempt.last_error_kind
                == Some(ArtifactRepairAttemptErrorKind::FullTextPublicationBusy)
    });
    assert!(gate.is_held());
    daemon.finish_clean(&data_dir);
    assert!(gate.is_held());
}

fn seed_orphaned_running_attempt(data_dir: &Path) {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data dir is owned"),
    };
    let store = owner.open_store().unwrap();
    let context = store.artifact_repair_context().unwrap().unwrap();
    let key = ArtifactRepairKey::new(
        context.generation,
        context.publication_fingerprint,
        context.visible_epoch,
    );
    let mut session = store.wait_for_search_publication_session().unwrap();
    assert!(matches!(
        session
            .acquire_artifact_repair_attempt(&key, UnixTimestamp::from_unix_seconds(1))
            .unwrap(),
        ArtifactRepairAttemptAcquire::Started(_)
    ));
}

fn assert_repairing_v29_contract(
    store: &ReadMetaStore,
    fixture: &V28LegacyArtifactRepairFixtureFacts,
) {
    assert_eq!(store.schema_version().unwrap(), 29);
    let state = store.search_projection_state().unwrap();
    assert_eq!(state.service_state, SearchProjectionServiceState::Repairing);
    assert_eq!(
        state.repair_reason,
        Some(SearchRepairReason::ArtifactUnavailable)
    );
    assert_eq!(state.generation.as_deref(), Some(fixture.generation()));
    assert_eq!(state.visible_epoch, fixture.inherited_visible_epoch());
    assert_eq!(state.publication, None);
    let context = store.artifact_repair_context().unwrap().unwrap();
    assert_exact_context(&context, fixture);
}

fn assert_exact_context(
    context: &ArtifactRepairContext,
    fixture: &V28LegacyArtifactRepairFixtureFacts,
) {
    let projection_digest = SearchProjectionDigest::from_pairs([(
        fixture.document_id().as_str(),
        fixture.resume_version_id().as_str(),
    )])
    .unwrap();
    assert_eq!(context.generation, fixture.generation());
    assert_eq!(context.visible_epoch, fixture.inherited_visible_epoch());
    assert_eq!(context.classifier_epoch, CLASSIFIER_EPOCH);
    assert_eq!(context.projection_digest, projection_digest);
    assert_eq!(context.projection_count, 1);
    assert_eq!(context.vector, ArtifactRepairVectorContext::Disabled);
    let publication_fingerprint = context.publication_fingerprint.as_str();
    assert_eq!(publication_fingerprint.len(), 71);
    assert!(publication_fingerprint.starts_with("sha256:"));
    assert!(publication_fingerprint[7..]
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
}

fn wait_for_attempt(
    daemon: &mut DesktopDaemon,
    store: &ReadMetaStore,
    label: &str,
    mut predicate: impl FnMut(&ArtifactRepairAttemptState) -> bool,
) -> ArtifactRepairAttemptState {
    poll_until(daemon, REPAIR_TIMEOUT, label, || {
        retry_transient_metadata_read(store.artifact_repair_attempt_state())?
            .filter(|attempt| predicate(attempt))
    })
}

fn wait_for_ready_projection(
    daemon: &mut DesktopDaemon,
    store: &ReadMetaStore,
    fixture: &V28LegacyArtifactRepairFixtureFacts,
) -> SearchProjectionState {
    let state = poll_until(daemon, REPAIR_TIMEOUT, "ready artifact publication", || {
        let state = retry_transient_metadata_read(store.search_projection_state())?;
        (state.service_state == SearchProjectionServiceState::Ready
            && state.repair_reason.is_none()
            && state.generation.as_deref() != Some(fixture.generation()))
        .then_some(state)
    });
    assert_eq!(state.visible_epoch, fixture.inherited_visible_epoch() + 1);
    state
}

fn retry_transient_metadata_read<T>(result: meta_store::Result<T>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) if error.class() == MetaStoreErrorClass::Storage => None,
        Err(error) => panic!("non-transient metadata read failure: {error:?}"),
    }
}

fn assert_ready_artifacts(
    data_dir: &Path,
    ready: &SearchProjectionState,
    fixture: &V28LegacyArtifactRepairFixtureFacts,
) {
    let generation = ready.generation.as_deref().unwrap();
    let publication = ready.publication.as_ref().unwrap();
    let fulltext_descriptor = publication.fulltext.as_ref().unwrap();
    assert_eq!(fulltext_descriptor.generation(), generation);
    assert_eq!(
        fulltext_descriptor.manifest_schema(),
        "fulltext.snapshot.v3"
    );
    assert_eq!(fulltext_descriptor.index_schema(), "tantivy.fulltext.v3");
    let fulltext_root = data_dir.join("search-index");
    let fulltext_lease = SnapshotReadLease::acquire(&fulltext_root).unwrap().unwrap();
    let inspected_fulltext = FullTextIndex::inspect_snapshot_manifest_with_lease(
        &fulltext_root,
        generation,
        &fulltext_lease,
    )
    .unwrap()
    .unwrap();
    assert_eq!(inspected_fulltext.schema(), FULLTEXT_SNAPSHOT_SCHEMA_V3);
    assert_eq!(inspected_fulltext.generation(), generation);
    assert_eq!(inspected_fulltext.document_count(), 1);
    assert_eq!(
        inspected_fulltext.projection_digest(),
        fulltext_descriptor.projection_digest()
    );
    let fulltext =
        FullTextIndex::open_snapshot_with_lease(&fulltext_root, generation, fulltext_lease)
            .unwrap()
            .unwrap();
    assert_eq!(fulltext.snapshot_metadata(), Some(&inspected_fulltext));
    let hits = fulltext
        .search(SearchQuery::new(SEARCH_TERM).with_limit(1))
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].doc_id, fixture.document_id().as_str());
    assert_eq!(
        hits[0].resume_version_id,
        fixture.resume_version_id().as_str()
    );

    let vector_descriptor = publication.vector.as_ref().unwrap();
    assert_eq!(vector_descriptor.generation(), generation);
    assert_eq!(vector_descriptor.manifest_schema(), "vector.snapshot.v4");
    assert_eq!(vector_descriptor.index_schema(), "hnsw-vector.v4");
    assert_eq!(vector_descriptor.mode(), &VectorSnapshotMode::Disabled);
    let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index")).unwrap();
    let vector_lease = vector_root.acquire_read_lease().unwrap();
    let inspected_vector = vector_root
        .inspect_generation_manifest_with_lease(
            generation,
            &VectorModelContract::Disabled,
            &vector_lease,
        )
        .unwrap()
        .unwrap();
    assert_eq!(inspected_vector.schema(), VECTOR_SNAPSHOT_SCHEMA_V4);
    assert_eq!(inspected_vector.generation(), generation);
    assert_eq!(inspected_vector.projection_count(), 1);
    assert_eq!(inspected_vector.vector_count(), 0);
    assert_eq!(
        inspected_vector.projection_digest(),
        vector_descriptor.projection_digest()
    );
    let vector = vector_root
        .open_generation_with_lease(generation, &VectorModelContract::Disabled, vector_lease)
        .unwrap();
    assert_eq!(vector.summary().schema(), VECTOR_SNAPSHOT_SCHEMA_V4);
    assert_eq!(vector.exact_projection().len(), 1);
    assert_eq!(
        vector.exact_projection()[0].document_id.as_str(),
        fixture.document_id().as_str()
    );
    assert_eq!(
        vector.exact_projection()[0].resume_version_id.as_str(),
        fixture.resume_version_id().as_str()
    );
}

fn assert_no_search_artifact_candidates(data_dir: &Path) {
    for path in [
        data_dir.join("search-index/snapshots"),
        data_dir.join("search-index/staging"),
        data_dir.join("search-index/generation-pins"),
        data_dir.join("vector-index/snapshots"),
        data_dir.join("vector-index/staging"),
        data_dir.join("vector-index/generation-pins"),
    ] {
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries.count(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => panic!("read synthetic artifact layout: {error}"),
        };
        assert_eq!(entries, 0, "failed repair generation accumulated artifacts");
    }
}

fn assert_exact_ready_search(
    response: &HttpResponse,
    ready: &SearchProjectionState,
    fixture: &V28LegacyArtifactRepairFixtureFacts,
) {
    assert_eq!(response.status_code, 200, "{}", response.raw);
    assert_eq!(
        response.body["schema_version"],
        "resume-ir.search-response.v3"
    );
    assert_eq!(response.body["request_id"], "s84-ready-search");
    assert_eq!(response.body["status"], "ok");
    assert_eq!(response.body["visible_epoch"], ready.visible_epoch);
    assert_eq!(response.body["result_count"], 1);
    let results = response.body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["selection"]["doc_id"],
        fixture.document_id().as_str()
    );
    assert_eq!(
        results[0]["selection"]["version_id"],
        fixture.resume_version_id().as_str()
    );
    assert_eq!(
        results[0]["selection"]["visible_epoch"],
        ready.visible_epoch
    );
}

fn search(generation: &IpcGeneration, request_id: &str) -> HttpResponse {
    request_json(
        &generation.search_endpoint,
        &generation.token,
        "POST",
        Some(serde_json::json!({
            "schema_version": "resume-ir.ipc-request.v3",
            "request_id": request_id,
            "client_capability": "codex_validation",
            "deadline_ms": 5_000,
            "payload": {
                "query": SEARCH_TERM,
                "mode": "fulltext",
                "top_k": 1,
            },
        })),
    )
}

fn poll_until<T>(
    daemon: &mut DesktopDaemon,
    timeout: Duration,
    label: &str,
    mut observe: impl FnMut() -> Option<T>,
) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = observe() {
            return value;
        }
        daemon.assert_alive(label);
        assert!(Instant::now() < deadline, "timed out waiting for {label}");
        thread::sleep(POLL_INTERVAL);
    }
}

struct IpcGeneration {
    instance_id: String,
    token: String,
    status_endpoint: String,
    search_endpoint: String,
}

fn wait_for_fresh_ipc_generation(
    daemon: &mut DesktopDaemon,
    data_dir: &Path,
    rejected_instance: Option<&str>,
) -> IpcGeneration {
    poll_until(daemon, STARTUP_TIMEOUT, "fresh IPC generation", || {
        let endpoints = read_owner_json(&data_dir.join("ipc.endpoints.json"))?;
        let auth = read_owner_json(&data_dir.join("ipc.auth"))?;
        let endpoint_instance = endpoints["instance_id"].as_str()?;
        let auth_instance = auth["instance_id"].as_str()?;
        if endpoint_instance != auth_instance || rejected_instance == Some(endpoint_instance) {
            return None;
        }
        assert_eq!(endpoints["schema_version"], "resume-ir.daemon-ipc.v2");
        assert_eq!(endpoints["owner_mode"], "desktop_supervised");
        assert_eq!(auth["schema_version"], "resume-ir.daemon-auth.v2");
        assert_eq!(endpoint_instance.len(), 64);
        let token = auth["token"].as_str().unwrap();
        assert_eq!(token.len(), 64);
        let status_endpoint = endpoints["status"].as_str().unwrap();
        let search_endpoint = endpoints["search"].as_str().unwrap();
        assert!(status_endpoint.starts_with("http://127.0.0.1:"));
        assert!(search_endpoint.starts_with("http://127.0.0.1:"));
        Some(IpcGeneration {
            instance_id: endpoint_instance.to_string(),
            token: token.to_string(),
            status_endpoint: status_endpoint.to_string(),
            search_endpoint: search_endpoint.to_string(),
        })
    })
}

fn read_owner_json(path: &Path) -> Option<serde_json::Value> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("read daemon owner file metadata: {error}"),
    };
    assert!(
        metadata.file_type().is_file(),
        "daemon owner path is unsafe"
    );
    assert!(
        metadata.len() <= 16 * 1024,
        "daemon owner file is unbounded"
    );
    #[cfg(unix)]
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    let body = match fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => panic!("read daemon owner file: {error}"),
    };
    serde_json::from_str(&body).ok()
}

fn assert_generation_is_current(
    daemon: &mut DesktopDaemon,
    data_dir: &Path,
    child_id: u32,
    instance_id: &str,
) {
    daemon.assert_alive("same daemon generation");
    assert_eq!(daemon.id(), child_id);
    let endpoints = read_owner_json(&data_dir.join("ipc.endpoints.json")).unwrap();
    let auth = read_owner_json(&data_dir.join("ipc.auth")).unwrap();
    assert_eq!(endpoints["instance_id"], instance_id);
    assert_eq!(auth["instance_id"], instance_id);
}

fn request_json(
    endpoint: &str,
    token: &str,
    method: &str,
    body: Option<serde_json::Value>,
) -> HttpResponse {
    let rest = endpoint.strip_prefix("http://").unwrap();
    let (address, path) = rest.split_once('/').unwrap();
    let body = body.map(|value| value.to_string()).unwrap_or_default();
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
    HttpResponse::parse(raw)
}

struct HttpResponse {
    status_code: u16,
    body: serde_json::Value,
    raw: String,
}

impl HttpResponse {
    fn parse(raw: String) -> Self {
        let status_code = raw
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|value| value.parse().ok())
            .unwrap();
        let body = serde_json::from_str(raw.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        Self {
            status_code,
            body,
            raw,
        }
    }
}

struct DesktopDaemon {
    child: Option<ContainedChild>,
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
                "--work-imports",
                "--work-index",
                "--rescan-completed-imports",
                "--watch-import-roots",
                "--import-rescan-min-age-seconds",
                "300",
                "--expected-ipc-protocol",
                "resume-ir.daemon-ipc.v2",
                "--ipc-listen",
                "127.0.0.1:0",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = ContainedChild::spawn(&mut command).unwrap();
        let parent_stdin = child.take_stdin().unwrap();
        let stderr = child.take_stderr().unwrap();
        Self {
            child: Some(child),
            parent_stdin: Some(parent_stdin),
            stderr: Some(stderr),
        }
    }

    fn id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    fn assert_alive(&mut self, context: &str) {
        let status = self.child.as_mut().unwrap().try_wait().unwrap();
        if let Some(status) = status {
            let stderr = self.read_stderr();
            panic!("daemon exited while waiting for {context}: {status}; stderr={stderr:?}");
        }
    }

    fn finish_clean(mut self, data_dir: &Path) {
        drop(self.parent_stdin.take());
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let status = loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "daemon did not stop after parent EOF"
            );
            thread::sleep(POLL_INTERVAL);
        };
        let stderr = self.read_stderr();
        assert!(status.success(), "daemon exit={status}; stderr={stderr:?}");
        assert!(stderr.is_empty(), "unexpected daemon stderr: {stderr}");
        assert!(!stderr.contains("\"event\":\"fatal\""));
        assert!(!data_dir.join("ipc.endpoints.json").exists());
        assert!(!data_dir.join("ipc.auth").exists());
    }

    fn read_stderr(&mut self) -> String {
        let mut stderr = String::new();
        self.stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        stderr
    }
}

impl Drop for DesktopDaemon {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            child.terminate();
        }
    }
}
