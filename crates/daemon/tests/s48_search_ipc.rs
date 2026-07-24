use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::num::NonZeroUsize;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use import_pipeline::{
    current_import_processing_contract, finalize_migration_rebuild, import_root_with_options,
    prepare_migration_rebuild_artifacts, ImportOptions, ImportParseWorkers, PipelineRunControl,
    SearchPublicationEmbeddingFailure, SearchPublicationEmbeddingInput,
    SearchPublicationEmbeddingOutput, SearchPublicationVectorization, SearchPublicationVectorizer,
};
use meta_store::{
    ActiveSearchProjection, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    ExactHitHydration, ImportTask, ImportTaskId, ImportTaskStatus, OwnedMetaStore, ReadMetaStore,
    SearchProjectionServiceState, UnixTimestamp,
};
use process_containment::ContainedChild;

mod support;

const TARGET_FILE: &str = "synthetic-target.txt";
const DUPLICATE_FILE: &str = "synthetic-duplicate.txt";
const KNOWN_TIER_FILE: &str = "synthetic-known-tier.txt";
const SYNTHETIC_VECTOR_MODEL_ID: &str = "intfloat-multilingual-e5-small-qint8-r1";
const SYNTHETIC_VECTOR_DIMENSION: usize = 384;
const CORE_CONVERGENCE_TIMEOUT: Duration = Duration::from_secs(120);

const TARGET_TEXT: &str = "\
Synthetic Candidate
Email: shared-candidate@example.test
Location: Shanghai, China
SUMMARY
filtersentinel filtersentinel filtersentinel foldsentinel
EDUCATION
School: Synthetic University
Degree: MSc Computer Science
Major: Computer Science
EXPERIENCE
Company: Synthetic Payments Inc.
Title: Senior Backend Engineer
2017.01 - 2024.03
Built reliable synthetic search services.
SKILLS
Rust, Java
CERTIFICATIONS
PMP
";

const DUPLICATE_TEXT: &str = "\
Synthetic Duplicate Candidate
Email: shared-candidate@example.test
SUMMARY
filtersentinel foldsentinel
EDUCATION
School: Synthetic College
Degree: Bachelor of Science
EXPERIENCE
2023.01 - 2024.01
Built synthetic backend services.
SKILLS
Java
";

const KNOWN_TIER_TEXT: &str = "\
Synthetic Known Candidate
SUMMARY
filtersentinel
EDUCATION
School: Synthetic 985 University
School Tier: 985
Degree: Bachelor of Science
EXPERIENCE
2023.01 - 2024.01
Built synthetic platform services.
SKILLS
Go
";

#[test]
fn keyword_v3_returns_exact_selections_and_all_filter_semantics() {
    let corpus = SyntheticCorpus::rich("keyword-v3");
    let target_candidate = candidate_id(&corpus.store, &corpus.target);
    let duplicate_candidate = candidate_id(&corpus.store, &corpus.duplicate);
    assert_eq!(target_candidate, duplicate_candidate);
    let contact_hash = corpus
        .store
        .candidate_by_id(&target_candidate)
        .unwrap()
        .unwrap()
        .email_hash
        .unwrap();

    let mut daemon = DaemonHarness::start(&corpus.data_dir);
    let folded = daemon.search(
        "candidate-folding",
        serde_json::json!({
            "query": "foldsentinel",
            "mode": "fulltext",
            "top_k": 10,
        }),
    );
    assert_ok_search(&folded, "candidate-folding");
    assert_eq!(folded.body["result_count"], 1);
    assert_candidate_pair_selection(
        &folded.body["results"][0],
        &corpus.target,
        &corpus.duplicate,
    );

    let cases = vec![
        ("degree", serde_json::json!({"degree_min": "master"}), false),
        ("skill", serde_json::json!({"skills_any": ["rust"]}), false),
        (
            "contact",
            serde_json::json!({"contact_hashes_any": [contact_hash.as_str()]}),
            true,
        ),
        (
            "unknown-school-tier",
            serde_json::json!({"school_tiers_any": ["unknown"]}),
            true,
        ),
        (
            "name",
            serde_json::json!({"names_any": ["SYNTHETIC CANDIDATE"]}),
            false,
        ),
        (
            "school",
            serde_json::json!({"schools_any": ["SYNTHETIC UNIVERSITY"]}),
            false,
        ),
        (
            "major",
            serde_json::json!({"majors_any": ["COMPUTER_SCIENCE"]}),
            false,
        ),
        (
            "certificate",
            serde_json::json!({"certificates_any": ["PMP"]}),
            false,
        ),
        (
            "date-range",
            serde_json::json!({"date_range_overlaps": "2021-01/2021-12"}),
            false,
        ),
        (
            "company",
            serde_json::json!({"companies_any": ["SYNTHETIC PAYMENTS"]}),
            false,
        ),
        (
            "title",
            serde_json::json!({"titles_any": ["BACKEND_ENGINEER"]}),
            false,
        ),
        (
            "location",
            serde_json::json!({"locations_any": ["SHANGHAI"]}),
            false,
        ),
        (
            "years-experience",
            serde_json::json!({"years_experience_min": 5.0}),
            false,
        ),
    ];
    assert_eq!(cases.len(), 13);

    for (label, filters, folded_pair) in cases {
        let request_id = format!("filter-{label}");
        let response = daemon.search(
            &request_id,
            serde_json::json!({
                "query": "filtersentinel",
                "mode": "fulltext",
                "top_k": 10,
                "filters": filters,
            }),
        );
        assert_ok_search(&response, &request_id);
        assert_eq!(response.body["result_count"], 1, "{label}");
        let result = &response.body["results"][0];
        if folded_pair {
            assert_candidate_pair_selection(result, &corpus.target, &corpus.duplicate);
        } else {
            assert_selection(result, &corpus.target);
        }
        assert_eq!(
            result["selection"]["visible_epoch"], response.body["visible_epoch"],
            "{label}"
        );
        assert!(result.get("doc_id").is_none(), "{label}");
        assert!(result.get("version_id").is_none(), "{label}");
        assert!(!response.raw.contains(contact_hash.as_str()), "{label}");
    }

    daemon.finish();
    corpus.remove();
}

