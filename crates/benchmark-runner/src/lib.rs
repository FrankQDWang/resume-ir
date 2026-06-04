use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use embedder::{
    Embedder, EmbeddingBudget, EmbeddingInput, LocalEmbeddingCommandEmbedder,
    LocalEmbeddingCommandSpec,
};
use extractor_rules::{extract_strong_fields, FieldType};
use index_fulltext::{FullTextIndex, IndexDocument, IndexSection, SearchQuery};
use ocr_client::{
    CancellationToken, LocalOcrCommandClient, LocalOcrCommandSpec, OcrClient, OcrOptions,
    OcrPageRequest, OcrWorkerBudget, RenderedPage, TesseractOcrClient, TesseractOcrSpec,
};

pub fn crate_name() -> &'static str {
    "benchmark-runner"
}

const DEFAULT_TOP_K: usize = 10;
const MAX_TOP_K: usize = 100;
const DEFAULT_SYNTHETIC_OCR_RENDER_DPI: u32 = 150;
const DEFAULT_VECTOR_QUALITY_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_VECTOR_QUALITY_TEXT_BYTES: usize = 128 * 1024;

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SyntheticOcrBenchmarkConfig {
    page_count: usize,
    page_timeout_ms: u64,
    render_dpi: u32,
}

impl SyntheticOcrBenchmarkConfig {
    pub fn new(page_count: usize, page_timeout_ms: u64) -> Result<Self> {
        if page_count == 0 || page_count > u32::MAX as usize {
            return Err(BenchmarkError::invalid_config("ocr_page_count"));
        }
        if page_timeout_ms == 0 {
            return Err(BenchmarkError::invalid_config("ocr_page_timeout_ms"));
        }

        Ok(Self {
            page_count,
            page_timeout_ms,
            render_dpi: DEFAULT_SYNTHETIC_OCR_RENDER_DPI,
        })
    }

    pub fn with_render_dpi(mut self, render_dpi: u32) -> Result<Self> {
        if render_dpi == 0 {
            return Err(BenchmarkError::invalid_config("ocr_render_dpi"));
        }
        self.render_dpi = render_dpi;
        Ok(self)
    }

    pub fn page_count(self) -> usize {
        self.page_count
    }

    pub fn page_timeout_ms(self) -> u64 {
        self.page_timeout_ms
    }

    pub fn render_dpi(self) -> u32 {
        self.render_dpi
    }
}

impl fmt::Debug for SyntheticOcrBenchmarkConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyntheticOcrBenchmarkConfig")
            .field("page_count", &self.page_count)
            .field("page_timeout_ms", &self.page_timeout_ms)
            .field("render_dpi", &self.render_dpi)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum SyntheticOcrBenchmarkEngine {
    LocalCommand { command: PathBuf },
    Tesseract { command: PathBuf },
}

impl SyntheticOcrBenchmarkEngine {
    pub fn local_command(command: impl AsRef<Path>) -> Result<Self> {
        let command = command.as_ref().to_path_buf();
        if command.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config("ocr_command"));
        }
        Ok(Self::LocalCommand { command })
    }

    pub fn tesseract(command: impl AsRef<Path>) -> Result<Self> {
        let command = command.as_ref().to_path_buf();
        if command.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config("ocr_tesseract_command"));
        }
        Ok(Self::Tesseract { command })
    }

    fn engine_kind(&self) -> &'static str {
        match self {
            Self::LocalCommand { .. } => "local-command",
            Self::Tesseract { .. } => "tesseract",
        }
    }
}

impl fmt::Debug for SyntheticOcrBenchmarkEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyntheticOcrBenchmarkEngine")
            .field("engine_kind", &self.engine_kind())
            .field("command", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct VectorQualityConfig {
    command: PathBuf,
    model_id: String,
    dimension: usize,
    top_k: usize,
    timeout_ms: u64,
    max_text_bytes: usize,
}

impl VectorQualityConfig {
    pub fn new(
        command: impl AsRef<Path>,
        model_id: impl Into<String>,
        dimension: usize,
    ) -> Result<Self> {
        let command = command.as_ref().to_path_buf();
        let model_id = model_id.into();
        if command.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config("vector_command"));
        }
        if model_id.trim().is_empty() {
            return Err(BenchmarkError::invalid_config("vector_model_id"));
        }
        if dimension == 0 {
            return Err(BenchmarkError::invalid_config("vector_dimension"));
        }

        Ok(Self {
            command,
            model_id,
            dimension,
            top_k: DEFAULT_TOP_K,
            timeout_ms: DEFAULT_VECTOR_QUALITY_TIMEOUT_MS,
            max_text_bytes: DEFAULT_VECTOR_QUALITY_TEXT_BYTES,
        })
    }

    pub fn with_top_k(mut self, top_k: usize) -> Self {
        self.top_k = top_k.clamp(1, MAX_TOP_K);
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Result<Self> {
        if timeout_ms == 0 {
            return Err(BenchmarkError::invalid_config("vector_timeout_ms"));
        }
        self.timeout_ms = timeout_ms;
        Ok(self)
    }

    pub fn with_max_text_bytes(mut self, max_text_bytes: usize) -> Result<Self> {
        if max_text_bytes == 0 {
            return Err(BenchmarkError::invalid_config("vector_max_text_bytes"));
        }
        self.max_text_bytes = max_text_bytes;
        Ok(self)
    }

    pub fn top_k(&self) -> usize {
        self.top_k
    }
}

