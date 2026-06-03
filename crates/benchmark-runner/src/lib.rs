pub fn crate_name() -> &'static str {
    "benchmark-runner"
}

use std::fmt;
use std::fs;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use index_fulltext::{FullTextIndex, IndexDocument, IndexSection, SearchQuery};

const DEFAULT_TOP_K: usize = 10;
const MAX_TOP_K: usize = 100;

pub type Result<T> = std::result::Result<T, BenchmarkError>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SyntheticBenchmarkConfig {
    document_count: usize,
    query_count: usize,
    top_k: usize,
}

impl SyntheticBenchmarkConfig {
    pub fn new(document_count: usize, query_count: usize) -> Result<Self> {
        if document_count == 0 {
            return Err(BenchmarkError::invalid_config("document_count"));
        }
        if query_count == 0 {
            return Err(BenchmarkError::invalid_config("query_count"));
        }

        Ok(Self {
            document_count,
            query_count,
            top_k: DEFAULT_TOP_K,
        })
    }

    pub fn with_top_k(mut self, top_k: usize) -> Self {
        self.top_k = top_k.clamp(1, MAX_TOP_K);
        self
    }

    pub fn document_count(self) -> usize {
        self.document_count
    }

    pub fn query_count(self) -> usize {
        self.query_count
    }

    pub fn top_k(self) -> usize {
        self.top_k
    }
}

impl fmt::Debug for SyntheticBenchmarkConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyntheticBenchmarkConfig")
            .field("document_count", &self.document_count)
            .field("query_count", &self.query_count)
            .field("top_k", &self.top_k)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct BenchmarkReport {
    run_id: String,
    platform: String,
    dataset_kind: &'static str,
    document_count: usize,
    query_count: usize,
    top_k: usize,
    build_ms: f64,
    query_total_ms: f64,
    index_size_bytes: u64,
    zero_result_queries: usize,
    total_hits: usize,
    latency: LatencySummary,
    million_scale_verified: bool,
    percentile_confidence: &'static str,
    target_claim: &'static str,
}

impl BenchmarkReport {
    pub fn dataset_kind(&self) -> &'static str {
        self.dataset_kind
    }

    pub fn document_count(&self) -> usize {
        self.document_count
    }

    pub fn query_count(&self) -> usize {
        self.query_count
    }

    pub fn top_k(&self) -> usize {
        self.top_k
    }

    pub fn qps(&self) -> f64 {
        if self.query_total_ms <= 0.0 {
            return 0.0;
        }

        self.query_count as f64 / (self.query_total_ms / 1000.0)
    }

    pub fn index_size_bytes(&self) -> u64 {
        self.index_size_bytes
    }

    pub fn latency(&self) -> &LatencySummary {
        &self.latency
    }

    pub fn million_scale_verified(&self) -> bool {
        self.million_scale_verified
    }

    pub fn percentile_confidence(&self) -> &'static str {
        self.percentile_confidence
    }

    pub fn to_redacted_json(&self) -> String {
        format!(
            concat!(
                "{{",
                "\"schema_version\":\"benchmark.v1\",",
                "\"run_id\":\"{}\",",
                "\"platform\":\"{}\",",
                "\"dataset_kind\":\"{}\",",
                "\"document_count\":{},",
                "\"query_count\":{},",
                "\"top_k\":{},",
                "\"build_ms\":{},",
                "\"query_total_ms\":{},",
                "\"qps\":{},",
                "\"index_size_bytes\":{},",
                "\"query_latency_ms\":{{",
                "\"samples\":{},",
                "\"min\":{},",
                "\"mean\":{},",
                "\"p50\":{},",
                "\"p95\":{},",
                "\"p99\":{},",
                "\"max\":{}",
                "}},",
                "\"zero_result_queries\":{},",
                "\"total_hits\":{},",
                "\"million_scale_verified\":{},",
                "\"percentile_confidence\":\"{}\",",
                "\"target_claim\":\"{}\",",
                "\"scope\":\"synthetic query benchmark; no raw resume text, paths, or queries included\"",
                "}}"
            ),
            self.run_id,
            self.platform,
            self.dataset_kind,
            self.document_count,
            self.query_count,
            self.top_k,
            format_ms(self.build_ms),
            format_ms(self.query_total_ms),
            format_ms(self.qps()),
            self.index_size_bytes,
            self.latency.samples,
            format_ms(self.latency.min_ms),
            format_ms(self.latency.mean_ms),
            format_ms(self.latency.p50_ms),
            format_ms(self.latency.p95_ms),
            format_ms(self.latency.p99_ms),
            format_ms(self.latency.max_ms),
            self.zero_result_queries,
            self.total_hits,
            self.million_scale_verified,
            self.percentile_confidence,
            self.target_claim,
        )
    }
}