#[test]
fn missing_embedding_keeps_keyword_and_detail_and_returns_lexical_hybrid_partial() {
    let corpus = SyntheticCorpus::single_vectorized(
        "embedding-unavailable-hybrid",
        "synthetic-lexical-fallback.txt",
        resume_text("lexicalfallbacksentinel", "Synthetic Lexical Fallback"),
    );
    let mut daemon = DaemonHarness::start(&corpus.data_dir);

    let keyword = daemon.search(
        "embedding-unavailable-keyword",
        serde_json::json!({
            "query": "lexicalfallbacksentinel",
            "mode": "fulltext",
            "top_k": 1,
        }),
    );
    assert_ok_search(&keyword, "embedding-unavailable-keyword");
    assert_selection(&keyword.body["results"][0], &corpus.target);

    let hybrid = daemon.search(
        "embedding-unavailable-hybrid",
        serde_json::json!({
            "query": "lexicalfallbacksentinel",
            "mode": "hybrid",
            "top_k": 1,
        }),
    );
    assert_eq!(hybrid.status_code, 200, "{}", hybrid.raw);
    assert_eq!(
        hybrid.body["schema_version"],
        "resume-ir.search-response.v3"
    );
    assert_eq!(hybrid.body["query_mode"], "hybrid");
    assert_eq!(hybrid.body["partial"], true);
    assert_eq!(
        hybrid.body["partial_reasons"],
        serde_json::json!(["embedding_runtime_unavailable"])
    );
    assert_eq!(hybrid.body["result_count"], 1);
    assert_selection(&hybrid.body["results"][0], &corpus.target);

    let detail = daemon.detail(
        "embedding-unavailable-detail",
        &hybrid.body["results"][0]["selection"],
    );
    assert_eq!(detail.status_code, 200, "{}", detail.raw);
    assert_eq!(
        detail.body["schema_version"],
        "resume-ir.detail-response.v3"
    );
    assert_eq!(
        detail.body["selection"],
        hybrid.body["results"][0]["selection"]
    );

    daemon.finish();
    corpus.remove();
}

#[test]
fn product_disabled_semantic_contract_returns_semantic_disabled() {
    let corpus = SyntheticCorpus::single(
        "semantic-product-disabled",
        "semantic-disabled.txt",
        resume_text("semanticdisabledsentinel", "Synthetic Semantic Disabled"),
    );
    let mut daemon = DaemonHarness::start_with_embedding_runtime(&corpus.data_dir);
    let status = daemon.wait_for_core_state("ready");
    assert_eq!(
        status.body["optional_runtimes"]["embedding"]["state"],
        "available"
    );
    assert_eq!(
        status.body["capabilities"]["semantic_search"]["state"],
        "available"
    );

    let semantic = daemon.search(
        "product-disabled-semantic",
        serde_json::json!({
            "query": "semanticdisabledsentinel",
            "mode": "semantic",
            "top_k": 1,
        }),
    );
    assert_eq!(semantic.status_code, 503, "{}", semantic.raw);
    assert_eq!(semantic.body["schema_version"], "resume-ir.error.v2");
    assert_eq!(semantic.body["request_id"], "product-disabled-semantic");
    assert_eq!(semantic.body["error"]["code"], "SEMANTIC_DISABLED");
    assert_eq!(semantic.body["error"]["action"], "select_supported_mode");
    assert_eq!(
        semantic.body["error"]["capability"],
        serde_json::Value::Null
    );
    assert_eq!(semantic.body["error"]["reason"], serde_json::Value::Null);
    assert!(semantic.body.get("results").is_none());
    assert!(!semantic.raw.contains("semanticdisabledsentinel"));

    daemon.finish();
    corpus.remove();
}

#[test]
fn invalid_filter_and_unavailable_semantic_runtime_fail_closed() {
    let corpus = SyntheticCorpus::rich("hard-errors");
    let mut daemon = DaemonHarness::start(&corpus.data_dir);

    let invalid = daemon.search(
        "unknown-filter",
        serde_json::json!({
            "query": "private-unknown-filter-query",
            "mode": "fulltext",
            "filters": {"legacy_visibility": "searchable"},
        }),
    );
    assert_eq!(invalid.status_code, 400);
    assert_eq!(invalid.body["error"]["code"], "BAD_REQUEST");
    assert!(!invalid.raw.contains("private-unknown-filter-query"));

    let semantic = daemon.search(
        "unavailable-semantic",
        serde_json::json!({
            "query": "filtersentinel",
            "mode": "semantic",
            "top_k": 10,
        }),
    );
    assert_eq!(semantic.status_code, 503, "{}", semantic.raw);
    assert_eq!(semantic.body["schema_version"], "resume-ir.error.v2");
    assert_eq!(semantic.body["request_id"], "unavailable-semantic");
    assert_eq!(semantic.body["error"]["code"], "CAPABILITY_UNAVAILABLE");
    assert_eq!(semantic.body["error"]["action"], "select_supported_mode");
    assert_eq!(semantic.body["error"]["capability"], "semantic_search");
    assert_eq!(semantic.body["error"]["reason"], "embedding_unavailable");
    assert!(semantic.body.get("results").is_none());
    assert!(!semantic.raw.contains("filtersentinel"));

    daemon.finish();
    corpus.remove();
}