impl fmt::Debug for VectorQualityConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorQualityConfig")
            .field("command", &"<redacted>")
            .field("model_id", &self.model_id)
            .field("dimension", &self.dimension)
            .field("top_k", &self.top_k)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_text_bytes", &self.max_text_bytes)
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

#[derive(Clone, PartialEq)]
pub struct OcrThroughputReport {
    run_id: String,
    platform: String,
    dataset_kind: &'static str,
    engine_kind: &'static str,
    page_count: usize,
    total_ms: f64,
    total_page_bytes: usize,
    total_text_bytes: usize,
    mean_confidence: f32,
    latency: LatencySummary,
    target_claim: &'static str,
}

impl OcrThroughputReport {
    pub fn dataset_kind(&self) -> &'static str {
        self.dataset_kind
    }

    pub fn engine_kind(&self) -> &'static str {
        self.engine_kind
    }

    pub fn page_count(&self) -> usize {
        self.page_count
    }

    pub fn total_page_bytes(&self) -> usize {
        self.total_page_bytes
    }

    pub fn total_text_bytes(&self) -> usize {
        self.total_text_bytes
    }

    pub fn latency(&self) -> &LatencySummary {
        &self.latency
    }

    pub fn pages_per_second(&self) -> f64 {
        if self.total_ms <= 0.0 {
            return 0.0;
        }

        self.page_count as f64 / (self.total_ms / 1000.0)
    }

    pub fn to_redacted_json(&self) -> String {
        format!(
            concat!(
                "{{",
                "\"schema_version\":\"ocr-throughput.v1\",",
                "\"run_id\":\"{}\",",
                "\"platform\":\"{}\",",
                "\"dataset_kind\":\"{}\",",
                "\"engine_kind\":\"{}\",",
                "\"page_count\":{},",
                "\"total_ms\":{},",
                "\"pages_per_second\":{},",
                "\"total_page_bytes\":{},",
                "\"total_text_bytes\":{},",
                "\"mean_confidence\":{},",
                "\"page_latency_ms\":{{",
                "\"samples\":{},",
                "\"min\":{},",
                "\"mean\":{},",
                "\"p50\":{},",
                "\"p95\":{},",
                "\"p99\":{},",
                "\"max\":{}",
                "}},",
                "\"target_claim\":\"{}\",",
                "\"scope\":\"synthetic OCR throughput benchmark; no raw OCR text, page bytes, command paths, or resume paths included\"",
                "}}"
            ),
            self.run_id,
            self.platform,
            self.dataset_kind,
            self.engine_kind,
            self.page_count,
            format_ms(self.total_ms),
            format_ms(self.pages_per_second()),
            self.total_page_bytes,
            self.total_text_bytes,
            format_ms(self.mean_confidence as f64),
            self.latency.samples,
            format_ms(self.latency.min_ms),
            format_ms(self.latency.mean_ms),
            format_ms(self.latency.p50_ms),
            format_ms(self.latency.p95_ms),
            format_ms(self.latency.p99_ms),
            format_ms(self.latency.max_ms),
            self.target_claim,
        )
    }
}

impl fmt::Debug for OcrThroughputReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OcrThroughputReport")
            .field("run_id", &self.run_id)
            .field("platform", &self.platform)
            .field("dataset_kind", &self.dataset_kind)
            .field("engine_kind", &self.engine_kind)
            .field("page_count", &self.page_count)
            .field("total_ms", &self.total_ms)
            .field("total_page_bytes", &self.total_page_bytes)
            .field("total_text_bytes", &self.total_text_bytes)
            .field("mean_confidence", &self.mean_confidence)
            .field("latency", &self.latency)
            .field("target_claim", &self.target_claim)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct VectorQualityReport {
    run_id: String,
    platform: String,
    dataset_kind: &'static str,
    sample_count: usize,
    candidate_count: usize,
    relevant_count: usize,
    top_k: usize,
    recall_at_k: f64,
    mrr: f64,
    ndcg_at_k: f64,
    zero_recall_queries: usize,
    model_id: String,
    dimension: usize,
    target_claim: &'static str,
}

impl VectorQualityReport {
    pub fn dataset_kind(&self) -> &'static str {
        self.dataset_kind
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub fn candidate_count(&self) -> usize {
        self.candidate_count
    }

    pub fn top_k(&self) -> usize {
        self.top_k
    }

    pub fn recall_at_k(&self) -> f64 {
        self.recall_at_k
    }

    pub fn mrr(&self) -> f64 {
        self.mrr
    }

    pub fn ndcg_at_k(&self) -> f64 {
        self.ndcg_at_k
    }

    pub fn zero_recall_queries(&self) -> usize {
        self.zero_recall_queries
    }

