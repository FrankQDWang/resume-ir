use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use super::{synthetic_query_workload, BenchmarkError, Result};
use crate::resident_query_client::{
    invalid_observation, send_query, send_query_at_workload_index, workload_index, Observation,
    STAGES,
};
use crate::resident_query_fixture::{
    hardware_profile, prepare_fixture, ResidentDaemon, RssSampler,
};

const REPORT_SCHEMA: &str = "resume-ir.resident-query-load.v1";
const HISTOGRAM_BOUNDS_MS: [f64; 10] = [
    1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 5_000.0,
];
const CAPACITY_FRACTIONS: [f64; 4] = [0.3, 0.7, 1.0, 1.2];
const STABLE_ARRIVAL_P95_MS_MAX: f64 = 1_500.0;
const QUERY_BUCKETS: [&str; 7] = [
    "single_term",
    "and_2",
    "and_3_5",
    "and_6_16",
    "field_filter",
    "hybrid",
    "semantic",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResidentQueryLoadConfig {
    warmup_seconds: u64,
    point_seconds: u64,
    repetitions: usize,
    top_k: usize,
    smoke: bool,
}

impl ResidentQueryLoadConfig {
    pub fn production() -> Self {
        Self {
            warmup_seconds: 30,
            point_seconds: 5,
            repetitions: 5,
            top_k: 10,
            smoke: false,
        }
    }

    pub fn smoke() -> Self {
        Self {
            warmup_seconds: 1,
            point_seconds: 1,
            repetitions: 1,
            top_k: 10,
            smoke: true,
        }
    }

    pub fn validate(self) -> Result<Self> {
        let strict = self.warmup_seconds >= 30 && self.point_seconds >= 5 && self.repetitions >= 5;
        if self.top_k == 0 || self.top_k > 100 || (!self.smoke && !strict) {
            return Err(BenchmarkError::invalid_config("resident_query_load"));
        }
        Ok(self)
    }
}

pub struct ResidentQueryLoadReport {
    value: serde_json::Value,
}

impl ResidentQueryLoadReport {
    pub fn to_redacted_json(&self) -> String {
        self.value.to_string()
    }
}

pub fn run_resident_public_query_load(
    data_dir: &Path,
    daemon_command: &Path,
    embedding_command: &Path,
    config: ResidentQueryLoadConfig,
) -> Result<ResidentQueryLoadReport> {
    let config = config.validate()?;
    prepare_fixture(data_dir)?;
    let daemon = ResidentDaemon::start(data_dir, daemon_command, embedding_command)?;
    let endpoint = daemon.endpoint();
    let token = fs::read_to_string(data_dir.join("ipc.auth")).map_err(BenchmarkError::io)?;
    for index in [0_usize, 50, 125, 275, 325, 400, 475] {
        if !send_query_at_workload_index(endpoint, token.trim(), index, config.top_k)?.successful()
        {
            return Err(BenchmarkError::invalid_config(
                "resident_query_bucket_preflight",
            ));
        }
    }
    let sampler = RssSampler::start(daemon.pid());

    let (warmup, _) = run_closed_loop(
        endpoint,
        token.trim(),
        1,
        config.warmup_seconds,
        config.top_k,
        0,
    );
    let mut closed_loop = Vec::new();
    let mut closed_sequence_start = 0_usize;
    for concurrency in [1_usize, 2, 4, 8] {
        let mut observations = Vec::new();
        let mut elapsed_seconds = 0.0;
        for _ in 0..config.repetitions {
            let (mut run, elapsed) = run_closed_loop(
                endpoint,
                token.trim(),
                concurrency,
                config.point_seconds,
                config.top_k,
                closed_sequence_start,
            );
            closed_sequence_start += run.len();
            observations.append(&mut run);
            elapsed_seconds += elapsed.as_secs_f64();
        }
        closed_loop.push((concurrency, elapsed_seconds, observations));
    }
    let capacity_qps = closed_loop
        .iter()
        .map(|(_, elapsed_seconds, observations)| {
            successful_count(observations) as f64 / elapsed_seconds.max(0.001)
        })
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let mut open_loop = Vec::new();
    for fraction in CAPACITY_FRACTIONS {
        let target_qps = capacity_qps * fraction;
        let mut observations = Vec::new();
        let mut elapsed_seconds = 0.0;
        let requests_per_repetition = (target_qps * config.point_seconds as f64).ceil() as usize;
        for repetition in 0..config.repetitions {
            let (mut run, elapsed) = run_open_loop(
                endpoint,
                token.trim(),
                target_qps,
                config.point_seconds,
                config.top_k,
                repetition * requests_per_repetition,
            );
            observations.append(&mut run);
            elapsed_seconds += elapsed.as_secs_f64();
        }
        open_loop.push((fraction, target_qps, elapsed_seconds, observations));
    }

    let resource_sample = sampler.finish();
    drop(daemon);

    let result_consistent = warmup.iter().all(|observation| observation.contract_valid)
        && closed_loop
            .iter()
            .flat_map(|(_, _, observations)| observations)
            .all(|observation| observation.contract_valid)
        && open_loop
            .iter()
            .flat_map(|(_, _, _, observations)| observations)
            .all(|observation| observation.contract_valid);
    if !result_consistent {
        return Err(BenchmarkError::invalid_config(
            "resident_query_response_contract",
        ));
    }

    let stable_capacity_qps = open_loop
        .iter()
        .filter_map(|(_, target_qps, elapsed_seconds, observations)| {
            let achieved_qps = successful_count(observations) as f64 / elapsed_seconds.max(0.001);
            let arrival_p95_ms = percentile(&arrival_latencies(observations), 0.95);
            (overload_count(observations) == 0
                && achieved_qps >= target_qps * 0.95
                && arrival_p95_ms <= STABLE_ARRIVAL_P95_MS_MAX)
                .then_some(*target_qps)
        })
        .max_by(f64::total_cmp);

    let closed_json = closed_loop
        .iter()
        .map(|(concurrency, elapsed_seconds, observations)| {
            serde_json::json!({
                "concurrency": concurrency,
                "qps": successful_count(observations) as f64 / elapsed_seconds.max(0.001),
                "request_count": observations.len(),
                "successful_count": successful_count(observations),
                "overload_count": overload_count(observations),
                "executed_bucket_counts": bucket_counts_json(observations),
                "executed_mode_counts": mode_counts_json(observations),
                "latency_ms": latency_json(observations, false),
            })
        })
        .collect::<Vec<_>>();
    let open_json = open_loop
        .iter()
        .map(|(fraction, target_qps, elapsed_seconds, observations)| {
            serde_json::json!({
                "capacity_fraction": fraction,
                "target_qps": target_qps,
                "achieved_qps": successful_count(observations) as f64 / elapsed_seconds.max(0.001),
                "request_count": observations.len(),
                "successful_count": successful_count(observations),
                "overload_count": overload_count(observations),
                "overload_response_latency_ms": overload_latency_json(observations),
                "executed_bucket_counts": bucket_counts_json(observations),
                "executed_mode_counts": mode_counts_json(observations),
                "arrival_latency_ms": latency_json(observations, true),
                "service_latency_ms": latency_json(observations, false),
                "arrival_latency_by_bucket_ms": latency_by_bucket_json(observations, true),
                "service_latency_by_bucket_ms": latency_by_bucket_json(observations, false),
                "unattributed_latency_ms": unattributed_latency_json(observations),
                "stage_latency_ms": stage_latency_json(observations),
                "stage_histogram_ms": stage_histogram_json(observations),
            })
        })
        .collect::<Vec<_>>();

    let (hardware_tier, resource_budget_mb) = hardware_profile();
    let daemon_rss_peak_mb = resource_sample.daemon_rss_peak_bytes as f64 / 1_048_576.0;
    Ok(ResidentQueryLoadReport {
        value: serde_json::json!({
            "schema_version": REPORT_SCHEMA,
            "target_claim": if config.smoke { "harness_smoke" } else { "initial_usable_baseline_observed" },
            "evidence_lane": "smoke",
            "benchmark_lane": "query_hot_path",
            "workload": serde_json::from_str::<serde_json::Value>(&synthetic_query_workload::redacted_contract_json()).expect("public workload contract is JSON"),
            "document_count": synthetic_query_workload::CANONICAL_DOCUMENT_COUNT,
            "vector_document_count": synthetic_query_workload::CANONICAL_DOCUMENT_COUNT,
            "top_k": config.top_k,
            "warmup_seconds": config.warmup_seconds,
            "repetitions": config.repetitions,
            "point_seconds": config.point_seconds,
            "connection_reuse": false,
            "persistent_connection": false,
            "coordinated_omission_correction": "scheduled_start_to_completion",
            "capacity_qps": capacity_qps,
            "stable_capacity_qps": stable_capacity_qps,
            "stable_capacity_rule": {
                "achieved_to_target_ratio_min": 0.95,
                "arrival_p95_ms_max": STABLE_ARRIVAL_P95_MS_MAX,
                "response_contract_must_be_consistent": true,
                "overload_count_must_be_zero": true,
            },
            "bucket_preflight": {
                "all_buckets_validated": true,
                "executed_bucket_counts": {
                    "single_term": 1,
                    "and_2": 1,
                    "and_3_5": 1,
                    "and_6_16": 1,
                    "field_filter": 1,
                    "hybrid": 1,
                    "semantic": 1,
                },
                "executed_mode_counts": {"fulltext": 5, "hybrid": 1, "semantic": 1},
            },
            "closed_loop": closed_json,
            "open_loop": open_json,
            "resources": {
                "hardware_tier": hardware_tier,
                "private_or_anonymous_budget_mb": resource_budget_mb,
                "daemon_rss_peak_mb": daemon_rss_peak_mb,
                "host_cpu_mean_pct": resource_sample.host_cpu_mean_pct,
                "host_cpu_peak_pct": resource_sample.host_cpu_peak_pct,
                "daemon_cpu_peak_pct": resource_sample.daemon_cpu_peak_pct,
                "resource_budget_exceeded": daemon_rss_peak_mb > resource_budget_mb as f64,
            },
            "hot_path": {
                "ocr": false,
                "parsing": false,
                "heavy_model_inference": false,
                "daemon_resident": true,
                "outer_spawn_per_query": false,
                "semantic_embedding_command_spawn_per_query": true,
            },
            "result_contract_consistent": true,
            "privacy": {
                "contains_raw_resume_text": false,
                "contains_raw_query_text": false,
                "contains_candidate_results": false,
                "contains_local_paths": false,
                "contains_tokens": false,
                "contains_diagnostics_package": false,
            }
        }),
    })
}

fn run_closed_loop(
    endpoint: &str,
    token: &str,
    concurrency: usize,
    seconds: u64,
    top_k: usize,
    sequence_start: usize,
) -> (Vec<Observation>, Duration) {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let sequence = AtomicUsize::new(0);
    let observations = thread::scope(|scope| {
        let mut workers = Vec::new();
        for _ in 0..concurrency {
            workers.push(scope.spawn(|| {
                let mut output = Vec::new();
                while Instant::now() < deadline {
                    let index = sequence.fetch_add(1, Ordering::Relaxed);
                    let sequence_index = sequence_start + index;
                    if let Ok(observation) = send_query(endpoint, token, sequence_index, top_k) {
                        output.push(observation);
                    } else {
                        output.push(invalid_observation(workload_index(sequence_index)));
                    }
                }
                output
            }));
        }
        workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap_or_default())
            .collect()
    });
    (observations, started.elapsed())
}