#[test]
fn content_update_publishes_a_new_immutable_version_pair() {
    let corpus = SyntheticCorpus::single(
        "immutable-update",
        "versioned.txt",
        resume_text("immutablealpha", "Synthetic Alpha Candidate"),
    );
    let old_projection = corpus.target.clone();
    let old_version = corpus
        .store
        .resume_version_by_id(&old_projection.resume_version_id)
        .unwrap()
        .unwrap();
    let mut daemon = DaemonHarness::start(&corpus.data_dir);

    let before = daemon.search(
        "immutable-before",
        serde_json::json!({"query": "immutablealpha", "mode": "fulltext"}),
    );
    assert_ok_search(&before, "immutable-before");
    assert_selection(&before.body["results"][0], &old_projection);
    daemon.finish();

    fs::write(
        corpus.source_root.join("versioned.txt"),
        resume_text(
            "immutablebeta with deliberately different bytes",
            "Synthetic Beta Candidate",
        ),
    )
    .unwrap();
    let store = open_owned_store(&corpus.data_dir);
    run_import(
        &corpus.data_dir,
        &corpus.source_root,
        &store,
        "immutable-update-second",
        1_800_048_100,
    );
    drop(store);
    let new_projection = active_projection_for_file(&corpus.store, "versioned.txt");
    assert_ne!(
        old_projection.resume_version_id,
        new_projection.resume_version_id
    );
    assert_eq!(
        corpus
            .store
            .resume_version_by_id(&old_projection.resume_version_id)
            .unwrap()
            .unwrap(),
        old_version
    );

    let mut daemon = DaemonHarness::start(&corpus.data_dir);
    let after = daemon.search(
        "immutable-after",
        serde_json::json!({"query": "immutablebeta", "mode": "fulltext"}),
    );
    assert_ok_search(&after, "immutable-after");
    assert_selection(&after.body["results"][0], &new_projection);
    assert_ne!(before.body["visible_epoch"], after.body["visible_epoch"]);

    daemon.finish();
    corpus.remove();
}

#[test]
fn corrupted_published_generation_is_rebuilt_before_search() {
    let corpus = SyntheticCorpus::single_vectorized(
        "corrupt-generation",
        "cached.txt",
        resume_text("cacheoldsentry", "Synthetic Cached Candidate"),
    );
    let mut daemon = DaemonHarness::start(&corpus.data_dir);
    let cached = daemon.search(
        "cache-prime",
        serde_json::json!({"query": "cacheoldsentry", "mode": "fulltext"}),
    );
    assert_ok_search(&cached, "cache-prime");
    assert_eq!(cached.body["result_count"], 1);
    daemon.finish();

    fs::write(
        corpus.source_root.join("new-generation.txt"),
        resume_text("newgenerationsentry", "Synthetic New Candidate"),
    )
    .unwrap();
    let store = open_owned_store(&corpus.data_dir);
    run_vectorized_import(
        &corpus.data_dir,
        &corpus.source_root,
        &store,
        "corrupt-generation-second",
        1_800_048_200,
    );
    let generation = store.search_projection_state().unwrap().generation.unwrap();
    drop(store);
    let corrupted = corpus
        .data_dir
        .join("search-index")
        .join("snapshots")
        .join(&generation);
    fs::remove_dir_all(&corrupted).unwrap();

    let mut daemon = DaemonHarness::start_with_index_worker(&corpus.data_dir);
    wait_for_repaired_generation(&mut daemon, &corpus.store, &generation);
    let ready = daemon.wait_for_core_state("ready");
    assert_eq!(
        ready.body["capabilities"]["keyword_search"]["state"],
        "available"
    );
    let response = daemon.search(
        "cache-must-not-fallback",
        serde_json::json!({"query": "cacheoldsentry", "mode": "fulltext"}),
    );
    assert_ok_search(&response, "cache-must-not-fallback");
    assert_eq!(response.body["result_count"], 1);

    daemon.finish();
    let state = corpus.store.search_projection_state().unwrap();
    assert_eq!(
        state.service_state,
        meta_store::SearchProjectionServiceState::Ready
    );
    assert_eq!(state.repair_reason, None);
    assert_ne!(state.generation.as_deref(), Some(generation.as_str()));
    corpus.remove();
}

#[test]
fn client_disconnect_only_ends_that_connection() {
    let corpus = SyntheticCorpus::single(
        "client-disconnect",
        "disconnect.txt",
        resume_text("disconnectsentry", "Synthetic Disconnect Candidate"),
    );
    let mut daemon = DaemonHarness::start(&corpus.data_dir);
    daemon.disconnect_mid_request();

    let response = daemon.search(
        "after-disconnect",
        serde_json::json!({"query": "disconnectsentry", "mode": "fulltext"}),
    );
    assert_ok_search(&response, "after-disconnect");
    assert_eq!(response.body["result_count"], 1);

    daemon.finish();
    corpus.remove();
}

#[test]
fn persisted_startup_repair_converges_before_the_first_post_repair_search() {
    let corpus = SyntheticCorpus::single_vectorized(
        "repairing-context",
        "repairing.txt",
        resume_text("repairingsentry", "Synthetic Repairing Candidate"),
    );
    let state = corpus.store.search_projection_state().unwrap();
    let store = open_owned_store(&corpus.data_dir);
    store
        .begin_artifact_repair(
            state.generation.as_deref().unwrap(),
            state.visible_epoch,
            UnixTimestamp::from_unix_seconds(1_800_048_300),
        )
        .unwrap();
    drop(store);
    let repair_generation = state.generation.unwrap();
    let mut daemon = DaemonHarness::start_with_index_worker(&corpus.data_dir);
    wait_for_repaired_generation(&mut daemon, &corpus.store, &repair_generation);
    let ready = daemon.wait_for_core_state("ready");
    assert_eq!(
        ready.body["capabilities"]["keyword_search"]["state"],
        "available"
    );

    let response = daemon.search(
        "repairing-request-context",
        serde_json::json!({"query": "repairingsentry", "mode": "fulltext"}),
    );
    assert_ok_search(&response, "repairing-request-context");
    assert_eq!(response.body["request_id"], "repairing-request-context");
    assert_eq!(response.body["result_count"], 1);

    daemon.finish();
    let state = corpus.store.search_projection_state().unwrap();
    assert_eq!(
        state.service_state,
        meta_store::SearchProjectionServiceState::Ready
    );
    assert_eq!(state.repair_reason, None);
    corpus.remove();
}

