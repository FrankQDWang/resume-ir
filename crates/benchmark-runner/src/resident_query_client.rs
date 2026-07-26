use std::io::{Read, Write};
use std::net::TcpStream;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::{synthetic_query_workload, BenchmarkError, Result};
use core_domain::{DocumentId, ResumeVersionId};

static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

pub(super) const STAGES: [&str; 7] = [
    "query_parse",
    "prefilter",
    "bm25",
    "ann",
    "fusion",
    "bulk_hydrate",
    "snippet",
];

#[derive(Clone)]
pub(super) struct Observation {
    pub(super) service_ms: f64,
    pub(super) arrival_ms: f64,
    pub(super) stages_ms: [f64; 7],
    pub(super) contract_valid: bool,
    pub(super) hits: usize,
    pub(super) overloaded: bool,
    pub(super) bucket: &'static str,
    pub(super) mode: &'static str,
}

impl Observation {
    pub(super) fn successful(&self) -> bool {
        self.contract_valid && !self.overloaded && self.hits > 0
    }
}

pub(super) fn send_query(
    endpoint: &str,
    token: &str,
    index: usize,
    top_k: usize,
) -> Result<Observation> {
    send_query_at_workload_index(endpoint, token, workload_index(index), top_k)
}

pub(super) fn send_query_at_workload_index(
    endpoint: &str,
    token: &str,
    workload_index: usize,
    top_k: usize,
) -> Result<Observation> {
    let rest = endpoint
        .strip_prefix("http://")
        .ok_or_else(|| BenchmarkError::invalid_config("resident_query_endpoint"))?;
    let (addr, _) = rest
        .split_once('/')
        .ok_or_else(|| BenchmarkError::invalid_config("resident_query_endpoint"))?;
    let query = synthetic_query_workload::query(workload_index);
    let bucket = core_domain::QuerySetSampleShape::from_query(&query).bucket();
    let mode = mode_for_bucket(bucket);
    let filters = if bucket == "field_filter" {
        serde_json::json!({"locations_any":["shanghai"]})
    } else {
        serde_json::json!({})
    };
    let request_id = format!(
        "resident-benchmark-{}-{}",
        std::process::id(),
        NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    );
    let body = serde_json::json!({
        "schema_version": "resume-ir.ipc-request.v3",
        "request_id": request_id,
        "client_capability": "benchmark",
        "deadline_ms": 9_000,
        "payload": {
            "query": query,
            "mode": mode,
            "top_k": top_k,
            "filters": filters,
        },
    })
    .to_string();
    let request = format!(
        "POST /search HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let started = Instant::now();
    let mut stream = TcpStream::connect(addr).map_err(BenchmarkError::io)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(BenchmarkError::io)?;
    stream
        .write_all(request.as_bytes())
        .map_err(BenchmarkError::io)?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(BenchmarkError::io)?;
    let service_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let payload = response
        .split("\r\n\r\n")
        .nth(1)
        .and_then(|body| serde_json::from_str::<serde_json::Value>(body).ok())
        .ok_or_else(|| BenchmarkError::invalid_config("resident_query_response"))?;
    let overloaded = response.starts_with("HTTP/1.1 503 Service Unavailable")
        && valid_correlated_overload(&payload, &request_id);
    if overloaded {
        return Ok(Observation {
            service_ms,
            arrival_ms: service_ms,
            stages_ms: [0.0; 7],
            contract_valid: true,
            hits: 0,
            overloaded: true,
            bucket,
            mode,
        });
    }
    let stages_ms = parse_server_timing(&response)?;
    let response_mode = if mode == "fulltext" { "keyword" } else { mode };
    let results_valid = valid_search_results(&payload);
    let contract_valid = response.starts_with("HTTP/1.1 200 OK")
        && payload["schema_version"] == "resume-ir.search-response.v3"
        && payload["request_id"] == request_id
        && payload["query_mode"] == response_mode
        && payload["search_index"] == "available"
        && results_valid;
    let hits = payload["result_count"].as_u64().unwrap_or(0) as usize;
    Ok(Observation {
        service_ms,
        arrival_ms: service_ms,
        stages_ms,
        contract_valid,
        hits,
        overloaded: false,
        bucket,
        mode,
    })
}

fn valid_correlated_overload(payload: &serde_json::Value, request_id: &str) -> bool {
    let Some(envelope) = payload.as_object() else {
        return false;
    };
    let Some(error) = envelope.get("error").and_then(serde_json::Value::as_object) else {
        return false;
    };

    exact_keys(
        envelope,
        &["schema_version", "request_id", "status", "error"],
    ) && exact_keys(
        error,
        &["code", "action", "retry_after_ms", "capability", "reason"],
    ) && envelope
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        == Some("resume-ir.error.v2")
        && envelope
            .get("request_id")
            .and_then(serde_json::Value::as_str)
            == Some(request_id)
        && envelope.get("status").and_then(serde_json::Value::as_str) == Some("error")
        && error.get("code").and_then(serde_json::Value::as_str) == Some("OVERLOADED")
        && error.get("action").and_then(serde_json::Value::as_str) == Some("retry")
        && error
            .get("retry_after_ms")
            .and_then(serde_json::Value::as_u64)
            == Some(250)
        && error
            .get("capability")
            .is_some_and(serde_json::Value::is_null)
        && error.get("reason").is_some_and(serde_json::Value::is_null)
}

fn exact_keys(object: &serde_json::Map<String, serde_json::Value>, expected: &[&str]) -> bool {
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn valid_search_results(payload: &serde_json::Value) -> bool {
    let Some(results) = payload["results"].as_array() else {
        return false;
    };
    if payload["result_count"].as_u64() != Some(results.len() as u64) {
        return false;
    }
    let Some(visible_epoch) = payload["visible_epoch"].as_u64() else {
        return false;
    };
    results.iter().enumerate().all(|(index, result)| {
        let Some(selection) = result["selection"].as_object() else {
            return false;
        };
        selection.len() == 3
            && result["rank"].as_u64() == Some((index + 1) as u64)
            && selection
                .get("doc_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| DocumentId::from_str(value).is_ok())
            && selection
                .get("version_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| ResumeVersionId::from_str(value).is_ok())
            && selection
                .get("visible_epoch")
                .and_then(serde_json::Value::as_u64)
                == Some(visible_epoch)
    })
}

pub(super) fn workload_index(sequence: usize) -> usize {
    sequence.wrapping_mul(137) % synthetic_query_workload::CYCLE_QUERY_COUNT
}

fn mode_for_bucket(bucket: &str) -> &'static str {
    match bucket {
        "hybrid" => "hybrid",
        "semantic" => "semantic",
        _ => "fulltext",
    }
}

pub(super) fn parse_server_timing(response: &str) -> Result<[f64; 7]> {
    let header = response
        .lines()
        .find_map(|line| line.strip_prefix("Server-Timing: "))
        .ok_or_else(|| BenchmarkError::invalid_config("resident_query_server_timing"))?;
    let mut values = [0.0; 7];
    let observations = header.split(',').collect::<Vec<_>>();
    if observations.len() != STAGES.len() {
        return Err(BenchmarkError::invalid_config(
            "resident_query_server_timing",
        ));
    }
    for (index, (observation, expected)) in observations.iter().zip(STAGES).enumerate() {
        let (stage, value) = observation
            .split_once(";dur=")
            .ok_or_else(|| BenchmarkError::invalid_config("resident_query_server_timing"))?;
        if stage != expected {
            return Err(BenchmarkError::invalid_config(
                "resident_query_server_timing",
            ));
        }
        values[index] = value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .ok_or_else(|| BenchmarkError::invalid_config("resident_query_server_timing"))?;
    }
    Ok(values)
}

pub(super) fn invalid_observation(workload_index: usize) -> Observation {
    let query = synthetic_query_workload::query(workload_index);
    let bucket = core_domain::QuerySetSampleShape::from_query(&query).bucket();
    Observation {
        service_ms: 0.0,
        arrival_ms: 0.0,
        stages_ms: [0.0; 7],
        contract_valid: false,
        hits: 0,
        overloaded: false,
        bucket,
        mode: mode_for_bucket(bucket),
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn resident_client_correlates_v3_request_and_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let body = request.split("\r\n\r\n").nth(1).unwrap();
            let envelope: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(envelope["schema_version"], "resume-ir.ipc-request.v3");
            assert_eq!(envelope["client_capability"], "benchmark");
            assert_eq!(envelope["deadline_ms"], 9_000);
            assert!(envelope["payload"]["query"].as_str().is_some());
            let request_id = envelope["request_id"].as_str().unwrap();
            let response = serde_json::json!({
                "schema_version": "resume-ir.search-response.v3",
                "request_id": request_id,
                "status": "ok",
                "visible_epoch": 1,
                "query_mode": "keyword",
                "partial": false,
                "partial_reasons": [],
                "latency_ms": 1.0,
                "stage_latency_ms": {},
                "search_index": "available",
                "result_count": 1,
                "results": [{
                    "rank": 1,
                    "selection": {
                        "doc_id": format!("doc_{}", "1".repeat(32)),
                        "version_id": format!("ver_{}", "2".repeat(32)),
                        "visible_epoch": 1,
                    },
                    "file_name": "synthetic.txt",
                    "snippet": "synthetic result",
                }],
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nServer-Timing: query_parse;dur=0.1,prefilter;dur=0.1,bm25;dur=0.1,ann;dur=0.0,fusion;dur=0.0,bulk_hydrate;dur=0.1,snippet;dur=0.1\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
        });

        let observation = send_query_at_workload_index(
            &format!("http://{addr}/search"),
            "synthetic-token",
            0,
            10,
        )
        .unwrap();
        server.join().unwrap();
        assert!(observation.successful());
    }

    #[test]
    fn resident_client_accepts_correlated_overload_as_a_bounded_load_outcome() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let body = request.split("\r\n\r\n").nth(1).unwrap();
            let envelope: serde_json::Value = serde_json::from_str(body).unwrap();
            let request_id = envelope["request_id"].as_str().unwrap();
            let response = serde_json::json!({
                "schema_version": "resume-ir.error.v2",
                "request_id": request_id,
                "status": "error",
                "error": {
                    "code": "OVERLOADED",
                    "action": "retry",
                    "retry_after_ms": 250,
                    "capability": null,
                    "reason": null,
                },
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
        });

        let observation = send_query_at_workload_index(
            &format!("http://{addr}/search"),
            "synthetic-token",
            0,
            10,
        )
        .unwrap();
        server.join().unwrap();
        assert!(observation.contract_valid);
        assert!(observation.overloaded);
        assert!(!observation.successful());
        assert_eq!(observation.hits, 0);
    }

    #[test]
    fn overload_contract_rejects_legacy_missing_and_uncorrelated_payloads() {
        let request_id = "resident-benchmark-1-1";
        let valid = serde_json::json!({
            "schema_version": "resume-ir.error.v2",
            "request_id": request_id,
            "status": "error",
            "error": {
                "code": "OVERLOADED",
                "action": "retry",
                "retry_after_ms": 250,
                "capability": null,
                "reason": null,
            },
        });
        assert!(valid_correlated_overload(&valid, request_id));

        let mut legacy = valid.clone();
        legacy["schema_version"] = serde_json::Value::String("resume-ir.error.v1".to_string());
        assert!(!valid_correlated_overload(&legacy, request_id));

        let mut missing_required_null = valid.clone();
        missing_required_null["error"]
            .as_object_mut()
            .unwrap()
            .remove("capability");
        assert!(!valid_correlated_overload(
            &missing_required_null,
            request_id
        ));

        let mut wrong_request = valid;
        wrong_request["request_id"] = serde_json::Value::String("other-request".to_string());
        assert!(!valid_correlated_overload(&wrong_request, request_id));

        let mut unknown = wrong_request;
        unknown["error"]["private_debug"] = serde_json::Value::Bool(true);
        assert!(!valid_correlated_overload(&unknown, "other-request"));
    }

    fn read_request(stream: &mut TcpStream) -> String {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 512];
        let body_end = loop {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0);
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|position| position + 4)
            else {
                continue;
            };
            let headers = std::str::from_utf8(&request[..header_end]).unwrap();
            let content_length = headers
                .lines()
                .find_map(|line| line.strip_prefix("Content-Length: "))
                .unwrap()
                .parse::<usize>()
                .unwrap();
            break header_end + content_length;
        };
        while request.len() < body_end {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0);
            request.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8(request).unwrap()
    }
}