fn run_open_loop(
    endpoint: &str,
    token: &str,
    target_qps: f64,
    seconds: u64,
    top_k: usize,
    sequence_start: usize,
) -> (Vec<Observation>, Duration) {
    let total = (target_qps * seconds as f64).ceil().clamp(1.0, 100_000.0) as usize;
    let sequence = AtomicUsize::new(0);
    let started = Instant::now();
    let output = Mutex::new(Vec::with_capacity(total));
    thread::scope(|scope| {
        for _ in 0..total.min(64) {
            scope.spawn(|| loop {
                let index = sequence.fetch_add(1, Ordering::Relaxed);
                if index >= total {
                    break;
                }
                let scheduled = Duration::from_secs_f64(index as f64 / target_qps);
                if let Some(wait) = scheduled.checked_sub(started.elapsed()) {
                    thread::sleep(wait);
                }
                let sequence_index = sequence_start + index;
                let mut observation = send_query(endpoint, token, sequence_index, top_k)
                    .unwrap_or_else(|_| invalid_observation(workload_index(sequence_index)));
                observation.arrival_ms =
                    started.elapsed().saturating_sub(scheduled).as_secs_f64() * 1_000.0;
                output.lock().expect("load output lock").push(observation);
            });
        }
    });
    (
        output.into_inner().expect("load output lock"),
        started.elapsed(),
    )
}