#[test]
fn status_only_does_not_deep_open_a_corrupt_search_payload() {
    let corpus = SyntheticCorpus::single(
        "status-shallow-probe",
        "status-shallow.txt",
        resume_text("statusshallowsentry", "Synthetic Shallow Status Candidate"),
    );
    let generation = corrupt_active_fulltext_payload(&corpus);
    let mut daemon = DaemonHarness::start(&corpus.data_dir);

    let response = daemon.status();
    assert_eq!(response.status_code, 200, "{}", response.raw);
    daemon.finish();

    let state = corpus.store.search_projection_state().unwrap();
    assert_eq!(
        state.service_state,
        meta_store::SearchProjectionServiceState::Ready
    );
    assert_eq!(state.generation.as_deref(), Some(generation.as_str()));
    corpus.remove();
}

#[test]
fn routine_index_ticks_use_manifests_without_deep_opening_payloads() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let corpus = SyntheticCorpus::single_vectorized(
        "routine-shallow-probe",
        "routine-shallow.txt",
        resume_text("routineshallowsentry", "Synthetic Shallow Tick Candidate"),
    );
    let generation = corrupt_active_fulltext_payload(&corpus);

    let output = support::import_capable_daemon_command(&runtime_capacity)
        .args([
            "--data-dir",
            path_str(&corpus.data_dir),
            "run",
            "--foreground",
            "--work-index",
            "--worker-interval-ms",
            "1",
            "--max-worker-ticks",
            "3",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "daemon stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let state = corpus.store.search_projection_state().unwrap();
    assert_eq!(
        state.service_state,
        meta_store::SearchProjectionServiceState::Ready
    );
    assert_eq!(state.generation.as_deref(), Some(generation.as_str()));
    corpus.remove();
}

#[test]
fn closed_bootstrap_stdout_does_not_interrupt_query_fault_repair_or_final_accepted_response() {
    let corpus = SyntheticCorpus::single_vectorized(
        "query-fault-repair",
        "query-fault.txt",
        resume_text("queryfaultsentry", "Synthetic Query Fault Candidate"),
    );
    let corrupt_generation = corrupt_active_fulltext_payload(&corpus);
    let mut daemon = DaemonHarness::start_with_index_worker(&corpus.data_dir);

    let failed = daemon.search(
        "query-fault-first",
        serde_json::json!({"query": "queryfaultsentry", "mode": "fulltext"}),
    );
    assert_eq!(failed.status_code, 503, "{}", failed.raw);
    assert_eq!(failed.body["schema_version"], "resume-ir.error.v2");
    assert_eq!(failed.body["request_id"], "query-fault-first");
    assert_eq!(failed.body["error"]["code"], "QUERY_SERVICE_UNAVAILABLE");
    assert_eq!(failed.body["error"]["action"], "repair_required");
    assert_eq!(failed.body["error"]["capability"], serde_json::Value::Null);
    assert_eq!(failed.body["error"]["reason"], serde_json::Value::Null);
    wait_for_repaired_generation(&mut daemon, &corpus.store, &corrupt_generation);
    let ready = daemon.wait_for_core_state("ready");
    assert_eq!(
        ready.body["capabilities"]["keyword_search"]["state"],
        "available"
    );

    let recovered = daemon.search(
        "query-fault-recovered",
        serde_json::json!({"query": "queryfaultsentry", "mode": "fulltext"}),
    );
    assert_ok_search(&recovered, "query-fault-recovered");
    assert_eq!(recovered.body["result_count"], 1);
    daemon.finish();
    corpus.remove();
}

#[test]
fn generation_local_key_fault_is_repaired_without_restarting_daemon() {
    let corpus = SyntheticCorpus::single_vectorized(
        "query-key-fault-repair",
        "query-key-fault.txt",
        resume_text("querykeyfaultsentry", "Synthetic Query Key Fault Candidate"),
    );
    let corrupt_generation = corrupt_active_fulltext_key(&corpus);
    let mut daemon = DaemonHarness::start_with_index_worker(&corpus.data_dir);

    let failed = daemon.search(
        "query-key-fault-first",
        serde_json::json!({"query": "querykeyfaultsentry", "mode": "fulltext"}),
    );
    assert_eq!(failed.status_code, 503, "{}", failed.raw);
    assert_eq!(failed.body["schema_version"], "resume-ir.error.v2");
    assert_eq!(failed.body["request_id"], "query-key-fault-first");
    assert_eq!(failed.body["error"]["code"], "QUERY_SERVICE_UNAVAILABLE");
    assert_eq!(failed.body["error"]["action"], "repair_required");
    assert_eq!(failed.body["error"]["capability"], serde_json::Value::Null);
    assert_eq!(failed.body["error"]["reason"], serde_json::Value::Null);
    wait_for_repaired_generation(&mut daemon, &corpus.store, &corrupt_generation);
    let ready = daemon.wait_for_core_state("ready");
    assert_eq!(
        ready.body["capabilities"]["keyword_search"]["state"],
        "available"
    );

    let recovered = daemon.search(
        "query-key-fault-recovered",
        serde_json::json!({"query": "querykeyfaultsentry", "mode": "fulltext"}),
    );
    assert_ok_search(&recovered, "query-key-fault-recovered");
    assert_eq!(recovered.body["result_count"], 1);
    daemon.finish();
    corpus.remove();
}

#[cfg(unix)]
#[test]
fn unsafe_artifact_root_blocks_services_without_exiting_daemon() {
    let corpus = SyntheticCorpus::single_vectorized(
        "unsafe-artifact-root",
        "unsafe-root.txt",
        resume_text("unsaferootsentry", "Synthetic Unsafe Root Candidate"),
    );
    let ready = corpus.store.search_projection_state().unwrap();
    let snapshots = corpus.data_dir.join("search-index/snapshots");
    let real_snapshots = corpus.data_dir.join("search-index/snapshots-real");
    fs::rename(&snapshots, &real_snapshots).unwrap();
    symlink(&real_snapshots, &snapshots).unwrap();
    let mut daemon = DaemonHarness::start_with_index_worker(&corpus.data_dir);
    wait_for_repair_blocked(&mut daemon, &corpus.store);

    let status = daemon.wait_for_core_state("blocked");
    assert_eq!(status.status_code, 200, "{}", status.raw);
    assert_eq!(status.body["schema_version"], "daemon.status.v3");
    assert_eq!(status.body["status"], "blocked");
    assert_eq!(status.body["process_state"], "ready");
    assert_eq!(status.body["core"]["state"], "blocked");
    assert_eq!(status.body["core"]["reason"], "runtime_invariant");
    assert_eq!(status.body["repair_progress"]["phase"], "blocked");
    assert_eq!(status.body["error"]["code"], "SERVICE_BLOCKED");
    assert_eq!(
        status.body["error"]["action"], "repair_required",
        "{}",
        status.raw
    );
    assert_eq!(status.body["error"]["capability"], serde_json::Value::Null);
    assert_eq!(status.body["error"]["reason"], "runtime_invariant");

    let selection = serde_json::json!({
        "doc_id": corpus.target.document_id.as_str(),
        "version_id": corpus.target.resume_version_id.as_str(),
        "visible_epoch": ready.visible_epoch,
    });
    let detail = daemon.detail("unsafe-root-detail", &selection);
    assert_eq!(detail.status_code, 503, "{}", detail.raw);
    assert_eq!(detail.body["schema_version"], "resume-ir.error.v2");
    assert_eq!(detail.body["request_id"], "unsafe-root-detail");
    assert_eq!(detail.body["error"]["code"], "SERVICE_BLOCKED");
    assert_eq!(detail.body["error"]["action"], "repair_required");
    assert_eq!(detail.body["error"]["capability"], serde_json::Value::Null);
    assert_eq!(detail.body["error"]["reason"], "runtime_invariant");

    let search = daemon.search(
        "unsafe-root-search",
        serde_json::json!({"query": "unsaferootsentry", "mode": "fulltext"}),
    );
    assert_eq!(search.status_code, 503, "{}", search.raw);
    assert_eq!(search.body["schema_version"], "resume-ir.error.v2");
    assert_eq!(search.body["request_id"], "unsafe-root-search");
    assert_eq!(search.body["error"]["code"], "SERVICE_BLOCKED");
    assert_eq!(search.body["error"]["action"], "repair_required");
    assert_eq!(search.body["error"]["capability"], serde_json::Value::Null);
    assert_eq!(search.body["error"]["reason"], "runtime_invariant");

    daemon.assert_running("after unsafe-root detail and search requests");
    let next_status = daemon.status();
    assert_eq!(next_status.status_code, 200, "{}", next_status.raw);
    assert_eq!(next_status.body["schema_version"], "daemon.status.v3");
    assert_eq!(next_status.body["status"], "blocked");
    assert_eq!(next_status.body["process_state"], "ready");
    assert_eq!(next_status.body["core"]["state"], "blocked");
    assert_eq!(next_status.body["core"]["reason"], "runtime_invariant");
    daemon.finish();

    let blocked = corpus.store.search_projection_state().unwrap();
    assert_eq!(
        blocked.service_state,
        meta_store::SearchProjectionServiceState::RepairBlocked
    );
    assert_eq!(
        blocked.repair_reason,
        Some(meta_store::SearchRepairReason::RuntimeInvariant)
    );
    assert_eq!(blocked.generation, ready.generation);
    assert_eq!(blocked.visible_epoch, ready.visible_epoch);
    corpus.remove();
}

struct SyntheticCorpus {
    base: PathBuf,
    data_dir: PathBuf,
    source_root: PathBuf,
    store: ReadMetaStore,
    target: ActiveSearchProjection,
    duplicate: ActiveSearchProjection,
}

impl SyntheticCorpus {
    fn rich(label: &str) -> Self {
        let (base, data_dir, source_root, store) = empty_corpus(label);
        fs::write(source_root.join(TARGET_FILE), TARGET_TEXT).unwrap();
        fs::write(source_root.join(DUPLICATE_FILE), DUPLICATE_TEXT).unwrap();
        fs::write(source_root.join(KNOWN_TIER_FILE), KNOWN_TIER_TEXT).unwrap();
        run_import(
            &data_dir,
            &source_root,
            &store,
            &format!("{label}-initial"),
            1_800_048_000,
        );
        drop(store);
        let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let target = active_projection_for_file(&store, TARGET_FILE);
        let duplicate = active_projection_for_file(&store, DUPLICATE_FILE);
        assert!(store
            .active_search_projection_for_document(&document_for_file(&store, KNOWN_TIER_FILE).id)
            .unwrap()
            .is_some());
        Self {
            base,
            data_dir,
            source_root,
            store,
            target,
            duplicate,
        }
    }

    fn single(label: &str, file_name: &str, text: String) -> Self {
        let (base, data_dir, source_root, store) = empty_corpus(label);
        fs::write(source_root.join(file_name), text).unwrap();
        run_import(
            &data_dir,
            &source_root,
            &store,
            &format!("{label}-initial"),
            1_800_048_000,
        );
        drop(store);
        let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let target = active_projection_for_file(&store, file_name);
        Self {
            base,
            data_dir,
            source_root,
            store,
            duplicate: target.clone(),
            target,
        }
    }

    fn single_vectorized(label: &str, file_name: &str, text: String) -> Self {
        let (base, data_dir, source_root, store) = empty_corpus(label);
        fs::write(source_root.join(file_name), text).unwrap();
        run_vectorized_import(
            &data_dir,
            &source_root,
            &store,
            &format!("{label}-initial"),
            1_800_048_000,
        );
        drop(store);
        let store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let target = active_projection_for_file(&store, file_name);
        Self {
            base,
            data_dir,
            source_root,
            store,
            duplicate: target.clone(),
            target,
        }
    }

    fn remove(self) {
        let _ = fs::remove_dir_all(self.base);
    }
}

fn empty_corpus(label: &str) -> (PathBuf, PathBuf, PathBuf, OwnedMetaStore) {
    let base = temp_dir(label);
    let data_dir = base.join("data");
    let source_root = base.join("source");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&source_root).unwrap();
    let store = open_owned_store(&data_dir);
    (base, data_dir, source_root, store)
}

