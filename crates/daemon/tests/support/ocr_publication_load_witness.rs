//! Synthetic continuous-publication resource and supervision witness.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use import_pipeline::{
    SearchPublicationEmbeddingFailure, SearchPublicationEmbeddingInput,
    SearchPublicationEmbeddingOutput, SearchPublicationVectorization, SearchPublicationVectorizer,
};
use meta_store::{IngestJobStatus, ReadMetaStore, UnixTimestamp};
use process_containment::ContainedChild;

use super::{
    path_str, remove_dir, response_payload, result_selection_order, support,
    synthetic_scanned_resume_pdf, wait_contained_with_stderr, wait_for_generation, wait_for_status,
};

const SYNTHETIC_SEARCHABLE_DOCUMENTS: usize = 7_328;
const SYNTHETIC_OCR_PUBLICATIONS: usize = 3;
const SYNTHETIC_SUPERVISOR_GENERATION: u64 = 1;
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(2);
const WITNESS_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_DAEMON_RSS_KIB: u64 = 512 * 1_024;

const QUERY_REDLINE_MS: [(&str, u128); 3] = [("fulltext", 120), ("semantic", 500), ("hybrid", 250)];

pub(super) fn witness_vectorization() -> SearchPublicationVectorization {
    SearchPublicationVectorization::enabled(Arc::new(SyntheticWitnessVectorizer))
}

struct SyntheticWitnessVectorizer;

impl SearchPublicationVectorizer for SyntheticWitnessVectorizer {
    fn model_id(&self) -> &str {
        "intfloat-multilingual-e5-small-qint8-r1"
    }

    fn dimension(&self) -> usize {
        384
    }

    fn max_batch_inputs(&self) -> usize {
        16
    }

    fn max_text_bytes(&self) -> usize {
        65_536
    }