fn latency_json(observations: &[Observation], arrival: bool) -> serde_json::Value {
    let mut values = if arrival {
        arrival_latencies(observations)
    } else {
        observations
            .iter()
            .filter(|observation| observation.successful())
            .map(|observation| observation.service_ms)
            .collect::<Vec<_>>()
    };
    values.sort_by(f64::total_cmp);
    serde_json::json!({"p50":percentile(&values,0.50),"p95":percentile(&values,0.95),"p99":percentile(&values,0.99),"max":values.last().copied().unwrap_or(0.0)})
}

fn arrival_latencies(observations: &[Observation]) -> Vec<f64> {
    let mut values = observations
        .iter()
        .filter(|observation| observation.successful())
        .map(|observation| observation.arrival_ms)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values
}

fn latency_by_bucket_json(observations: &[Observation], arrival: bool) -> serde_json::Value {
    serde_json::Value::Object(
        QUERY_BUCKETS
            .iter()
            .filter_map(|bucket| {
                let mut values = observations
                    .iter()
                    .filter(|observation| observation.successful() && observation.bucket == *bucket)
                    .map(|observation| {
                        if arrival {
                            observation.arrival_ms
                        } else {
                            observation.service_ms
                        }
                    })
                    .collect::<Vec<_>>();
                if values.is_empty() {
                    return None;
                }
                values.sort_by(f64::total_cmp);
                Some((
                    (*bucket).to_string(),
                    serde_json::json!({
                        "request_count": values.len(),
                        "latency_ms": latency_summary_json(&values),
                    }),
                ))
            })
            .collect(),
    )
}