fn run_import(
    data_dir: &Path,
    source_root: &Path,
    store: &OwnedMetaStore,
    label: &str,
    timestamp: i64,
) {
    let now = UnixTimestamp::from_unix_seconds(timestamp);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s48-v29", label]),
        root_path: source_root.to_string_lossy().into_owned(),
        status: ImportTaskStatus::Running,
        queued_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    };
    support::insert_import_task(store, &task);
    let options = ImportOptions {
        parse_workers: ImportParseWorkers::sequential(),
        ..ImportOptions::default()
    };
    let summary =
        import_root_with_options(data_dir, store, &task, source_root, now, options).unwrap();
    assert!(
        summary.searchable_documents > 0,
        "import did not publish synthetic resumes: {summary:?}"
    );
}

fn run_vectorized_import(
    data_dir: &Path,
    source_root: &Path,
    store: &OwnedMetaStore,
    label: &str,
    timestamp: i64,
) {
    let vectorization = SearchPublicationVectorization::enabled(Arc::new(SyntheticVectorizer));
    let options = ImportOptions {
        parse_workers: ImportParseWorkers::sequential(),
        search_vectorization: vectorization.clone(),
        ..ImportOptions::default()
    };
    let now = UnixTimestamp::from_unix_seconds(timestamp);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s48-vectorized", label]),
        root_path: source_root.to_string_lossy().into_owned(),
        status: ImportTaskStatus::Running,
        queued_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    };
    insert_import_task_for_options(store, &task, &options, &vectorization);
    let summary =
        import_root_with_options(data_dir, store, &task, source_root, now, options).unwrap();
    assert!(
        summary.searchable_documents > 0,
        "vectorized import did not publish synthetic resumes: {summary:?}"
    );
}