    pub fn to_redacted_json(&self) -> String {
        format!(
            concat!(
                "{{",
                "\"schema_version\":\"vector-quality.v1\",",
                "\"run_id\":\"{}\",",
                "\"platform\":\"{}\",",
                "\"dataset_kind\":\"{}\",",
                "\"sample_count\":{},",
                "\"candidate_count\":{},",
                "\"relevant_count\":{},",
                "\"top_k\":{},",
                "\"recall_at_k\":{},",
                "\"mrr\":{},",
                "\"ndcg_at_k\":{},",
                "\"zero_recall_queries\":{},",
                "\"model_id\":\"{}\",",
                "\"dimension\":{},",
                "\"target_claim\":\"{}\",",
                "\"scope\":\"labeled vector retrieval quality; no raw queries, candidate text, sample ids, candidate ids, vectors, command paths, or resume paths included\"",
                "}}"
            ),
            self.run_id,
            self.platform,
            self.dataset_kind,
            self.sample_count,
            self.candidate_count,
            self.relevant_count,
            self.top_k,
            format_ms(self.recall_at_k),
            format_ms(self.mrr),
            format_ms(self.ndcg_at_k),
            self.zero_recall_queries,
            escape_json_string(&self.model_id),
            self.dimension,
            self.target_claim,
        )
    }
}

impl fmt::Debug for VectorQualityReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VectorQualityReport")
            .field("run_id", &self.run_id)
            .field("platform", &self.platform)
            .field("dataset_kind", &self.dataset_kind)
            .field("sample_count", &self.sample_count)
            .field("candidate_count", &self.candidate_count)
            .field("relevant_count", &self.relevant_count)
            .field("top_k", &self.top_k)
            .field("recall_at_k", &self.recall_at_k)
            .field("mrr", &self.mrr)
            .field("ndcg_at_k", &self.ndcg_at_k)
            .field("zero_recall_queries", &self.zero_recall_queries)
            .field("model_id", &self.model_id)
            .field("dimension", &self.dimension)
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

pub fn run_synthetic_ocr_throughput_benchmark(
    engine: SyntheticOcrBenchmarkEngine,
    config: SyntheticOcrBenchmarkConfig,
) -> Result<OcrThroughputReport> {
    let engine_kind = engine.engine_kind();
    let client: Box<dyn OcrClient> = match engine {
        SyntheticOcrBenchmarkEngine::LocalCommand { command } => {
            let spec =
                LocalOcrCommandSpec::new(command, Vec::<String>::new(), "synthetic-benchmark")
                    .map_err(BenchmarkError::ocr)?;
            Box::new(LocalOcrCommandClient::new(spec))
        }
        SyntheticOcrBenchmarkEngine::Tesseract { command } => {
            let spec = TesseractOcrSpec::new(command, "synthetic-benchmark")
                .map_err(BenchmarkError::ocr)?;
            Box::new(TesseractOcrClient::new(spec))
        }
    };
    let budget = OcrWorkerBudget::new(config.page_timeout_ms).map_err(BenchmarkError::ocr)?;
    let options = OcrOptions::new("eng", "synthetic-benchmark").map_err(BenchmarkError::ocr)?;
    let cancellation = CancellationToken::new();
    let mut latencies = Vec::with_capacity(config.page_count);
    let mut total_page_bytes = 0_usize;
    let mut total_text_bytes = 0_usize;
    let mut confidence_sum = 0.0_f32;
    let run_started = Instant::now();

    for index in 0..config.page_count {
        let page_no = u32::try_from(index + 1)
            .map_err(|_| BenchmarkError::invalid_config("ocr_page_count"))?;
        let page_bytes = synthetic_ocr_page_bytes(index);
        total_page_bytes += page_bytes.len();
        let rendered_page = RenderedPage::new(page_no, config.render_dpi, page_bytes)
            .map_err(BenchmarkError::ocr)?;
        let request =
            OcrPageRequest::new(rendered_page, options.clone()).map_err(BenchmarkError::ocr)?;
        let page_started = Instant::now();
        let page = client
            .recognize_page(request, budget, &cancellation)
            .map_err(BenchmarkError::ocr)?;
        latencies.push(elapsed_ms(page_started));
        total_text_bytes += page.text().len();
        confidence_sum += page.confidence();
    }

    let total_ms = elapsed_ms(run_started);
    Ok(OcrThroughputReport {
        run_id: generate_run_id(),
        platform: platform_label(),
        dataset_kind: "synthetic",
        engine_kind,
        page_count: config.page_count,
        total_ms,
        total_page_bytes,
        total_text_bytes,
        mean_confidence: confidence_sum / config.page_count as f32,
        latency: LatencySummary::from_samples(latencies)?,
        target_claim: "not_evaluated",
    })
}