impl fmt::Debug for BenchmarkReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BenchmarkReport")
            .field("run_id", &self.run_id)
            .field("platform", &self.platform)
            .field("dataset_kind", &self.dataset_kind)
            .field("document_count", &self.document_count)
            .field("query_count", &self.query_count)
            .field("top_k", &self.top_k)
            .field("build_ms", &self.build_ms)
            .field("query_total_ms", &self.query_total_ms)
            .field("index_size_bytes", &self.index_size_bytes)
            .field("zero_result_queries", &self.zero_result_queries)
            .field("total_hits", &self.total_hits)
            .field("latency", &self.latency)
            .field("million_scale_verified", &self.million_scale_verified)
            .field("percentile_confidence", &self.percentile_confidence)
            .field("target_claim", &self.target_claim)
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct LatencySummary {
    samples: usize,
    min_ms: f64,
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    max_ms: f64,
}

impl LatencySummary {
    pub fn samples(&self) -> usize {
        self.samples
    }

    pub fn p50_ms(&self) -> f64 {
        self.p50_ms
    }

    pub fn p95_ms(&self) -> f64 {
        self.p95_ms
    }
}

impl fmt::Debug for LatencySummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LatencySummary")
            .field("samples", &self.samples)
            .field("min_ms", &self.min_ms)
            .field("mean_ms", &self.mean_ms)
            .field("p50_ms", &self.p50_ms)
            .field("p95_ms", &self.p95_ms)
            .field("p99_ms", &self.p99_ms)
            .field("max_ms", &self.max_ms)
            .finish()
    }
}