fn insert_import_task_for_options(
    store: &OwnedMetaStore,
    task: &ImportTask,
    options: &ImportOptions,
    vectorization: &SearchPublicationVectorization,
) {
    let contract = current_import_processing_contract(options).unwrap();
    store
        .activate_migration_rebuild_contract(&contract, task.queued_at)
        .unwrap();
    let control = PipelineRunControl::default();
    prepare_migration_rebuild_artifacts(store, task.queued_at, &control).unwrap();
    finalize_migration_rebuild(store, task.queued_at, &contract, vectorization, &control).unwrap();
    assert_eq!(
        store.search_projection_state().unwrap().service_state,
        SearchProjectionServiceState::Ready
    );
    let queued = ImportTask {
        status: ImportTaskStatus::Queued,
        started_at: None,
        ..task.clone()
    };
    store
        .insert_import_task_with_scan_scope(
            &queued,
            &support::empty_import_scan_scope(&queued),
            &contract,
        )
        .unwrap();
    store
        .claim_observed_import_task_for_worker(&queued, task.started_at.unwrap())
        .unwrap()
        .unwrap();
}

struct SyntheticVectorizer;

impl SearchPublicationVectorizer for SyntheticVectorizer {
    fn model_id(&self) -> &str {
        SYNTHETIC_VECTOR_MODEL_ID
    }

    fn dimension(&self) -> usize {
        SYNTHETIC_VECTOR_DIMENSION
    }

    fn max_batch_inputs(&self) -> usize {
        8
    }

    fn max_text_bytes(&self) -> usize {
        64 * 1024
    }

    fn embed_batch(
        &self,
        inputs: &[SearchPublicationEmbeddingInput],
        _is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<SearchPublicationEmbeddingOutput>, SearchPublicationEmbeddingFailure> {
        Ok(inputs
            .iter()
            .map(|input| {
                let mut vector = vec![0.0; SYNTHETIC_VECTOR_DIMENSION];
                vector[0] = 1.0;
                vector[1] = input.text().len() as f32;
                SearchPublicationEmbeddingOutput::new(input.id(), self.model_id(), vector)
            })
            .collect())
    }
}

fn document_for_file(store: &ReadMetaStore, file_name: &str) -> meta_store::Document {
    store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == file_name)
        .unwrap()
}

fn active_projection_for_file(store: &ReadMetaStore, file_name: &str) -> ActiveSearchProjection {
    let document = document_for_file(store, file_name);
    store
        .active_search_projection_for_document(&document.id)
        .unwrap()
        .unwrap()
}

fn candidate_id(
    store: &ReadMetaStore,
    projection: &ActiveSearchProjection,
) -> meta_store::CandidateId {
    store
        .with_search_metadata_snapshot(|snapshot| {
            let cap = NonZeroUsize::new(1).unwrap();
            let hydrated = snapshot
                .hydrate_exact_hits(std::slice::from_ref(projection), cap)
                .unwrap();
            let ExactHitHydration::Hydrated(hits) = hydrated else {
                panic!("exact synthetic projection did not hydrate");
            };
            Ok::<_, ()>(hits.into_iter().next().unwrap().candidate_id.unwrap())
        })
        .unwrap()
}

fn open_owned_store(data_dir: &Path) -> OwnedMetaStore {
    let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
        DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        DataDirectoryOwnerAcquisition::Contended => panic!("synthetic data directory is owned"),
    };
    owner.open_store().unwrap()
}

fn resume_text(keyword: &str, name: &str) -> String {
    format!(
        "{name}\nSUMMARY\n{keyword}\nEDUCATION\nDegree: Bachelor of Science\nEXPERIENCE\n2020.01 - 2024.03\nBuilt reliable synthetic systems.\nSKILLS\nRust\n"
    )
}