fn bucket_counts_json(observations: &[Observation]) -> serde_json::Value {
    serde_json::json!({
        "single_term": count_bucket(observations, "single_term"),
        "and_2": count_bucket(observations, "and_2"),
        "and_3_5": count_bucket(observations, "and_3_5"),
        "and_6_16": count_bucket(observations, "and_6_16"),
        "field_filter": count_bucket(observations, "field_filter"),
        "hybrid": count_bucket(observations, "hybrid"),
        "semantic": count_bucket(observations, "semantic"),
    })
}

fn count_bucket(observations: &[Observation], bucket: &str) -> usize {
    observations
        .iter()
        .filter(|observation| observation.successful() && observation.bucket == bucket)
        .count()
}

fn mode_counts_json(observations: &[Observation]) -> serde_json::Value {
    serde_json::json!({
        "fulltext": observations.iter().filter(|observation| observation.successful() && observation.mode == "fulltext").count(),
        "hybrid": observations.iter().filter(|observation| observation.successful() && observation.mode == "hybrid").count(),
        "semantic": observations.iter().filter(|observation| observation.successful() && observation.mode == "semantic").count(),
    })
}

fn unattributed_latency_json(observations: &[Observation]) -> serde_json::Value {
    let mut values = observations
        .iter()
        .filter(|observation| observation.successful())
        .map(|observation| {
            (observation.service_ms - observation.stages_ms.iter().sum::<f64>()).max(0.0)
        })
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    latency_summary_json(&values)
}

