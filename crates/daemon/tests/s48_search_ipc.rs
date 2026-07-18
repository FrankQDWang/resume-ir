use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use import_pipeline::{import_root_with_options, ImportOptions, ImportParseWorkers};
use meta_store::{
    ActiveSearchProjection, ExactHitHydration, ImportTask, ImportTaskId, ImportTaskStatus,
    MetaStore, SearchRepairReason, UnixTimestamp,
};

const TARGET_FILE: &str = "synthetic-target.txt";
const DUPLICATE_FILE: &str = "synthetic-duplicate.txt";
const KNOWN_TIER_FILE: &str = "synthetic-known-tier.txt";

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

    let mut daemon = DaemonHarness::start(&corpus.data_dir, 14);
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
fn invalid_filter_and_disabled_semantic_modes_fail_closed() {
    let corpus = SyntheticCorpus::rich("hard-errors");
    let mut daemon = DaemonHarness::start(&corpus.data_dir, 3);

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

    for mode in ["semantic", "hybrid"] {
        let request_id = format!("disabled-{mode}");
        let response = daemon.search(
            &request_id,
            serde_json::json!({
                "query": "filtersentinel",
                "mode": mode,
                "top_k": 10,
            }),
        );
        assert_eq!(response.status_code, 503, "{mode}: {}", response.raw);
        assert_eq!(response.body["request_id"], request_id);
        assert_eq!(response.body["error"]["code"], "SEMANTIC_DISABLED");
        assert!(response.body.get("results").is_none());
        assert!(!response.raw.contains("filtersentinel"));
    }

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
    let mut daemon = DaemonHarness::start(&corpus.data_dir, 2);

    let before = daemon.search(
        "immutable-before",
        serde_json::json!({"query": "immutablealpha", "mode": "fulltext"}),
    );
    assert_ok_search(&before, "immutable-before");
    assert_selection(&before.body["results"][0], &old_projection);

    fs::write(
        corpus.source_root.join("versioned.txt"),
        resume_text(
            "immutablebeta with deliberately different bytes",
            "Synthetic Beta Candidate",
        ),
    )
    .unwrap();
    run_import(
        &corpus.data_dir,
        &corpus.source_root,
        &corpus.store,
        "immutable-update-second",
        1_800_048_100,
    );
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
fn corrupted_new_generation_never_falls_back_to_cached_results() {
    let corpus = SyntheticCorpus::single(
        "corrupt-generation",
        "cached.txt",
        resume_text("cacheoldsentry", "Synthetic Cached Candidate"),
    );
    let mut daemon = DaemonHarness::start(&corpus.data_dir, 2);
    let cached = daemon.search(
        "cache-prime",
        serde_json::json!({"query": "cacheoldsentry", "mode": "fulltext"}),
    );
    assert_ok_search(&cached, "cache-prime");
    assert_eq!(cached.body["result_count"], 1);

    fs::write(
        corpus.source_root.join("new-generation.txt"),
        resume_text("newgenerationsentry", "Synthetic New Candidate"),
    )
    .unwrap();
    run_import(
        &corpus.data_dir,
        &corpus.source_root,
        &corpus.store,
        "corrupt-generation-second",
        1_800_048_200,
    );
    let generation = corpus
        .store
        .search_projection_state()
        .unwrap()
        .generation
        .unwrap();
    let corrupted = corpus
        .data_dir
        .join("search-index")
        .join("snapshots")
        .join(generation);
    fs::remove_dir_all(&corrupted).unwrap();

    let response = daemon.search(
        "cache-must-not-fallback",
        serde_json::json!({"query": "cacheoldsentry", "mode": "fulltext"}),
    );
    assert_eq!(response.status_code, 503, "{}", response.raw);
    assert_eq!(response.body["error"]["code"], "QUERY_SERVICE_UNAVAILABLE");
    assert!(response.body.get("results").is_none());
    assert!(!response.raw.contains("cacheoldsentry"));

    daemon.finish();
    corpus.remove();
}

#[test]
fn client_disconnect_only_ends_that_connection() {
    let corpus = SyntheticCorpus::single(
        "client-disconnect",
        "disconnect.txt",
        resume_text("disconnectsentry", "Synthetic Disconnect Candidate"),
    );
    let mut daemon = DaemonHarness::start(&corpus.data_dir, 2);
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
fn repairing_preflight_preserves_the_parsed_search_request_context() {
    let corpus = SyntheticCorpus::single(
        "repairing-context",
        "repairing.txt",
        resume_text("repairingsentry", "Synthetic Repairing Candidate"),
    );
    let mut daemon = DaemonHarness::start(&corpus.data_dir, 1);
    corpus
        .store
        .mark_search_repairing(
            SearchRepairReason::ArtifactUnavailable,
            UnixTimestamp::from_unix_seconds(1_800_048_300),
        )
        .unwrap();

    let response = daemon.search(
        "repairing-request-context",
        serde_json::json!({"query": "repairingsentry", "mode": "fulltext"}),
    );
    assert_eq!(response.status_code, 503, "{}", response.raw);
    assert_eq!(response.body["schema_version"], "resume-ir.error.v1");
    assert_eq!(response.body["request_id"], "repairing-request-context");
    assert_eq!(response.body["error"]["code"], "REPAIRING");
    assert_eq!(response.body["error"]["action"], "wait_for_repair");
    assert!(!response.raw.contains("repairingsentry"));

    daemon.finish();
    corpus.remove();
}

struct SyntheticCorpus {
    base: PathBuf,
    data_dir: PathBuf,
    source_root: PathBuf,
    store: MetaStore,
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

fn empty_corpus(label: &str) -> (PathBuf, PathBuf, PathBuf, MetaStore) {
    let base = temp_dir(label);
    let data_dir = base.join("data");
    let source_root = base.join("source");
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&source_root).unwrap();
    let store = MetaStore::open_data_dir(&data_dir).unwrap();
    store.run_migrations().unwrap();
    (base, data_dir, source_root, store)
}

fn run_import(data_dir: &Path, source_root: &Path, store: &MetaStore, label: &str, timestamp: i64) {
    let now = UnixTimestamp::from_unix_seconds(timestamp);
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["s48-v27", label]),
        root_path: source_root.to_string_lossy().into_owned(),
        status: ImportTaskStatus::Running,
        queued_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
    };
    store.insert_import_task(&task).unwrap();
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

fn document_for_file(store: &MetaStore, file_name: &str) -> meta_store::Document {
    store
        .visible_documents()
        .unwrap()
        .into_iter()
        .find(|document| document.file_name == file_name)
        .unwrap()
}

fn active_projection_for_file(store: &MetaStore, file_name: &str) -> ActiveSearchProjection {
    let document = document_for_file(store, file_name);
    store
        .active_search_projection_for_document(&document.id)
        .unwrap()
        .unwrap()
}

fn candidate_id(store: &MetaStore, projection: &ActiveSearchProjection) -> meta_store::CandidateId {
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

fn resume_text(keyword: &str, name: &str) -> String {
    format!(
        "{name}\nSUMMARY\n{keyword}\nEDUCATION\nDegree: Bachelor of Science\nEXPERIENCE\n2020.01 - 2024.03\nBuilt reliable synthetic systems.\nSKILLS\nRust\n"
    )
}

struct DaemonHarness {
    child: Option<Child>,
    endpoint: String,
    token: String,
}

impl DaemonHarness {
    fn start(data_dir: &Path, max_requests: usize) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
            .args([
                "--data-dir",
                path_str(data_dir),
                "run",
                "--foreground",
                "--ipc-listen",
                "127.0.0.1:0",
                "--max-requests",
                &max_requests.to_string(),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let endpoint = read_ipc_endpoint(&mut child, &mut BufReader::new(stdout));
        let token = read_ipc_auth_token(data_dir);
        Self {
            child: Some(child),
            endpoint,
            token,
        }
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
        self.request(&format!(
            "POST /search HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.address(),
            self.token,
            body.len(),
            body
        ))
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

    fn request(&self, request: &str) -> HttpResponse {
        let mut stream = TcpStream::connect(self.address()).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).unwrap();
        HttpResponse::parse(raw)
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
        let output = self.child.take().unwrap().wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "daemon stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
    }
}

impl Drop for DaemonHarness {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
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

fn read_ipc_endpoint(child: &mut Child, stdout: &mut BufReader<impl Read>) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut line = String::new();
    while Instant::now() < deadline {
        line.clear();
        let bytes = stdout.read_line(&mut line).unwrap();
        if bytes == 0 {
            if let Ok(Some(status)) = child.try_wait() {
                let mut stderr = String::new();
                child
                    .stderr
                    .as_mut()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                panic!("daemon exited before endpoint: {status}\nstderr:\n{stderr}");
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
    assert_eq!(auth["schema_version"], "resume-ir.daemon-auth.v2");
    auth["token"].as_str().unwrap().to_string()
}

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("resume-ir-s48-v27-{label}-{unique}"));
    fs::create_dir_all(&path).unwrap();
    path
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
}