fn corrupt_active_fulltext_payload(corpus: &SyntheticCorpus) -> String {
    let generation = corpus
        .store
        .search_projection_state()
        .unwrap()
        .generation
        .unwrap();
    fs::write(
        corpus
            .data_dir
            .join("search-index/snapshots")
            .join(&generation)
            .join("fulltext.snapshot.enc"),
        b"synthetic corrupt encrypted payload",
    )
    .unwrap();
    generation
}

fn corrupt_active_fulltext_key(corpus: &SyntheticCorpus) -> String {
    let generation = corpus
        .store
        .search_projection_state()
        .unwrap()
        .generation
        .unwrap();
    fs::write(
        corpus
            .data_dir
            .join("search-index/snapshots")
            .join(&generation)
            .join("fulltext.snapshot.key-v3"),
        b"synthetic corrupt generation-local key",
    )
    .unwrap();
    generation
}

fn wait_for_repaired_generation(
    daemon: &mut DaemonHarness,
    store: &ReadMetaStore,
    corrupt_generation: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let state = store.search_projection_state().unwrap();
        if state.service_state == meta_store::SearchProjectionServiceState::Ready
            && state.generation.as_deref() != Some(corrupt_generation)
        {
            return;
        }
        if let Some(status) = daemon
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .expect("poll daemon during artifact repair")
        {
            panic!("daemon exited during artifact repair: {status}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("reported query artifact fault did not converge");
}

fn wait_for_repair_blocked(daemon: &mut DaemonHarness, store: &ReadMetaStore) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let state = store.search_projection_state().unwrap();
        if state.service_state == meta_store::SearchProjectionServiceState::RepairBlocked
            && state.repair_reason == Some(meta_store::SearchRepairReason::RuntimeInvariant)
        {
            return;
        }
        if let Some(status) = daemon
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .expect("poll daemon during artifact block")
        {
            panic!("daemon exited during artifact block: {status}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("unsafe artifact layout did not become repair-blocked");
}

struct DaemonHarness {
    _runtime_capacity: Option<support::ImportRuntimeCapacityLease>,
    child: Option<ContainedChild>,
    parent_lifecycle: Option<ChildStdin>,
    bootstrap_stdout: Option<BufReader<ChildStdout>>,
    stderr: Option<ChildStderr>,
    endpoint: String,
    token: String,
}

#[derive(Clone, Copy)]
enum DaemonRuntimeFixture {
    Plain,
    ImportCapable,
}

impl DaemonHarness {
    fn start(data_dir: &Path) -> Self {
        Self::start_with_args(data_dir, &[], DaemonRuntimeFixture::Plain)
    }

    fn start_with_embedding_runtime(data_dir: &Path) -> Self {
        Self::start_with_args(data_dir, &[], DaemonRuntimeFixture::ImportCapable)
    }

    fn start_with_index_worker(data_dir: &Path) -> Self {
        Self::start_with_args(
            data_dir,
            &["--work-index", "--worker-interval-ms", "10"],
            DaemonRuntimeFixture::ImportCapable,
        )
    }

    fn start_with_args(
        data_dir: &Path,
        extra: &[&str],
        runtime_fixture: DaemonRuntimeFixture,
    ) -> Self {
        let launch_id = random_launch_id();
        let mut args = vec![
            "--data-dir",
            path_str(data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            launch_id.as_str(),
        ];
        args.extend_from_slice(extra);
        args.extend_from_slice(&["--ipc-listen", "127.0.0.1:0"]);
        let runtime_capacity = match runtime_fixture {
            DaemonRuntimeFixture::Plain => None,
            DaemonRuntimeFixture::ImportCapable => Some(support::import_runtime_capacity_lease()),
        };
        let mut command = match runtime_fixture {
            DaemonRuntimeFixture::Plain => Command::new(env!("CARGO_BIN_EXE_resume-daemon")),
            DaemonRuntimeFixture::ImportCapable => {
                support::import_capable_daemon_command(runtime_capacity.as_ref().unwrap())
            }
        };
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = ContainedChild::spawn(&mut command).unwrap();
        let parent_lifecycle = child.take_stdin().unwrap();
        let mut bootstrap_stdout = BufReader::new(child.take_stdout().unwrap());
        let mut stderr = child.take_stderr().unwrap();
        let endpoint = read_ipc_endpoint(&mut child, &mut stderr, &mut bootstrap_stdout);
        let token = read_ipc_auth_token(data_dir);
        let mut harness = Self {
            _runtime_capacity: runtime_capacity,
            child: Some(child),
            parent_lifecycle: Some(parent_lifecycle),
            bootstrap_stdout: Some(bootstrap_stdout),
            stderr: Some(stderr),
            endpoint,
            token,
        };
        harness.wait_for_core_initialized();
        drop(harness.bootstrap_stdout.take());
        harness
    }

    fn status(&self) -> HttpResponse {
        let response = self.request(&format!(
            "GET /status HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
            self.address(),
            self.token,
        ));
        response
    }

    fn search(&self, request_id: &str, payload: serde_json::Value) -> HttpResponse {
        let body = serde_json::json!({
            "schema_version": "resume-ir.ipc-request.v3",
            "request_id": request_id,
            "client_capability": "codex_validation",
            "deadline_ms": 5_000,
            "payload": payload,
        })
        .to_string();
        let response = self.request(&format!(
            "POST /search HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.address(),
            self.token,
            body.len(),
            body
        ));
        response
    }

    fn detail(&self, request_id: &str, selection: &serde_json::Value) -> HttpResponse {
        let body = serde_json::json!({
            "schema_version": "resume-ir.detail-request.v3",
            "request_id": request_id,
            "selection": selection,
        })
        .to_string();
        let response = self.request(&format!(
            "POST /details HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.address(),
            self.token,
            body.len(),
            body
        ));
        response
    }

    fn disconnect_mid_request(&self) {
        let mut stream = TcpStream::connect(self.address()).unwrap();
        let prefix = format!(
            "POST /search HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n{{\"schema_version\":",
            self.address(), self.token
        );
        stream.write_all(prefix.as_bytes()).unwrap();
        stream.shutdown(Shutdown::Both).unwrap();
        drop(stream);
        std::thread::sleep(Duration::from_millis(30));
    }

    fn wait_for_core_initialized(&self) {
        let response = self.wait_for_core(|state| state != "initializing");
        assert_eq!(response.status_code, 200, "{}", response.raw);
    }

    fn wait_for_core_state(&self, expected: &str) -> HttpResponse {
        self.wait_for_core(|state| state == expected)
    }

    fn wait_for_core(&self, predicate: impl Fn(&str) -> bool) -> HttpResponse {
        let deadline = Instant::now() + CORE_CONVERGENCE_TIMEOUT;
        while Instant::now() < deadline {
            let response = self.status();
            assert_eq!(response.status_code, 200, "{}", response.raw);
            if response.body["schema_version"] == "daemon.status.v3"
                && response.body["process_state"] == "ready"
                && response.body["core"]["state"]
                    .as_str()
                    .is_some_and(&predicate)
            {
                return response;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("daemon core state did not converge before the bounded deadline");
    }

    fn request(&self, request: &str) -> HttpResponse {
        const HTTP_DEADLINE: Duration = Duration::from_secs(5);
        const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;

        let deadline = Instant::now() + HTTP_DEADLINE;
        let address = self.address().parse().expect("parse daemon IPC address");
        let mut stream = TcpStream::connect_timeout(&address, HTTP_DEADLINE)
            .expect("connect to daemon IPC before deadline");
        stream
            .set_write_timeout(Some(deadline.saturating_duration_since(Instant::now())))
            .expect("configure bounded daemon IPC request write");
        stream
            .write_all(request.as_bytes())
            .expect("write daemon IPC request before deadline");
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "daemon IPC response deadline elapsed");
            stream
                .set_read_timeout(Some(remaining))
                .expect("configure bounded daemon IPC response read");
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    assert!(
                        bytes.len().saturating_add(read) <= MAX_HTTP_RESPONSE_BYTES,
                        "daemon IPC response exceeded {MAX_HTTP_RESPONSE_BYTES} bytes"
                    );
                    bytes.extend_from_slice(&buffer[..read]);
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => panic!("read daemon IPC response before deadline: {error}"),
            }
        }
        let raw = String::from_utf8(bytes).expect("daemon IPC response is UTF-8");
        HttpResponse::parse(raw)
    }

    fn assert_running(&mut self, context: &str) {
        assert!(
            self.child
                .as_mut()
                .expect("daemon child is owned")
                .try_wait()
                .expect("poll daemon child")
                .is_none(),
            "daemon exited {context}"
        );
    }

    fn address(&self) -> &str {
        self.endpoint
            .strip_prefix("http://")
            .unwrap()
            .split_once('/')
            .unwrap()
            .0
    }

    fn finish(&mut self) {
        drop(self.parent_lifecycle.take());
        let status = self.child.as_mut().unwrap().wait().unwrap();
        let mut stderr = Vec::new();
        self.stderr
            .take()
            .unwrap()
            .read_to_end(&mut stderr)
            .unwrap();
        assert!(
            status.success(),
            "daemon exited with {status}; stderr:\n{}",
            String::from_utf8_lossy(&stderr)
        );
        assert!(stderr.is_empty());
        self.child.take();
    }
}

fn random_launch_id() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).expect("generate daemon test launch identifier");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl Drop for DaemonHarness {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.terminate();
        }
    }
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
        let body = serde_json::from_str(raw.split("\r\n\r\n").nth(1).unwrap_or_default()).unwrap();
        Self {
            status_code,
            body,
            raw,
        }
    }
}