    fn embed_batch(
        &self,
        inputs: &[SearchPublicationEmbeddingInput],
        _is_cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<SearchPublicationEmbeddingOutput>, SearchPublicationEmbeddingFailure> {
        Ok(inputs
            .iter()
            .map(|input| {
                let mut vector = vec![0.0; self.dimension()];
                vector[0] = 1.0;
                vector[1] = input.text().len() as f32;
                SearchPublicationEmbeddingOutput::new(input.id(), self.model_id(), vector)
            })
            .collect())
    }
}

#[test]
fn continuous_ocr_publications_keep_interactive_queries_and_supervision_within_redlines() {
    let runtime_capacity = support::import_runtime_capacity_lease();
    let scanned_fixture = synthetic_scanned_resume_pdf();
    let now = UnixTimestamp::from_unix_seconds(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    );
    let data_dir = super::data_dir_with_queued_ocr_load_corpus(
        "continuous-ocr-publication-query-load",
        &scanned_fixture,
        now,
        SYNTHETIC_OCR_PUBLICATIONS,
        SYNTHETIC_SEARCHABLE_DOCUMENTS,
    );
    let mut daemon = support::fully_capable_daemon_command(&runtime_capacity);
    daemon
        .args([
            "--data-dir",
            path_str(&data_dir),
            "run",
            "--foreground",
            "--parent-lifecycle-stdin",
            "--launch-id",
            "6161616161616161616161616161616161616161616161616161616161616161",
            "--work-ocr",
            "--work-index",
            "--ocr-jobs-per-tick",
            "3",
            "--worker-interval-ms",
            "60000",
            "--ipc-listen",
            "127.0.0.1:0",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = ContainedChild::spawn(&mut daemon).unwrap();
    let daemon_pid = child.id();
    let parent_stdin = child.take_stdin().unwrap();
    let generation = wait_for_generation(&mut child, &data_dir);
    let ready = wait_for_status(&mut child, &generation, |status| {
        status["core"]["state"] == "ready"
            && status["capabilities"]["ocr_import"]["state"] == "available"
            && status["capabilities"]["semantic_search"]["state"] == "available"
            && status["capabilities"]["hybrid_search"]["state"] == "available"
    });
    assert_eq!(ready["core"]["state"], "ready", "{ready}");

    for (mode, _) in QUERY_REDLINE_MS {
        let warm_deadline = Instant::now() + Duration::from_secs(60);
        let mut attempt = 0_u32;
        loop {
            let response = search_with_budgets(
                &generation.search_endpoint,
                &generation.token,
                &format!("load-warm-{mode}-{attempt}"),
                mode,
                60_000,
                Duration::from_secs(60),
            );
            if let Ok(response) = response {
                let payload = response_payload(&response);
                if response.starts_with("HTTP/1.1 200") && payload["status"] == "ok" {
                    assert_eq!(payload["partial"], false, "{payload}");
                    break;
                }
            }
            attempt += 1;
            assert!(child.try_wait().unwrap().is_none());
            assert!(
                Instant::now() < warm_deadline,
                "{mode} active generation did not become hot"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    let witness_store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
    let before = witness_store.search_projection_state().unwrap();
    let initial_jobs = witness_store.ingest_jobs().unwrap();
    assert_eq!(initial_jobs.len(), SYNTHETIC_OCR_PUBLICATIONS);
    assert_eq!(
        initial_jobs
            .iter()
            .filter(|job| job.status == IngestJobStatus::Completed)
            .count(),
        0,
        "publication load must start only after the active generation is hot"
    );

    let mut latencies = BTreeMap::<&str, Vec<Duration>>::new();
    let mut peak_rss_kib = 0_u64;
    let mut peak_cpu_percent = 0.0_f64;
    let mut heartbeat_failures = 0_u32;
    let mut consecutive_heartbeat_failures = 0_u32;
    let mut automatic_restart_attempts = 0_u32;
    let mut next_heartbeat = Instant::now();
    let deadline = Instant::now() + WITNESS_TIMEOUT;
    let mut query_index = 0_usize;
    let mut last_visible_epoch = before.visible_epoch;

    loop {
        assert!(
            child.try_wait().unwrap().is_none(),
            "isolated daemon exited during continuous publication load"
        );
        let jobs = witness_store.ingest_jobs().unwrap();
        let completed = jobs
            .iter()
            .filter(|job| job.status == IngestJobStatus::Completed)
            .count();
        assert!(
            jobs.iter().all(|job| {
                matches!(
                    job.status,
                    IngestJobStatus::Queued | IngestJobStatus::Running | IngestJobStatus::Completed
                )
            }),
            "synthetic OCR work entered a failure state"
        );
        let state = witness_store.search_projection_state().unwrap();
        if completed == SYNTHETIC_OCR_PUBLICATIONS
            && state.visible_epoch
                == before.visible_epoch + u64::try_from(SYNTHETIC_OCR_PUBLICATIONS).unwrap()
        {
            break;
        }

        let (mode, _) = QUERY_REDLINE_MS[query_index % QUERY_REDLINE_MS.len()];
        let started = Instant::now();
        let response = search_with_timeout(
            &generation.search_endpoint,
            &generation.token,
            &format!("load-interactive-{query_index}"),
            mode,
        )
        .unwrap();
        let elapsed = started.elapsed();
        let payload = response_payload(&response);
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert_eq!(payload["status"], "ok", "{payload}");
        assert_eq!(payload["partial"], false, "{payload}");
        assert_eq!(payload["partial_reasons"], serde_json::json!([]));
        let visible_epoch = payload["visible_epoch"].as_u64().unwrap();
        assert!(visible_epoch >= last_visible_epoch, "{payload}");
        last_visible_epoch = visible_epoch;
        assert_ranked_unique_results(&payload);
        latencies.entry(mode).or_default().push(elapsed);
        query_index += 1;

        let (rss_kib, cpu_percent) = process_sample(daemon_pid);
        peak_rss_kib = peak_rss_kib.max(rss_kib);
        peak_cpu_percent = peak_cpu_percent.max(cpu_percent);

        if Instant::now() >= next_heartbeat {
            match get_with_timeout(
                &generation.status_endpoint,
                &generation.token,
                HEARTBEAT_TIMEOUT,
            ) {
                Ok(response) => {
                    let status = response_payload(&response);
                    if response.starts_with("HTTP/1.1 200") && status["core"]["state"] == "ready" {
                        consecutive_heartbeat_failures = 0;
                    } else {
                        heartbeat_failures += 1;
                        consecutive_heartbeat_failures += 1;
                    }
                }
                Err(_) => {
                    heartbeat_failures += 1;
                    consecutive_heartbeat_failures += 1;
                }
            }
            if consecutive_heartbeat_failures >= 3 {
                automatic_restart_attempts += 1;
            }
            next_heartbeat = Instant::now() + HEARTBEAT_INTERVAL;
        }
        assert!(
            Instant::now() < deadline,
            "continuous OCR publication witness timed out"
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    for (mode, redline_ms) in QUERY_REDLINE_MS {
        let samples = latencies.get(mode).expect("query mode was sampled");
        assert!(samples.len() >= 3, "{mode} did not receive enough samples");
        let p95 = percentile_95(samples);
        assert!(
            p95.as_millis() < redline_ms,
            "{mode} p95 {p95:?} exceeded {redline_ms}ms under OCR publication load"
        );
    }
    assert!(
        peak_rss_kib <= MAX_DAEMON_RSS_KIB,
        "daemon RSS peak {:.1}MiB exceeded the 512MiB witness redline",
        peak_rss_kib as f64 / 1_024.0
    );
    let cpu_capacity = std::thread::available_parallelism().unwrap().get() as f64 * 100.0;
    assert!(peak_cpu_percent <= cpu_capacity + 10.0);
    assert_eq!(heartbeat_failures, 0);
    assert_eq!(automatic_restart_attempts, 0);
    assert_eq!(child.id(), daemon_pid);
    let current_generation = wait_for_generation(&mut child, &data_dir);
    assert_eq!(current_generation.launch_id, generation.launch_id);
    assert_eq!(current_generation.instance_id, generation.instance_id);

    let p95_fulltext = percentile_95(latencies.get("fulltext").unwrap()).as_millis();
    let p95_semantic = percentile_95(latencies.get("semantic").unwrap()).as_millis();
    let p95_hybrid = percentile_95(latencies.get("hybrid").unwrap()).as_millis();
    eprintln!(
        "synthetic publication-load witness: corpus={}, publications={}, queries={}, p95_ms=fulltext:{p95_fulltext}/semantic:{p95_semantic}/hybrid:{p95_hybrid}, peak_rss_mib={:.1}, peak_cpu_percent={peak_cpu_percent:.1}, supervisor_generation={SYNTHETIC_SUPERVISOR_GENERATION}, heartbeat_failures={heartbeat_failures}, automatic_restart_attempts={automatic_restart_attempts}",
        SYNTHETIC_SEARCHABLE_DOCUMENTS + SYNTHETIC_OCR_PUBLICATIONS,
        SYNTHETIC_OCR_PUBLICATIONS,
        query_index,
        peak_rss_kib as f64 / 1_024.0,
    );

    drop(parent_stdin);
    let (status, stderr) = wait_contained_with_stderr(child);
    assert!(status.success(), "{stderr}");
    remove_dir(&data_dir);
}

fn assert_ranked_unique_results(payload: &serde_json::Value) {
    let order = result_selection_order(payload);
    assert!(!order.is_empty(), "{payload}");
    let unique = order.iter().collect::<BTreeSet<_>>();
    assert_eq!(unique.len(), order.len(), "{payload}");
    for (index, result) in payload["results"].as_array().unwrap().iter().enumerate() {
        assert_eq!(result["rank"], index + 1);
    }
}

fn percentile_95(samples: &[Duration]) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * 95).div_ceil(100).saturating_sub(1);
    sorted[rank]
}

fn process_sample(pid: u32) -> (u64, f64) {
    let output = Command::new("/bin/ps")
        .args(["-o", "rss=,%cpu=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut fields = stdout.split_whitespace();
    let rss_kib = fields.next().unwrap().parse().unwrap();
    let cpu_percent = fields.next().unwrap().parse().unwrap();
    (rss_kib, cpu_percent)
}

fn search_with_timeout(
    endpoint: &str,
    token: &str,
    request_id: &str,
    mode: &str,
) -> io::Result<String> {
    search_with_budgets(
        endpoint,
        token,
        request_id,
        mode,
        1_000,
        Duration::from_secs(2),
    )
}

fn search_with_budgets(
    endpoint: &str,
    token: &str,
    request_id: &str,
    mode: &str,
    deadline_ms: u64,
    socket_timeout: Duration,
) -> io::Result<String> {
    let body = serde_json::json!({
        "schema_version": "resume-ir.ipc-request.v3",
        "request_id": request_id,
        "client_capability": "codex_validation",
        "deadline_ms": deadline_ms,
        "payload": {
            "query": "synthetic witness",
            "mode": mode,
            "top_k": 10
        }
    })
    .to_string();
    request_with_timeout(endpoint, token, "POST", Some(&body), socket_timeout)
}

fn get_with_timeout(endpoint: &str, token: &str, timeout: Duration) -> io::Result<String> {
    request_with_timeout(endpoint, token, "GET", None, timeout)
}

fn request_with_timeout(
    endpoint: &str,
    token: &str,
    method: &str,
    body: Option<&str>,
    timeout: Duration,
) -> io::Result<String> {
    let (address, path) = endpoint
        .strip_prefix("http://")
        .and_then(|endpoint| endpoint.split_once('/'))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid IPC endpoint"))?;
    let address: SocketAddr = address
        .parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid IPC address"))?;
    let mut stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    match body {
        Some(body) => write!(
            stream,
            "{method} /{path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?,
        None => write!(
            stream,
            "{method} /{path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
        )?,
    }
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}