pub fn run_synthetic_query_benchmark(
    index_dir: &Path,
    config: SyntheticBenchmarkConfig,
) -> Result<BenchmarkReport> {
    let build_started = Instant::now();
    let index = FullTextIndex::open_or_create(index_dir).map_err(BenchmarkError::fulltext)?;
    let documents = (0..config.document_count)
        .map(synthetic_document)
        .collect::<Vec<_>>();
    index
        .replace_documents(documents)
        .map_err(BenchmarkError::fulltext)?;
    index.commit().map_err(BenchmarkError::fulltext)?;
    index.reload().map_err(BenchmarkError::fulltext)?;
    let build_ms = elapsed_ms(build_started);

    let mut latencies = Vec::with_capacity(config.query_count);
    let query_batch_started = Instant::now();
    let mut total_hits = 0_usize;
    let mut zero_result_queries = 0_usize;
    for index_offset in 0..config.query_count {
        let query_started = Instant::now();
        let hits = index
            .search(SearchQuery::new(synthetic_query(index_offset)).with_limit(config.top_k))
            .map_err(BenchmarkError::fulltext)?;
        latencies.push(elapsed_ms(query_started));
        if hits.is_empty() {
            zero_result_queries += 1;
        }
        total_hits += hits.len();
    }
    let query_total_ms = elapsed_ms(query_batch_started);

    Ok(BenchmarkReport {
        run_id: generate_run_id(),
        platform: platform_label(),
        dataset_kind: "synthetic",
        document_count: config.document_count,
        query_count: config.query_count,
        top_k: config.top_k,
        build_ms,
        query_total_ms,
        index_size_bytes: directory_size_bytes(index_dir)?,
        zero_result_queries,
        total_hits,
        latency: LatencySummary::from_samples(latencies)?,
        million_scale_verified: config.document_count >= 1_000_000,
        percentile_confidence: percentile_confidence(config.query_count),
        target_claim: "not_evaluated",
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BenchmarkGateConfig {
    min_documents: usize,
    min_queries: usize,
    max_p95_ms: f64,
    max_zero_result_queries: usize,
    allow_synthetic: bool,
}

impl BenchmarkGateConfig {
    pub fn new(min_documents: usize, min_queries: usize, max_p95_ms: f64) -> Self {
        Self {
            min_documents,
            min_queries,
            max_p95_ms,
            max_zero_result_queries: 0,
            allow_synthetic: false,
        }
    }

    pub fn allow_synthetic(mut self) -> Self {
        self.allow_synthetic = true;
        self
    }

    pub fn with_max_zero_result_queries(mut self, max_zero_result_queries: usize) -> Self {
        self.max_zero_result_queries = max_zero_result_queries;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BenchmarkGateEvaluation {
    dataset_kind: String,
    document_count: usize,
    query_count: usize,
    p95_ms: f64,
}

impl BenchmarkGateEvaluation {
    pub fn dataset_kind(&self) -> &str {
        &self.dataset_kind
    }

    pub fn document_count(&self) -> usize {
        self.document_count
    }

    pub fn query_count(&self) -> usize {
        self.query_count
    }

    pub fn p95_ms(&self) -> f64 {
        self.p95_ms
    }
}

pub fn evaluate_benchmark_gate_json(
    report_json: &str,
    config: BenchmarkGateConfig,
) -> std::result::Result<BenchmarkGateEvaluation, BenchmarkGateError> {
    let report: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| BenchmarkGateError::invalid_json())?;

    let schema_version = required_str(&report, "schema_version")?;
    if schema_version != "benchmark.v1" {
        return Err(BenchmarkGateError::failed("unsupported benchmark schema"));
    }

    let dataset_kind = required_str(&report, "dataset_kind")?;
    let document_count = required_usize(&report, "document_count")?;
    let query_count = required_usize(&report, "query_count")?;
    let latency = report
        .get("query_latency_ms")
        .ok_or_else(|| BenchmarkGateError::missing_field("query_latency_ms"))?;
    let samples = required_usize(latency, "samples")?;
    let p95_ms = required_f64(latency, "p95")?;
    let zero_result_queries = required_usize(&report, "zero_result_queries")?;
    let million_scale_verified = required_bool(&report, "million_scale_verified")?;
    let target_claim = required_str(&report, "target_claim")?;

    if dataset_kind == "synthetic" && !config.allow_synthetic {
        return Err(BenchmarkGateError::failed(
            "synthetic benchmark requires explicit allowance",
        ));
    }
    if document_count < config.min_documents {
        return Err(BenchmarkGateError::failed(
            "document count below gate minimum",
        ));
    }
    if query_count < config.min_queries || samples < config.min_queries {
        return Err(BenchmarkGateError::failed(
            "query sample count below gate minimum",
        ));
    }
    if p95_ms > config.max_p95_ms {
        return Err(BenchmarkGateError::failed("query p95 exceeded threshold"));
    }
    if zero_result_queries > config.max_zero_result_queries {
        return Err(BenchmarkGateError::failed(
            "zero-result query count exceeded threshold",
        ));
    }
    if million_scale_verified && (dataset_kind == "synthetic" || document_count < 1_000_000) {
        return Err(BenchmarkGateError::failed(
            "million-scale claim is not proven",
        ));
    }
    if target_claim != "not_evaluated" && (dataset_kind == "synthetic" || !million_scale_verified) {
        return Err(BenchmarkGateError::failed("target claim is not proven"));
    }

    Ok(BenchmarkGateEvaluation {
        dataset_kind: dataset_kind.to_string(),
        document_count,
        query_count,
        p95_ms,
    })
}

fn required_str<'a>(
    value: &'a serde_json::Value,
    field: &'static str,
) -> std::result::Result<&'a str, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| BenchmarkGateError::missing_field(field))
}

fn required_usize(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<usize, BenchmarkGateError> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| BenchmarkGateError::missing_field(field))?;
    usize::try_from(number).map_err(|_| BenchmarkGateError::failed("numeric field is too large"))
}

fn required_f64(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<f64, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| BenchmarkGateError::missing_field(field))
}

fn required_bool(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<bool, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| BenchmarkGateError::missing_field(field))
}

impl LatencySummary {
    fn from_samples(mut samples: Vec<f64>) -> Result<Self> {
        if samples.is_empty() {
            return Err(BenchmarkError::invalid_config("latency_samples"));
        }
        samples.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
        let mean_ms = samples.iter().sum::<f64>() / samples.len() as f64;

        Ok(Self {
            samples: samples.len(),
            min_ms: samples[0],
            mean_ms,
            p50_ms: percentile(&samples, 50.0),
            p95_ms: percentile(&samples, 95.0),
            p99_ms: percentile(&samples, 99.0),
            max_ms: samples[samples.len() - 1],
        })
    }
}

fn percentile(sorted_samples: &[f64], percentile: f64) -> f64 {
    let rank = ((percentile / 100.0) * sorted_samples.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted_samples.len() - 1);
    sorted_samples[index]
}