fn assert_ok_search(response: &HttpResponse, request_id: &str) {
    assert_eq!(response.status_code, 200, "{}", response.raw);
    assert_eq!(
        response.body["schema_version"],
        "resume-ir.search-response.v3"
    );
    assert_eq!(response.body["request_id"], request_id);
    assert_eq!(response.body["status"], "ok");
    assert_eq!(response.body["query_mode"], "keyword");
    assert_eq!(response.body["search_index"], "available");
    assert!(!response.raw.contains("shared-candidate@example.test"));
}

fn assert_selection(result: &serde_json::Value, expected: &ActiveSearchProjection) {
    assert_eq!(result["selection"]["doc_id"], expected.document_id.as_str());
    assert_eq!(
        result["selection"]["version_id"],
        expected.resume_version_id.as_str()
    );
}

fn assert_candidate_pair_selection(
    result: &serde_json::Value,
    first: &ActiveSearchProjection,
    second: &ActiveSearchProjection,
) {
    let doc_id = result["selection"]["doc_id"].as_str().unwrap();
    let version_id = result["selection"]["version_id"].as_str().unwrap();
    assert!(
        (doc_id == first.document_id.as_str() && version_id == first.resume_version_id.as_str())
            || (doc_id == second.document_id.as_str()
                && version_id == second.resume_version_id.as_str())
    );
}

fn read_ipc_endpoint(
    child: &mut ContainedChild,
    stderr: &mut ChildStderr,
    stdout: &mut BufReader<impl Read>,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        let bytes = stdout.read_line(&mut line).unwrap();
        if bytes == 0 {
            if let Ok(Some(status)) = child.try_wait() {
                let mut stderr_body = String::new();
                stderr.read_to_string(&mut stderr_body).unwrap();
                panic!("daemon exited before endpoint: {status}\nstderr:\n{stderr_body}");
            }
            continue;
        }
        if let Some(endpoint) = line.trim().strip_prefix("ipc status endpoint: ") {
            return endpoint.to_string();
        }
    }
    panic!("daemon did not print ipc endpoint");
}

fn read_ipc_auth_token(data_dir: &Path) -> String {
    let body = fs::read_to_string(data_dir.join("ipc.auth")).unwrap();
    let auth: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(auth["schema_version"], "resume-ir.daemon-auth.v3");
    assert_eq!(auth["launch_id"].as_str().map(str::len), Some(64));
    auth["token"].as_str().unwrap().to_string()
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s48-v29-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}