fn stage_latency_json(observations: &[Observation]) -> serde_json::Value {
    serde_json::Value::Object(
        STAGES
            .iter()
            .enumerate()
            .map(|(stage_index, stage)| {
                let mut values = observations
                    .iter()
                    .filter(|observation| observation.successful())
                    .map(|observation| observation.stages_ms[stage_index])
                    .collect::<Vec<_>>();
                values.sort_by(f64::total_cmp);
                ((*stage).to_string(), latency_summary_json(&values))
            })
            .collect(),
    )
}

fn latency_summary_json(values: &[f64]) -> serde_json::Value {
    serde_json::json!({
        "p50": percentile(values, 0.50),
        "p95": percentile(values, 0.95),
        "p99": percentile(values, 0.99),
        "max": values.last().copied().unwrap_or(0.0),
    })
}

fn stage_histogram_json(observations: &[Observation]) -> serde_json::Value {
    serde_json::Value::Object(STAGES.iter().enumerate().map(|(stage_index, stage)| {
        let mut counts = vec![0_usize; HISTOGRAM_BOUNDS_MS.len()];
        let mut overflow = 0_usize;
        for value in observations.iter().filter(|observation| observation.successful()).map(|observation| observation.stages_ms[stage_index]) {
            if let Some(index) = HISTOGRAM_BOUNDS_MS.iter().position(|bound| value <= *bound) { counts[index] += 1; } else { overflow += 1; }
        }
        ((*stage).to_string(), serde_json::json!({"upper_bounds_ms":HISTOGRAM_BOUNDS_MS,"counts":counts,"overflow_count":overflow}))
    }).collect())
}

fn successful_count(observations: &[Observation]) -> usize {
    observations
        .iter()
        .filter(|observation| observation.successful())
        .count()
}

fn overload_count(observations: &[Observation]) -> usize {
    observations
        .iter()
        .filter(|observation| observation.overloaded)
        .count()
}

fn overload_latency_json(observations: &[Observation]) -> serde_json::Value {
    let mut values = observations
        .iter()
        .filter(|observation| observation.overloaded)
        .map(|observation| observation.service_ms)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    latency_summary_json(&values)
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let index = ((values.len() as f64 * percentile).ceil() as usize)
        .saturating_sub(1)
        .min(values.len() - 1);
    values[index]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::resident_query_client::parse_server_timing;

    #[test]
    fn strict_and_smoke_methodology_are_distinct() {
        assert!(ResidentQueryLoadConfig::production().validate().is_ok());
        assert!(ResidentQueryLoadConfig::smoke().validate().is_ok());

        let mut invalid = ResidentQueryLoadConfig::production();
        invalid.warmup_seconds = 29;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn fixed_schedule_is_a_full_query_cycle_permutation() {
        let observed = (0..synthetic_query_workload::CYCLE_QUERY_COUNT)
            .map(workload_index)
            .collect::<BTreeSet<_>>();
        let expected = (0..synthetic_query_workload::CYCLE_QUERY_COUNT).collect();

        assert_eq!(observed, expected);
    }

    #[test]
    fn server_timing_parser_requires_all_ordered_stages() {
        let response = concat!(
            "HTTP/1.1 200 OK\r\n",
            "Server-Timing: query_parse;dur=1.000,prefilter;dur=2.000,",
            "bm25;dur=3.000,ann;dur=4.000,fusion;dur=5.000,",
            "bulk_hydrate;dur=6.000,snippet;dur=7.000\r\n\r\n{}"
        );

        assert_eq!(
            parse_server_timing(response).unwrap(),
            [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]
        );
        assert!(parse_server_timing("HTTP/1.1 200 OK\r\n\r\n{}").is_err());
    }
}