fn synthetic_document(index: usize) -> IndexDocument {
    let skill = ["Java", "Rust", "Python", "Spring Cloud", "Kubernetes"][index % 5];
    let domain = [
        "payment gateway",
        "local search",
        "risk platform",
        "data governance",
        "index operations",
    ][index % 5];
    let degree = ["Bachelor", "Master", "Bachelor", "Associate"][index % 4];
    let clean_text = format!(
        "Synthetic Candidate {index}\nEducation\nSynthetic University\n{degree} of Computer Science\nExperience\nBuilt {skill} services for {domain} and resume retrieval.\n2020.01 - 2024.03\nSkills: {skill}, SQLite, Tantivy"
    );

    IndexDocument {
        doc_id: format!("bench_doc_{index:08}"),
        version_id: format!("bench_ver_{index:08}"),
        file_name: format!("synthetic-benchmark-{index:08}.pdf"),
        clean_text: clean_text.clone(),
        sections: vec![
            IndexSection {
                section_type: "experience".to_string(),
                text: clean_text,
            },
            IndexSection {
                section_type: "skill".to_string(),
                text: format!("Skills: {skill}, SQLite, Tantivy"),
            },
        ],
        is_deleted: false,
    }
}

fn synthetic_query(index: usize) -> &'static str {
    [
        "Java payment gateway",
        "Rust local search",
        "Python data governance",
        "Kubernetes platform",
        "Spring Cloud indexing",
    ][index % 5]
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn generate_run_id() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("bench_{millis}_{}", std::process::id())
}

fn platform_label() -> String {
    format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn percentile_confidence(query_count: usize) -> &'static str {
    if query_count < 100 {
        "smoke"
    } else {
        "sampled"
    }
}

fn directory_size_bytes(path: &Path) -> Result<u64> {
    let mut total = 0_u64;
    if !path.exists() {
        return Ok(0);
    }

    let entries = fs::read_dir(path).map_err(BenchmarkError::io)?;
    for entry in entries {
        let entry = entry.map_err(BenchmarkError::io)?;
        let metadata = entry.metadata().map_err(BenchmarkError::io)?;
        if metadata.is_dir() {
            total = total.saturating_add(directory_size_bytes(&entry.path())?);
        } else {
            total = total.saturating_add(metadata.len());
        }
    }

    Ok(total)
}

fn format_ms(value: f64) -> String {
    format!("{value:.3}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchmarkError {
    kind: BenchmarkErrorKind,
}

impl BenchmarkError {
    fn invalid_config(field: &'static str) -> Self {
        Self {
            kind: BenchmarkErrorKind::InvalidConfig { field },
        }
    }

    fn fulltext(_error: index_fulltext::FullTextError) -> Self {
        Self {
            kind: BenchmarkErrorKind::FullText,
        }
    }

    fn io(_error: std::io::Error) -> Self {
        Self {
            kind: BenchmarkErrorKind::Io,
        }
    }
}

impl fmt::Display for BenchmarkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            BenchmarkErrorKind::InvalidConfig { field } => {
                write!(formatter, "benchmark configuration is invalid for {field}")
            }
            BenchmarkErrorKind::FullText => formatter.write_str("benchmark full-text index failed"),
            BenchmarkErrorKind::Io => formatter.write_str("benchmark filesystem operation failed"),
        }
    }
}

impl std::error::Error for BenchmarkError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BenchmarkErrorKind {
    InvalidConfig { field: &'static str },
    FullText,
    Io,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BenchmarkGateError {
    message: &'static str,
}

impl BenchmarkGateError {
    fn invalid_json() -> Self {
        Self {
            message: "benchmark report is not valid JSON",
        }
    }

    fn missing_field(field: &'static str) -> Self {
        Self { message: field }
    }

    fn failed(message: &'static str) -> Self {
        Self { message }
    }
}

impl fmt::Display for BenchmarkGateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.message {
            "schema_version"
            | "dataset_kind"
            | "document_count"
            | "query_count"
            | "query_latency_ms"
            | "samples"
            | "p95"
            | "zero_result_queries"
            | "million_scale_verified"
            | "target_claim" => {
                write!(
                    formatter,
                    "benchmark report missing required field: {}",
                    self.message
                )
            }
            message => formatter.write_str(message),
        }
    }
}

impl std::error::Error for BenchmarkGateError {}