pub fn run_vector_quality_jsonl(
    dataset_jsonl: &str,
    config: VectorQualityConfig,
) -> Result<VectorQualityReport> {
    let samples = dataset_jsonl
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(parse_vector_quality_sample)
        .collect::<Result<Vec<_>>>()?;
    if samples.is_empty() {
        return Err(BenchmarkError::invalid_config("vector_quality_samples"));
    }

    let mut inputs = Vec::<EmbeddingInput>::new();
    let mut query_input_ids = Vec::<String>::new();
    let mut candidate_input_ids = Vec::<Vec<String>>::new();
    let mut candidate_count = 0_usize;
    let mut relevant_count = 0_usize;

    for (sample_index, sample) in samples.iter().enumerate() {
        let query_id = format!("query-{sample_index:06}");
        inputs.push(EmbeddingInput::new(
            query_id.as_str(),
            sample.query.as_str(),
        ));
        query_input_ids.push(query_id);

        let mut sample_candidate_ids = Vec::with_capacity(sample.candidates.len());
        for (candidate_index, candidate) in sample.candidates.iter().enumerate() {
            let candidate_id = format!("candidate-{sample_index:06}-{candidate_index:06}");
            inputs.push(EmbeddingInput::new(
                candidate_id.as_str(),
                candidate.text.as_str(),
            ));
            sample_candidate_ids.push(candidate_id);
            candidate_count += 1;
            if candidate.relevant {
                relevant_count += 1;
            }
        }
        candidate_input_ids.push(sample_candidate_ids);
    }

    let spec = LocalEmbeddingCommandSpec::new(
        config.command.clone(),
        Vec::<String>::new(),
        config.model_id.as_str(),
        config.dimension,
    )
    .and_then(|spec| spec.with_timeout_ms(config.timeout_ms))
    .map_err(BenchmarkError::embedding)?;
    let embedder = LocalEmbeddingCommandEmbedder::new(spec);
    let vectors = embedder
        .embed_batch(
            &inputs,
            EmbeddingBudget::new(inputs.len(), config.max_text_bytes),
        )
        .map_err(BenchmarkError::embedding)?;
    let vector_by_id = vectors
        .into_iter()
        .map(|vector| (vector.id().to_string(), vector.values().to_vec()))
        .collect::<BTreeMap<_, _>>();

    let mut recall_sum = 0.0_f64;
    let mut reciprocal_rank_sum = 0.0_f64;
    let mut ndcg_sum = 0.0_f64;
    let mut zero_recall_queries = 0_usize;

    for (sample_index, sample) in samples.iter().enumerate() {
        let query_vector = vector_by_id
            .get(&query_input_ids[sample_index])
            .ok_or_else(|| BenchmarkError::invalid_config("vector_quality.query_embedding"))?;
        let mut ranked = candidate_input_ids[sample_index]
            .iter()
            .enumerate()
            .map(|(candidate_index, candidate_id)| {
                let candidate_vector = vector_by_id.get(candidate_id).ok_or_else(|| {
                    BenchmarkError::invalid_config("vector_quality.candidate_embedding")
                })?;
                Ok(VectorQualityRankedCandidate {
                    candidate_index,
                    relevant: sample.candidates[candidate_index].relevant,
                    score: cosine_similarity(query_vector, candidate_vector),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ranked.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.candidate_index.cmp(&right.candidate_index))
        });

        let top_k = ranked.iter().take(config.top_k).collect::<Vec<_>>();
        let sample_relevant = sample
            .candidates
            .iter()
            .filter(|candidate| candidate.relevant)
            .count();
        let relevant_in_top_k = top_k.iter().filter(|candidate| candidate.relevant).count();
        if relevant_in_top_k == 0 {
            zero_recall_queries += 1;
        }
        recall_sum += relevant_in_top_k as f64 / sample_relevant as f64;
        reciprocal_rank_sum += first_relevant_reciprocal_rank(&ranked);
        ndcg_sum += binary_ndcg_at_k(&ranked, config.top_k, sample_relevant);
    }

    let sample_count = samples.len();
    Ok(VectorQualityReport {
        run_id: generate_run_id(),
        platform: platform_label(),
        dataset_kind: "labeled",
        sample_count,
        candidate_count,
        relevant_count,
        top_k: config.top_k,
        recall_at_k: recall_sum / sample_count as f64,
        mrr: reciprocal_rank_sum / sample_count as f64,
        ndcg_at_k: ndcg_sum / sample_count as f64,
        zero_recall_queries,
        model_id: config.model_id,
        dimension: config.dimension,
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OcrThroughputGateConfig {
    min_pages: usize,
    max_p95_ms: f64,
    min_pages_per_second: f64,
    allow_synthetic: bool,
}

impl OcrThroughputGateConfig {
    pub fn new(min_pages: usize, max_p95_ms: f64, min_pages_per_second: f64) -> Self {
        Self {
            min_pages,
            max_p95_ms,
            min_pages_per_second,
            allow_synthetic: false,
        }
    }

    pub fn allow_synthetic(mut self) -> Self {
        self.allow_synthetic = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OcrThroughputGateEvaluation {
    dataset_kind: String,
    page_count: usize,
    p95_ms: f64,
    pages_per_second: f64,
}

impl OcrThroughputGateEvaluation {
    pub fn dataset_kind(&self) -> &str {
        &self.dataset_kind
    }

    pub fn page_count(&self) -> usize {
        self.page_count
    }

    pub fn p95_ms(&self) -> f64 {
        self.p95_ms
    }

    pub fn pages_per_second(&self) -> f64 {
        self.pages_per_second
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldQualityGateConfig {
    min_precision: f64,
    min_recall: f64,
    min_f1: f64,
    min_samples: usize,
}

impl FieldQualityGateConfig {
    pub fn new(min_precision: f64, min_recall: f64, min_f1: f64) -> Self {
        Self {
            min_precision,
            min_recall,
            min_f1,
            min_samples: 1,
        }
    }

    pub fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.min_samples = min_samples;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldQualityGateEvaluation {
    dataset_kind: String,
    sample_count: usize,
    precision: f64,
    recall: f64,
    f1: f64,
}

impl FieldQualityGateEvaluation {
    pub fn dataset_kind(&self) -> &str {
        &self.dataset_kind
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub fn f1(&self) -> f64 {
        self.f1
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VectorQualityGateConfig {
    min_samples: usize,
    min_recall_at_k: f64,
    min_mrr: f64,
    min_ndcg_at_k: f64,
    max_zero_recall_queries: usize,
}

impl VectorQualityGateConfig {
    pub fn new(min_samples: usize, min_recall_at_k: f64, min_mrr: f64, min_ndcg_at_k: f64) -> Self {
        Self {
            min_samples,
            min_recall_at_k,
            min_mrr,
            min_ndcg_at_k,
            max_zero_recall_queries: 0,
        }
    }

    pub fn with_max_zero_recall_queries(mut self, max_zero_recall_queries: usize) -> Self {
        self.max_zero_recall_queries = max_zero_recall_queries;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorQualityGateEvaluation {
    dataset_kind: String,
    sample_count: usize,
    recall_at_k: f64,
    mrr: f64,
    ndcg_at_k: f64,
}

impl VectorQualityGateEvaluation {
    pub fn dataset_kind(&self) -> &str {
        &self.dataset_kind
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub fn recall_at_k(&self) -> f64 {
        self.recall_at_k
    }

    pub fn mrr(&self) -> f64 {
        self.mrr
    }

    pub fn ndcg_at_k(&self) -> f64 {
        self.ndcg_at_k
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct FieldCounts {
    true_positive: usize,
    false_positive: usize,
    false_negative: usize,
}

impl FieldCounts {
    fn record_true_positive(&mut self) {
        self.true_positive += 1;
    }

    fn record_false_positive(&mut self) {
        self.false_positive += 1;
    }

    fn record_false_negative(&mut self) {
        self.false_negative += 1;
    }

    fn extend(self, other: Self) -> Self {
        Self {
            true_positive: self.true_positive + other.true_positive,
            false_positive: self.false_positive + other.false_positive,
            false_negative: self.false_negative + other.false_negative,
        }
    }

    fn metric(self) -> FieldQualityMetric {
        let precision_denominator = self.true_positive + self.false_positive;
        let recall_denominator = self.true_positive + self.false_negative;
        let precision = if precision_denominator == 0 {
            0.0
        } else {
            self.true_positive as f64 / precision_denominator as f64
        };
        let recall = if recall_denominator == 0 {
            0.0
        } else {
            self.true_positive as f64 / recall_denominator as f64
        };
        let f1 = if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };

        FieldQualityMetric {
            true_positive: self.true_positive,
            false_positive: self.false_positive,
            false_negative: self.false_negative,
            precision,
            recall,
            f1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldQualityMetric {
    true_positive: usize,
    false_positive: usize,
    false_negative: usize,
    precision: f64,
    recall: f64,
    f1: f64,
}

impl FieldQualityMetric {
    pub fn precision(&self) -> f64 {
        self.precision
    }

    pub fn recall(&self) -> f64 {
        self.recall
    }

    pub fn f1(&self) -> f64 {
        self.f1
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldQualityReport {
    run_id: String,
    platform: String,
    dataset_kind: &'static str,
    sample_count: usize,
    expected_mentions: usize,
    predicted_mentions: usize,
    overall: FieldQualityMetric,
    fields: BTreeMap<String, FieldQualityMetric>,
    target_claim: &'static str,
}

impl FieldQualityReport {
    pub fn dataset_kind(&self) -> &'static str {
        self.dataset_kind
    }

    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub fn expected_mentions(&self) -> usize {
        self.expected_mentions
    }

    pub fn overall(&self) -> &FieldQualityMetric {
        &self.overall
    }

    pub fn field_metric(&self, field_type: &str) -> Option<&FieldQualityMetric> {
        self.fields.get(field_type)
    }

    pub fn to_redacted_json(&self) -> String {
        let fields_json = self
            .fields
            .iter()
            .map(|(field_type, metric)| format!("\"{field_type}\":{}", field_metric_json(*metric)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            concat!(
                "{{",
                "\"schema_version\":\"field-quality.v1\",",
                "\"run_id\":\"{}\",",
                "\"platform\":\"{}\",",
                "\"dataset_kind\":\"{}\",",
                "\"sample_count\":{},",
                "\"expected_mentions\":{},",
                "\"predicted_mentions\":{},",
                "\"overall\":{},",
                "\"fields\":{{{}}},",
                "\"target_claim\":\"{}\",",
                "\"scope\":\"labeled field extraction quality; no raw resume text, paths, sample ids, or field values included\"",
                "}}"
            ),
            self.run_id,
            self.platform,
            self.dataset_kind,
            self.sample_count,
            self.expected_mentions,
            self.predicted_mentions,
            field_metric_json(self.overall),
            fields_json,
            self.target_claim,
        )
    }
}

pub fn run_field_quality_jsonl(dataset_jsonl: &str) -> Result<FieldQualityReport> {
    let mut sample_count = 0_usize;
    let mut expected_mentions = 0_usize;
    let mut predicted_mentions = 0_usize;
    let mut field_counts = BTreeMap::<String, FieldCounts>::new();

    for line in dataset_jsonl
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let sample = parse_field_quality_sample(line)?;
        sample_count += 1;
        expected_mentions += sample.expected.len();
        let predictions = field_quality_predictions(&sample.text);
        predicted_mentions += predictions.len();
        score_field_quality_sample(&sample.expected, &predictions, &mut field_counts);
    }

    if sample_count == 0 {
        return Err(BenchmarkError::invalid_config("field_quality_samples"));
    }

    let all_counts = field_counts
        .values()
        .copied()
        .fold(FieldCounts::default(), FieldCounts::extend);
    let fields = field_counts
        .into_iter()
        .map(|(field_type, counts)| (field_type, counts.metric()))
        .collect::<BTreeMap<_, _>>();

    Ok(FieldQualityReport {
        run_id: generate_run_id(),
        platform: platform_label(),
        dataset_kind: "labeled",
        sample_count,
        expected_mentions,
        predicted_mentions,
        overall: all_counts.metric(),
        fields,
        target_claim: "not_evaluated",
    })
}

pub fn evaluate_field_quality_gate_json(
    report_json: &str,
    config: FieldQualityGateConfig,
) -> std::result::Result<FieldQualityGateEvaluation, BenchmarkGateError> {
    let report: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| BenchmarkGateError::invalid_json())?;

    let schema_version = required_str(&report, "schema_version")?;
    if schema_version != "field-quality.v1" {
        return Err(BenchmarkGateError::failed(
            "unsupported field quality schema",
        ));
    }
    let dataset_kind = required_str(&report, "dataset_kind")?;
    if dataset_kind != "labeled" {
        return Err(BenchmarkGateError::failed(
            "field quality requires labeled dataset",
        ));
    }
    let sample_count = required_usize(&report, "sample_count")?;
    let overall = report
        .get("overall")
        .ok_or_else(|| BenchmarkGateError::missing_field("overall"))?;
    let precision = required_f64(overall, "precision")?;
    let recall = required_f64(overall, "recall")?;
    let f1 = required_f64(overall, "f1")?;
    let target_claim = required_str(&report, "target_claim")?;

    if sample_count < config.min_samples {
        return Err(BenchmarkGateError::failed(
            "field sample count below gate minimum",
        ));
    }
    if precision < config.min_precision {
        return Err(BenchmarkGateError::failed(
            "field precision below threshold",
        ));
    }
    if recall < config.min_recall {
        return Err(BenchmarkGateError::failed("field recall below threshold"));
    }
    if f1 < config.min_f1 {
        return Err(BenchmarkGateError::failed("field f1 below threshold"));
    }
    if target_claim != "not_evaluated" {
        return Err(BenchmarkGateError::failed(
            "field target claim is not proven",
        ));
    }

    Ok(FieldQualityGateEvaluation {
        dataset_kind: dataset_kind.to_string(),
        sample_count,
        precision,
        recall,
        f1,
    })
}

pub fn evaluate_vector_quality_gate_json(
    report_json: &str,
    config: VectorQualityGateConfig,
) -> std::result::Result<VectorQualityGateEvaluation, BenchmarkGateError> {
    let report: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| BenchmarkGateError::invalid_json())?;

    let schema_version = required_str(&report, "schema_version")?;
    if schema_version != "vector-quality.v1" {
        return Err(BenchmarkGateError::failed(
            "unsupported vector quality schema",
        ));
    }
    let dataset_kind = required_str(&report, "dataset_kind")?;
    if dataset_kind != "labeled" {
        return Err(BenchmarkGateError::failed(
            "vector quality requires labeled dataset",
        ));
    }
    let sample_count = required_usize(&report, "sample_count")?;
    let recall_at_k = required_f64(&report, "recall_at_k")?;
    let mrr = required_f64(&report, "mrr")?;
    let ndcg_at_k = required_f64(&report, "ndcg_at_k")?;
    let zero_recall_queries = required_usize(&report, "zero_recall_queries")?;
    let target_claim = required_str(&report, "target_claim")?;

    if sample_count < config.min_samples {
        return Err(BenchmarkGateError::failed(
            "vector sample count below gate minimum",
        ));
    }
    if recall_at_k < config.min_recall_at_k {
        return Err(BenchmarkGateError::failed("vector recall below threshold"));
    }
    if mrr < config.min_mrr {
        return Err(BenchmarkGateError::failed("vector mrr below threshold"));
    }
    if ndcg_at_k < config.min_ndcg_at_k {
        return Err(BenchmarkGateError::failed("vector ndcg below threshold"));
    }
    if zero_recall_queries > config.max_zero_recall_queries {
        return Err(BenchmarkGateError::failed(
            "vector zero-recall query count exceeded threshold",
        ));
    }
    if target_claim != "not_evaluated" {
        return Err(BenchmarkGateError::failed(
            "vector target claim is not proven",
        ));
    }

    Ok(VectorQualityGateEvaluation {
        dataset_kind: dataset_kind.to_string(),
        sample_count,
        recall_at_k,
        mrr,
        ndcg_at_k,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FieldQualitySample {
    text: String,
    expected: Vec<FieldQualityMention>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VectorQualitySample {
    query: String,
    candidates: Vec<VectorQualityCandidate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VectorQualityCandidate {
    text: String,
    relevant: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct VectorQualityRankedCandidate {
    candidate_index: usize,
    relevant: bool,
    score: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct FieldQualityMention {
    field_type: String,
    normalized_value: String,
}

fn parse_field_quality_sample(line: &str) -> Result<FieldQualitySample> {
    let value = serde_json::from_str::<serde_json::Value>(line)
        .map_err(|_| BenchmarkError::invalid_config("field_quality_jsonl"))?;
    let text = value
        .get("text")
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| BenchmarkError::invalid_config("field_quality.text"))?
        .to_string();
    let expected_values = value
        .get("expected")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| BenchmarkError::invalid_config("field_quality.expected"))?;
    if expected_values.is_empty() {
        return Err(BenchmarkError::invalid_config("field_quality.expected"));
    }
    let mut expected = Vec::with_capacity(expected_values.len());
    for expected_value in expected_values {
        let field_type = expected_value
            .get("type")
            .and_then(serde_json::Value::as_str)
            .and_then(canonical_field_type)
            .ok_or_else(|| BenchmarkError::invalid_config("field_quality.expected.type"))?;
        let normalized_value = expected_value
            .get("normalized")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| BenchmarkError::invalid_config("field_quality.expected.normalized"))?;
        expected.push(FieldQualityMention {
            field_type: field_type.to_string(),
            normalized_value: normalized_value.to_string(),
        });
    }

    Ok(FieldQualitySample { text, expected })
}

fn parse_vector_quality_sample(line: &str) -> Result<VectorQualitySample> {
    let value = serde_json::from_str::<serde_json::Value>(line)
        .map_err(|_| BenchmarkError::invalid_config("vector_quality_jsonl"))?;
    let query = value
        .get("query")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| BenchmarkError::invalid_config("vector_quality.query"))?
        .to_string();
    let candidate_values = value
        .get("candidates")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| BenchmarkError::invalid_config("vector_quality.candidates"))?;
    if candidate_values.is_empty() {
        return Err(BenchmarkError::invalid_config("vector_quality.candidates"));
    }

    let mut candidates = Vec::with_capacity(candidate_values.len());
    for candidate_value in candidate_values {
        let _id = candidate_value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| BenchmarkError::invalid_config("vector_quality.candidate.id"))?;
        let text = candidate_value
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| BenchmarkError::invalid_config("vector_quality.candidate.text"))?
            .to_string();
        let relevant = candidate_value
            .get("relevant")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| BenchmarkError::invalid_config("vector_quality.candidate.relevant"))?;
        candidates.push(VectorQualityCandidate { text, relevant });
    }

    if !candidates.iter().any(|candidate| candidate.relevant) {
        return Err(BenchmarkError::invalid_config(
            "vector_quality.candidates.relevant",
        ));
    }

    Ok(VectorQualitySample { query, candidates })
}

fn first_relevant_reciprocal_rank(ranked: &[VectorQualityRankedCandidate]) -> f64 {
    ranked
        .iter()
        .position(|candidate| candidate.relevant)
        .map(|index| 1.0 / (index + 1) as f64)
        .unwrap_or(0.0)
}

fn binary_ndcg_at_k(
    ranked: &[VectorQualityRankedCandidate],
    top_k: usize,
    relevant_count: usize,
) -> f64 {
    let dcg = ranked
        .iter()
        .take(top_k)
        .enumerate()
        .filter(|(_, candidate)| candidate.relevant)
        .map(|(index, _)| 1.0 / ((index + 2) as f64).log2())
        .sum::<f64>();
    let ideal_hits = relevant_count.min(top_k);
    let idcg = (0..ideal_hits)
        .map(|index| 1.0 / ((index + 2) as f64).log2())
        .sum::<f64>();

    if idcg == 0.0 {
        0.0
    } else {
        dcg / idcg
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (left_value, right_value) in left.iter().zip(right.iter()) {
        dot += left_value * right_value;
        left_norm += left_value * left_value;
        right_norm += right_value * right_value;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn field_quality_predictions(text: &str) -> Vec<FieldQualityMention> {
    extract_strong_fields(text)
        .into_iter()
        .filter_map(|rule_match| {
            let field_type = field_type_label(&rule_match.field_type);
            let normalized_value = rule_match
                .normalized_value
                .unwrap_or(rule_match.raw_value)
                .trim()
                .to_string();
            (!normalized_value.is_empty()).then_some(FieldQualityMention {
                field_type: field_type.to_string(),
                normalized_value,
            })
        })
        .collect()
}

fn score_field_quality_sample(
    expected: &[FieldQualityMention],
    predictions: &[FieldQualityMention],
    field_counts: &mut BTreeMap<String, FieldCounts>,
) {
    let mut expected_counts = mention_multiset(expected);
    let known_fields = expected
        .iter()
        .chain(predictions.iter())
        .map(|mention| mention.field_type.clone())
        .collect::<BTreeSet<_>>();
    for field_type in known_fields {
        field_counts.entry(field_type).or_default();
    }

    for prediction in predictions {
        let counts = field_counts
            .entry(prediction.field_type.clone())
            .or_default();
        match expected_counts.get_mut(prediction) {
            Some(remaining) if *remaining > 0 => {
                *remaining -= 1;
                counts.record_true_positive();
            }
            _ => counts.record_false_positive(),
        }
    }

    for (mention, remaining) in expected_counts {
        let counts = field_counts.entry(mention.field_type).or_default();
        for _ in 0..remaining {
            counts.record_false_negative();
        }
    }
}

fn mention_multiset(mentions: &[FieldQualityMention]) -> BTreeMap<FieldQualityMention, usize> {
    let mut counts = BTreeMap::<FieldQualityMention, usize>::new();
    for mention in mentions {
        *counts.entry(mention.clone()).or_default() += 1;
    }
    counts
}

fn canonical_field_type(value: &str) -> Option<&'static str> {
    match value {
        "name" => Some("name"),
        "email" => Some("email"),
        "phone" => Some("phone"),
        "date_range" => Some("date_range"),
        "school" => Some("school"),
        "degree" => Some("degree"),
        "company" => Some("company"),
        "title" => Some("title"),
        "skill" => Some("skill"),
        "certificate" => Some("certificate"),
        "years_experience" => Some("years_experience"),
        _ => None,
    }
}

fn field_type_label(field_type: &FieldType) -> &'static str {
    match field_type {
        FieldType::Name => "name",
        FieldType::Email => "email",
        FieldType::Phone => "phone",
        FieldType::DateRange => "date_range",
        FieldType::School => "school",
        FieldType::Degree => "degree",
        FieldType::Company => "company",
        FieldType::Title => "title",
        FieldType::Skill => "skill",
        FieldType::Certificate => "certificate",
        FieldType::YearsExperience => "years_experience",
    }
}

fn field_metric_json(metric: FieldQualityMetric) -> String {
    format!(
        concat!(
            "{{",
            "\"true_positive\":{},",
            "\"false_positive\":{},",
            "\"false_negative\":{},",
            "\"precision\":{},",
            "\"recall\":{},",
            "\"f1\":{}",
            "}}"
        ),
        metric.true_positive,
        metric.false_positive,
        metric.false_negative,
        format_ms(metric.precision),
        format_ms(metric.recall),
        format_ms(metric.f1),
    )
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

pub fn evaluate_ocr_throughput_gate_json(
    report_json: &str,
    config: OcrThroughputGateConfig,
) -> std::result::Result<OcrThroughputGateEvaluation, BenchmarkGateError> {
    let report: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| BenchmarkGateError::invalid_json())?;

    let schema_version = required_str(&report, "schema_version")?;
    if schema_version != "ocr-throughput.v1" {
        return Err(BenchmarkGateError::failed(
            "unsupported OCR throughput schema",
        ));
    }

    let dataset_kind = required_str(&report, "dataset_kind")?;
    let page_count = required_usize(&report, "page_count")?;
    let latency = report
        .get("page_latency_ms")
        .ok_or_else(|| BenchmarkGateError::missing_field("page_latency_ms"))?;
    let samples = required_usize(latency, "samples")?;
    let p95_ms = required_f64(latency, "p95")?;
    let pages_per_second = required_f64(&report, "pages_per_second")?;
    let target_claim = required_str(&report, "target_claim")?;

    if dataset_kind == "synthetic" && !config.allow_synthetic {
        return Err(BenchmarkGateError::failed(
            "synthetic OCR benchmark requires explicit allowance",
        ));
    }
    if page_count < config.min_pages || samples < config.min_pages {
        return Err(BenchmarkGateError::failed(
            "OCR page sample count below gate minimum",
        ));
    }
    if p95_ms > config.max_p95_ms {
        return Err(BenchmarkGateError::failed(
            "OCR page p95 exceeded threshold",
        ));
    }
    if pages_per_second < config.min_pages_per_second {
        return Err(BenchmarkGateError::failed(
            "OCR pages-per-second below threshold",
        ));
    }
    if target_claim != "not_evaluated" {
        return Err(BenchmarkGateError::failed(
            "OCR throughput target claim is not proven",
        ));
    }

    Ok(OcrThroughputGateEvaluation {
        dataset_kind: dataset_kind.to_string(),
        page_count,
        p95_ms,
        pages_per_second,
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

fn synthetic_ocr_page_bytes(index: usize) -> Vec<u8> {
    let width = 32_usize;
    let height = 32_usize;
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    bytes.reserve(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let shade = if (x + y + index).is_multiple_of(11) {
                0
            } else {
                255
            };
            bytes.extend_from_slice(&[shade, shade, shade]);
        }
    }
    bytes
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

fn escape_json_string(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"<redacted>\"".to_string())
        .trim_matches('"')
        .to_string()
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

    fn ocr(_error: ocr_client::OcrError) -> Self {
        Self {
            kind: BenchmarkErrorKind::Ocr,
        }
    }

    fn embedding(_error: embedder::EmbeddingError) -> Self {
        Self {
            kind: BenchmarkErrorKind::Embedding,
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
            BenchmarkErrorKind::Ocr => formatter.write_str("benchmark OCR operation failed"),
            BenchmarkErrorKind::Embedding => {
                formatter.write_str("benchmark embedding operation failed")
            }
        }
    }
}

impl std::error::Error for BenchmarkError {}

#[derive(Clone, Debug, PartialEq, Eq)]
enum BenchmarkErrorKind {
    InvalidConfig { field: &'static str },
    FullText,
    Io,
    Ocr,
    Embedding,
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
            | "page_count"
            | "page_latency_ms"
            | "samples"
            | "p95"
            | "pages_per_second"
            | "zero_result_queries"
            | "sample_count"
            | "candidate_count"
            | "top_k"
            | "recall_at_k"
            | "mrr"
            | "ndcg_at_k"
            | "zero_recall_queries"
            | "model_id"
            | "dimension"
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
