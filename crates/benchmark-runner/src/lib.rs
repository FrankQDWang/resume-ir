use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use embedder::{
    Embedder, EmbeddingBudget, EmbeddingInput, LocalEmbeddingCommandEmbedder,
    LocalEmbeddingCommandSpec,
};
use extractor_rules::{extract_strong_fields, FieldType};
use index_fulltext::{FullTextIndex, IndexDocument, IndexSection, SearchQuery};
use ocr_client::{
    CancellationToken, LocalOcrCommandClient, LocalOcrCommandSpec, LocalPdfRenderCommandClient,
    LocalPdfRenderCommandSpec, OcrClient, OcrOptions, OcrPageRequest, OcrWorkerBudget,
    PdftoppmPdfRenderer, PdftoppmRenderSpec, RenderedPage, TesseractOcrClient, TesseractOcrSpec,
};
use rank_fusion::{soft_dedupe_score, DedupeProfile};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use sha2::{Digest, Sha256};

pub fn crate_name() -> &'static str {
    "benchmark-runner"
}

const DEFAULT_TOP_K: usize = 10;
const MAX_TOP_K: usize = 100;
const DEFAULT_SYNTHETIC_OCR_RENDER_DPI: u32 = 150;
const DEFAULT_PRIVATE_OCR_MAX_DOCUMENTS: usize = 100;
const DEFAULT_PRIVATE_OCR_MAX_PAGES: usize = 500;
const DEFAULT_PRIVATE_OCR_PAGES_PER_DOCUMENT: usize = 1;
const DEFAULT_PRIVATE_OCR_LANG: &str = "eng";
const DEFAULT_PRIVATE_OCR_PROFILE: &str = "private-real-corpus";
const DEFAULT_PRIVATE_QUERY_MAX_QUERIES: usize = PRIVATE_REAL_RELEASE_QUERY_SAMPLE_MIN;
const DEFAULT_PRIVATE_QUERY_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_VECTOR_QUALITY_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_VECTOR_QUALITY_TEXT_BYTES: usize = 128 * 1024;
const PRIVATE_REAL_RELEASE_QUERY_SAMPLE_MIN: usize = 500;

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

#[derive(Clone, PartialEq, Eq)]
pub struct PrivateQueryBenchmarkCommand {
    command: PathBuf,
    args: Vec<String>,
}

impl PrivateQueryBenchmarkCommand {
    pub fn local_command(command: impl AsRef<Path>) -> Result<Self> {
        Self::local_command_with_args(command, Vec::<String>::new())
    }

    pub fn local_command_with_args(
        command: impl AsRef<Path>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self> {
        let command = command.as_ref().to_path_buf();
        if command.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config("private_query_command"));
        }
        let args = args
            .into_iter()
            .map(Into::into)
            .map(|arg| {
                if arg.is_empty() {
                    return Err(BenchmarkError::invalid_config("private_query_command_arg"));
                }
                Ok(arg)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { command, args })
    }
}

impl fmt::Debug for PrivateQueryBenchmarkCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateQueryBenchmarkCommand")
            .field("command", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateQueryManifestDigests {
    dataset_manifest_sha256: String,
    query_set_sha256: String,
    model_manifest_sha256: String,
}

impl PrivateQueryManifestDigests {
    pub fn new(
        dataset_manifest_sha256: impl Into<String>,
        query_set_sha256: impl Into<String>,
        model_manifest_sha256: impl Into<String>,
    ) -> Result<Self> {
        let digests = Self {
            dataset_manifest_sha256: dataset_manifest_sha256.into(),
            query_set_sha256: query_set_sha256.into(),
            model_manifest_sha256: model_manifest_sha256.into(),
        };
        if !is_sha256_hex(&digests.dataset_manifest_sha256)
            || !is_sha256_hex(&digests.query_set_sha256)
            || !is_sha256_hex(&digests.model_manifest_sha256)
        {
            return Err(BenchmarkError::invalid_config(
                "private_query_manifest_sha256",
            ));
        }
        Ok(digests)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateFieldQualityManifestDigests {
    dataset_manifest_sha256: String,
    annotation_manifest_sha256: String,
}

impl PrivateFieldQualityManifestDigests {
    pub fn new(
        dataset_manifest_sha256: impl Into<String>,
        annotation_manifest_sha256: impl Into<String>,
    ) -> Result<Self> {
        let digests = Self {
            dataset_manifest_sha256: dataset_manifest_sha256.into(),
            annotation_manifest_sha256: annotation_manifest_sha256.into(),
        };
        if !is_sha256_hex(&digests.dataset_manifest_sha256)
            || !is_sha256_hex(&digests.annotation_manifest_sha256)
        {
            return Err(BenchmarkError::invalid_config(
                "private_field_quality_manifest_sha256",
            ));
        }
        Ok(digests)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateDedupeQualityManifestDigests {
    dataset_manifest_sha256: String,
    annotation_manifest_sha256: String,
}

impl PrivateDedupeQualityManifestDigests {
    pub fn new(
        dataset_manifest_sha256: impl Into<String>,
        annotation_manifest_sha256: impl Into<String>,
    ) -> Result<Self> {
        let digests = Self {
            dataset_manifest_sha256: dataset_manifest_sha256.into(),
            annotation_manifest_sha256: annotation_manifest_sha256.into(),
        };
        if !is_sha256_hex(&digests.dataset_manifest_sha256)
            || !is_sha256_hex(&digests.annotation_manifest_sha256)
        {
            return Err(BenchmarkError::invalid_config(
                "private_dedupe_quality_manifest_sha256",
            ));
        }
        Ok(digests)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateVectorQualityManifestDigests {
    dataset_manifest_sha256: String,
    annotation_manifest_sha256: String,
    model_manifest_sha256: String,
}

impl PrivateVectorQualityManifestDigests {
    pub fn new(
        dataset_manifest_sha256: impl Into<String>,
        annotation_manifest_sha256: impl Into<String>,
        model_manifest_sha256: impl Into<String>,
    ) -> Result<Self> {
        let digests = Self {
            dataset_manifest_sha256: dataset_manifest_sha256.into(),
            annotation_manifest_sha256: annotation_manifest_sha256.into(),
            model_manifest_sha256: model_manifest_sha256.into(),
        };
        if !is_sha256_hex(&digests.dataset_manifest_sha256)
            || !is_sha256_hex(&digests.annotation_manifest_sha256)
            || !is_sha256_hex(&digests.model_manifest_sha256)
        {
            return Err(BenchmarkError::invalid_config(
                "private_vector_quality_manifest_sha256",
            ));
        }
        Ok(digests)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateQueryCorpusSummary {
    document_count: usize,
    searchable_document_count: usize,
    vector_indexed_document_count: usize,
    sha256: String,
}

impl PrivateQueryCorpusSummary {
    pub fn from_redacted_json_file(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = fs::read(path).map_err(BenchmarkError::io)?;
        Self::from_redacted_json_bytes(bytes)
    }

    pub fn from_redacted_json_file_allowing_partial_hot_index_for_smoke(
        path: impl AsRef<Path>,
    ) -> Result<Self> {
        let bytes = fs::read(path).map_err(BenchmarkError::io)?;
        Self::from_redacted_json_bytes_with_policy(bytes, true)
    }

    pub fn from_redacted_json_bytes(bytes: impl AsRef<[u8]>) -> Result<Self> {
        Self::from_redacted_json_bytes_with_policy(bytes, false)
    }

    pub fn from_redacted_json_bytes_allowing_partial_hot_index_for_smoke(
        bytes: impl AsRef<[u8]>,
    ) -> Result<Self> {
        Self::from_redacted_json_bytes_with_policy(bytes, true)
    }

    fn from_redacted_json_bytes_with_policy(
        bytes: impl AsRef<[u8]>,
        allow_partial_hot_index_for_smoke: bool,
    ) -> Result<Self> {
        let bytes = bytes.as_ref();
        let report = serde_json::from_slice::<serde_json::Value>(bytes)
            .map_err(|_| BenchmarkError::invalid_config("private_query_corpus_summary_json"))?;
        validate_private_query_corpus_summary_shape(&report)?;
        if private_query_corpus_summary_str(&report, "schema_version")?
            != "benchmark-corpus-summary.v1"
            || private_query_corpus_summary_str(&report, "privacy_boundary")?
                != "redacted_local_aggregate"
            || private_query_corpus_summary_bool(&report, "contains_raw_resume_text")?
            || private_query_corpus_summary_bool(&report, "contains_resume_paths")?
            || private_query_corpus_summary_bool(&report, "contains_queries")?
            || private_query_corpus_summary_bool(&report, "contains_sample_ids")?
        {
            return Err(BenchmarkError::invalid_config(
                "private_query_corpus_summary_boundary",
            ));
        }

        let document_count = private_query_corpus_summary_usize(&report, "document_count")?;
        let searchable_document_count =
            private_query_corpus_summary_usize(&report, "searchable_document_count")?;
        let vector_indexed_document_count =
            private_query_corpus_summary_usize(&report, "vector_indexed_document_count")?;
        let hot_index_fully_covered =
            private_query_corpus_summary_bool(&report, "hot_index_fully_covered")?;
        let coverage_counts_are_valid = document_count > 0
            && searchable_document_count > 0
            && vector_indexed_document_count > 0
            && searchable_document_count <= document_count
            && vector_indexed_document_count <= document_count;
        let coverage_counts_are_complete = searchable_document_count == document_count
            && vector_indexed_document_count == document_count;
        if !coverage_counts_are_valid
            || hot_index_fully_covered != coverage_counts_are_complete
            || (!allow_partial_hot_index_for_smoke && !coverage_counts_are_complete)
        {
            return Err(BenchmarkError::invalid_config(
                "private_query_corpus_summary_hot_index",
            ));
        }

        Ok(Self {
            document_count,
            searchable_document_count,
            vector_indexed_document_count,
            sha256: sha256_hex(bytes),
        })
    }

    pub fn document_count(&self) -> usize {
        self.document_count
    }

    pub fn searchable_document_count(&self) -> usize {
        self.searchable_document_count
    }

    pub fn vector_indexed_document_count(&self) -> usize {
        self.vector_indexed_document_count
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateQueryBenchmarkConfig {
    query_set: PathBuf,
    command: PrivateQueryBenchmarkCommand,
    corpus_summary: PrivateQueryCorpusSummary,
    max_queries: usize,
    top_k: usize,
    timeout_ms: u64,
    index_size_bytes: u64,
    manifests: PrivateQueryManifestDigests,
}

impl PrivateQueryBenchmarkConfig {
    pub fn new(
        query_set: impl AsRef<Path>,
        command: PrivateQueryBenchmarkCommand,
        corpus_summary: PrivateQueryCorpusSummary,
        manifests: PrivateQueryManifestDigests,
    ) -> Result<Self> {
        let query_set = query_set.as_ref().to_path_buf();
        if query_set.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config("private_query_set"));
        }
        Ok(Self {
            query_set,
            command,
            corpus_summary,
            max_queries: DEFAULT_PRIVATE_QUERY_MAX_QUERIES,
            top_k: DEFAULT_TOP_K,
            timeout_ms: DEFAULT_PRIVATE_QUERY_TIMEOUT_MS,
            index_size_bytes: 0,
            manifests,
        })
    }

    pub fn with_max_queries(mut self, max_queries: usize) -> Result<Self> {
        if max_queries == 0 {
            return Err(BenchmarkError::invalid_config("private_query_max_queries"));
        }
        self.max_queries = max_queries;
        Ok(self)
    }

    pub fn with_top_k(mut self, top_k: usize) -> Result<Self> {
        if top_k == 0 || top_k > MAX_TOP_K {
            return Err(BenchmarkError::invalid_config("private_query_top_k"));
        }
        self.top_k = top_k;
        Ok(self)
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Result<Self> {
        if timeout_ms == 0 {
            return Err(BenchmarkError::invalid_config("private_query_timeout_ms"));
        }
        self.timeout_ms = timeout_ms;
        Ok(self)
    }

    pub fn with_index_size_bytes(mut self, index_size_bytes: u64) -> Self {
        self.index_size_bytes = index_size_bytes;
        self
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
pub enum PrivateOcrBenchmarkEngine {
    LocalCommand { command: PathBuf },
    Tesseract { command: PathBuf },
}

impl PrivateOcrBenchmarkEngine {
    pub fn local_command(command: impl AsRef<Path>) -> Result<Self> {
        let command = command.as_ref().to_path_buf();
        if command.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config("private_ocr_command"));
        }
        Ok(Self::LocalCommand { command })
    }

    pub fn tesseract(command: impl AsRef<Path>) -> Result<Self> {
        let command = command.as_ref().to_path_buf();
        if command.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config(
                "private_ocr_tesseract_command",
            ));
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

impl fmt::Debug for PrivateOcrBenchmarkEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateOcrBenchmarkEngine")
            .field("engine_kind", &self.engine_kind())
            .field("command", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum PrivatePdfRenderEngine {
    LocalCommand { command: PathBuf },
    Pdftoppm { command: PathBuf },
}

impl PrivatePdfRenderEngine {
    pub fn local_command(command: impl AsRef<Path>) -> Result<Self> {
        let command = command.as_ref().to_path_buf();
        if command.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config(
                "private_ocr_renderer_command",
            ));
        }
        Ok(Self::LocalCommand { command })
    }

    pub fn pdftoppm(command: impl AsRef<Path>) -> Result<Self> {
        let command = command.as_ref().to_path_buf();
        if command.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config(
                "private_ocr_pdftoppm_command",
            ));
        }
        Ok(Self::Pdftoppm { command })
    }
}

impl fmt::Debug for PrivatePdfRenderEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivatePdfRenderEngine")
            .field("renderer_kind", &private_pdf_renderer_kind(self))
            .field("command", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateOcrManifestDigests {
    dataset_manifest_sha256: String,
    ocr_runtime_manifest_sha256: String,
    renderer_manifest_sha256: String,
    language_pack_manifest_sha256: String,
}

impl PrivateOcrManifestDigests {
    pub fn new(
        dataset_manifest_sha256: impl Into<String>,
        ocr_runtime_manifest_sha256: impl Into<String>,
        renderer_manifest_sha256: impl Into<String>,
        language_pack_manifest_sha256: impl Into<String>,
    ) -> Result<Self> {
        let digests = Self {
            dataset_manifest_sha256: dataset_manifest_sha256.into(),
            ocr_runtime_manifest_sha256: ocr_runtime_manifest_sha256.into(),
            renderer_manifest_sha256: renderer_manifest_sha256.into(),
            language_pack_manifest_sha256: language_pack_manifest_sha256.into(),
        };
        if !is_sha256_hex(&digests.dataset_manifest_sha256)
            || !is_sha256_hex(&digests.ocr_runtime_manifest_sha256)
            || !is_sha256_hex(&digests.renderer_manifest_sha256)
            || !is_sha256_hex(&digests.language_pack_manifest_sha256)
        {
            return Err(BenchmarkError::invalid_config(
                "private_ocr_manifest_sha256",
            ));
        }
        Ok(digests)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateOcrThroughputConfig {
    root: PathBuf,
    ocr_engine: PrivateOcrBenchmarkEngine,
    renderer: PrivatePdfRenderEngine,
    manifests: PrivateOcrManifestDigests,
    max_documents: usize,
    max_pages: usize,
    pages_per_document: usize,
    page_timeout_ms: u64,
    max_run_ms: Option<u64>,
    render_dpi: u32,
    ocr_lang: String,
    engine_profile: String,
}

impl PrivateOcrThroughputConfig {
    pub fn new(
        root: impl AsRef<Path>,
        ocr_engine: PrivateOcrBenchmarkEngine,
        renderer: PrivatePdfRenderEngine,
        manifests: PrivateOcrManifestDigests,
    ) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        if root.as_os_str().is_empty() {
            return Err(BenchmarkError::invalid_config("private_ocr_root"));
        }
        Ok(Self {
            root,
            ocr_engine,
            renderer,
            manifests,
            max_documents: DEFAULT_PRIVATE_OCR_MAX_DOCUMENTS,
            max_pages: DEFAULT_PRIVATE_OCR_MAX_PAGES,
            pages_per_document: DEFAULT_PRIVATE_OCR_PAGES_PER_DOCUMENT,
            page_timeout_ms: 30_000,
            max_run_ms: None,
            render_dpi: DEFAULT_SYNTHETIC_OCR_RENDER_DPI,
            ocr_lang: DEFAULT_PRIVATE_OCR_LANG.to_string(),
            engine_profile: DEFAULT_PRIVATE_OCR_PROFILE.to_string(),
        })
    }

    pub fn with_max_documents(mut self, max_documents: usize) -> Result<Self> {
        if max_documents == 0 {
            return Err(BenchmarkError::invalid_config("private_ocr_max_documents"));
        }
        self.max_documents = max_documents;
        Ok(self)
    }

    pub fn with_max_pages(mut self, max_pages: usize) -> Result<Self> {
        if max_pages == 0 {
            return Err(BenchmarkError::invalid_config("private_ocr_max_pages"));
        }
        self.max_pages = max_pages;
        Ok(self)
    }

    pub fn with_pages_per_document(mut self, pages_per_document: usize) -> Result<Self> {
        if pages_per_document == 0 {
            return Err(BenchmarkError::invalid_config(
                "private_ocr_pages_per_document",
            ));
        }
        self.pages_per_document = pages_per_document;
        Ok(self)
    }

    pub fn with_page_timeout_ms(mut self, page_timeout_ms: u64) -> Result<Self> {
        if page_timeout_ms == 0 {
            return Err(BenchmarkError::invalid_config(
                "private_ocr_page_timeout_ms",
            ));
        }
        self.page_timeout_ms = page_timeout_ms;
        Ok(self)
    }

    pub fn with_max_run_ms(mut self, max_run_ms: u64) -> Result<Self> {
        if max_run_ms == 0 {
            return Err(BenchmarkError::invalid_config("private_ocr_max_run_ms"));
        }
        self.max_run_ms = Some(max_run_ms);
        Ok(self)
    }

    pub fn with_render_dpi(mut self, render_dpi: u32) -> Result<Self> {
        if render_dpi == 0 {
            return Err(BenchmarkError::invalid_config("private_ocr_render_dpi"));
        }
        self.render_dpi = render_dpi;
        Ok(self)
    }

    pub fn with_ocr_lang(mut self, ocr_lang: impl Into<String>) -> Result<Self> {
        let ocr_lang = ocr_lang.into();
        OcrOptions::new(ocr_lang.as_str(), self.engine_profile.as_str())
            .map_err(BenchmarkError::ocr)?;
        self.ocr_lang = ocr_lang;
        Ok(self)
    }

    pub fn with_engine_profile(mut self, engine_profile: impl Into<String>) -> Result<Self> {
        let engine_profile = engine_profile.into();
        OcrOptions::new(self.ocr_lang.as_str(), engine_profile.as_str())
            .map_err(BenchmarkError::ocr)?;
        self.engine_profile = engine_profile;
        Ok(self)
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
                "\"generation_mode\":\"streaming\",",
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
pub struct PrivateQueryBenchmarkReport {
    run_id: String,
    platform: String,
    document_count: usize,
    searchable_document_count: usize,
    vector_indexed_document_count: usize,
    corpus_summary_sha256: String,
    query_count: usize,
    top_k: usize,
    query_total_ms: f64,
    index_size_bytes: u64,
    zero_result_queries: usize,
    total_hits: usize,
    latency: LatencySummary,
    million_scale_verified: bool,
    percentile_confidence: &'static str,
    manifests: PrivateQueryManifestDigests,
}

impl PrivateQueryBenchmarkReport {
    pub fn document_count(&self) -> usize {
        self.document_count
    }

    pub fn searchable_document_count(&self) -> usize {
        self.searchable_document_count
    }

    pub fn vector_indexed_document_count(&self) -> usize {
        self.vector_indexed_document_count
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

    pub fn zero_result_queries(&self) -> usize {
        self.zero_result_queries
    }

    pub fn latency(&self) -> &LatencySummary {
        &self.latency
    }

    pub fn to_redacted_json(&self) -> String {
        format!(
            concat!(
                "{{",
                "\"schema_version\":\"benchmark.v1\",",
                "\"run_id\":\"{}\",",
                "\"platform\":\"{}\",",
                "\"dataset_kind\":\"private-real-corpus\",",
                "\"document_count\":{},",
                "\"searchable_document_count\":{},",
                "\"vector_indexed_document_count\":{},",
                "\"corpus_summary_sha256\":\"{}\",",
                "\"query_count\":{},",
                "\"top_k\":{},",
                "\"build_ms\":0.000,",
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
                "\"target_claim\":\"benchmark_baseline_observed\",",
                "\"corpus_origin\":\"private_local\",",
                "\"privacy_boundary\":\"redacted_local_aggregate\",",
                "\"query_protocol\":\"resume-ir-query-v1\",",
                "\"query_mode\":\"hybrid\",",
                "\"retrieval_layers\":\"fulltext+field+vector+rrf\",",
                "\"hot_index\":true,",
                "\"hot_path_ocr\":false,",
                "\"hot_path_parsing\":false,",
                "\"hot_path_heavy_model_inference\":false,",
                "\"contains_raw_resume_text\":false,",
                "\"contains_resume_paths\":false,",
                "\"contains_queries\":false,",
                "\"dataset_manifest_sha256\":\"{}\",",
                "\"query_set_sha256\":\"{}\",",
                "\"model_manifest_sha256\":\"{}\",",
                "\"scope\":\"private local real-corpus query benchmark; aggregate redacted report only\"",
                "}}"
            ),
            self.run_id,
            self.platform,
            self.document_count,
            self.searchable_document_count,
            self.vector_indexed_document_count,
            self.corpus_summary_sha256,
            self.query_count,
            self.top_k,
            format_consistency_number(self.query_total_ms),
            format_consistency_number(self.qps()),
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
            self.manifests.dataset_manifest_sha256,
            self.manifests.query_set_sha256,
            self.manifests.model_manifest_sha256,
        )
    }
}

impl fmt::Debug for PrivateQueryBenchmarkReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateQueryBenchmarkReport")
            .field("run_id", &self.run_id)
            .field("platform", &self.platform)
            .field("dataset_kind", &"private-real-corpus")
            .field("document_count", &self.document_count)
            .field("searchable_document_count", &self.searchable_document_count)
            .field(
                "vector_indexed_document_count",
                &self.vector_indexed_document_count,
            )
            .field("corpus_summary_sha256", &self.corpus_summary_sha256)
            .field("query_count", &self.query_count)
            .field("top_k", &self.top_k)
            .field("query_total_ms", &self.query_total_ms)
            .field("index_size_bytes", &self.index_size_bytes)
            .field("zero_result_queries", &self.zero_result_queries)
            .field("total_hits", &self.total_hits)
            .field("latency", &self.latency)
            .field("million_scale_verified", &self.million_scale_verified)
            .field("percentile_confidence", &self.percentile_confidence)
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
            format_consistency_number(self.total_ms),
            format_consistency_number(self.pages_per_second()),
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
pub struct PrivateOcrThroughputReport {
    run_id: String,
    platform: String,
    page_count: usize,
    document_count: usize,
    scanned_document_count: usize,
    failed_document_count: usize,
    render_failure_count: usize,
    ocr_failure_count: usize,
    run_budget_exhausted: bool,
    engine_kind: &'static str,
    total_ms: f64,
    latency: LatencySummary,
    manifests: PrivateOcrManifestDigests,
}

impl PrivateOcrThroughputReport {
    pub fn page_count(&self) -> usize {
        self.page_count
    }

    pub fn document_count(&self) -> usize {
        self.document_count
    }

    pub fn scanned_document_count(&self) -> usize {
        self.scanned_document_count
    }

    pub fn failed_document_count(&self) -> usize {
        self.failed_document_count
    }

    pub fn render_failure_count(&self) -> usize {
        self.render_failure_count
    }

    pub fn ocr_failure_count(&self) -> usize {
        self.ocr_failure_count
    }

    pub fn run_budget_exhausted(&self) -> bool {
        self.run_budget_exhausted
    }

    pub fn engine_kind(&self) -> &'static str {
        self.engine_kind
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

    fn target_claim(&self) -> &'static str {
        if self.page_count >= PRIVATE_REAL_OCR_THROUGHPUT_RELEASE_MIN_PAGES
            && !self.run_budget_exhausted
            && self.latency.p95_ms <= PRIVATE_REAL_OCR_THROUGHPUT_RELEASE_MAX_P95_MS
            && self.pages_per_second() >= PRIVATE_REAL_OCR_THROUGHPUT_RELEASE_MIN_PAGES_PER_SECOND
        {
            PRIVATE_REAL_OCR_THROUGHPUT_TARGET_CLAIM
        } else {
            "not_evaluated"
        }
    }

    pub fn to_redacted_json(&self) -> String {
        format!(
            concat!(
                "{{",
                "\"schema_version\":\"ocr-throughput.v1\",",
                "\"run_id\":\"{}\",",
                "\"platform\":\"{}\",",
                "\"dataset_kind\":\"private-real-corpus\",",
                "\"page_count\":{},",
                "\"document_count\":{},",
                "\"scanned_document_count\":{},",
                "\"failed_document_count\":{},",
                "\"render_failure_count\":{},",
                "\"ocr_failure_count\":{},",
                "\"run_budget_exhausted\":{},",
                "\"engine_kind\":\"{}\",",
                "\"total_ms\":{},",
                "\"page_latency_ms\":{{",
                "\"samples\":{},",
                "\"p50\":{},",
                "\"p95\":{},",
                "\"p99\":{}",
                "}},",
                "\"pages_per_second\":{},",
                "\"target_claim\":\"{}\",",
                "\"corpus_origin\":\"private_local\",",
                "\"privacy_boundary\":\"redacted_local_aggregate\",",
                "\"contains_raw_ocr_text\":false,",
                "\"contains_page_images\":false,",
                "\"contains_resume_paths\":false,",
                "\"contains_document_ids\":false,",
                "\"contains_page_ids\":false,",
                "\"contains_command_paths\":false,",
                "\"dataset_manifest_sha256\":\"{}\",",
                "\"ocr_runtime_manifest_sha256\":\"{}\",",
                "\"renderer_manifest_sha256\":\"{}\",",
                "\"language_pack_manifest_sha256\":\"{}\",",
                "\"scope\":\"private real-corpus OCR throughput benchmark; aggregate redacted report only\"",
                "}}"
            ),
            self.run_id,
            self.platform,
            self.page_count,
            self.document_count,
            self.scanned_document_count,
            self.failed_document_count,
            self.render_failure_count,
            self.ocr_failure_count,
            self.run_budget_exhausted,
            self.engine_kind,
            format_consistency_number(self.total_ms),
            self.latency.samples,
            format_ms(self.latency.p50_ms),
            format_ms(self.latency.p95_ms),
            format_ms(self.latency.p99_ms),
            format_consistency_number(self.pages_per_second()),
            self.target_claim(),
            self.manifests.dataset_manifest_sha256,
            self.manifests.ocr_runtime_manifest_sha256,
            self.manifests.renderer_manifest_sha256,
            self.manifests.language_pack_manifest_sha256,
        )
    }
}

impl fmt::Debug for PrivateOcrThroughputReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrivateOcrThroughputReport")
            .field("run_id", &self.run_id)
            .field("platform", &self.platform)
            .field("dataset_kind", &"private-real-corpus")
            .field("page_count", &self.page_count)
            .field("document_count", &self.document_count)
            .field("scanned_document_count", &self.scanned_document_count)
            .field("failed_document_count", &self.failed_document_count)
            .field("render_failure_count", &self.render_failure_count)
            .field("ocr_failure_count", &self.ocr_failure_count)
            .field("run_budget_exhausted", &self.run_budget_exhausted)
            .field("engine_kind", &self.engine_kind)
            .field("total_ms", &self.total_ms)
            .field("latency", &self.latency)
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
    private_boundary: Option<PrivateVectorQualityBoundary>,
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
        if let Some(boundary) = &self.private_boundary {
            return format!(
                concat!(
                    "{{",
                    "\"schema_version\":\"vector-quality.v1\",",
                    "\"run_id\":\"{}\",",
                    "\"platform\":\"{}\",",
                    "\"dataset_kind\":\"{}\",",
                    "\"sample_count\":{},",
                    "\"candidate_count\":{},",
                    "\"top_k\":{},",
                    "\"recall_at_k\":{},",
                    "\"mrr\":{},",
                    "\"ndcg_at_k\":{},",
                    "\"zero_recall_queries\":{},",
                    "\"target_claim\":\"{}\",",
                    "\"corpus_origin\":\"private_local\",",
                    "\"privacy_boundary\":\"redacted_local_aggregate\",",
                    "\"contains_raw_queries\":false,",
                    "\"contains_candidate_text\":false,",
                    "\"contains_resume_paths\":false,",
                    "\"contains_sample_ids\":false,",
                    "\"contains_candidate_ids\":false,",
                    "\"contains_vectors\":false,",
                    "\"dataset_manifest_sha256\":\"{}\",",
                    "\"annotation_manifest_sha256\":\"{}\",",
                    "\"model_manifest_sha256\":\"{}\",",
                    "\"vector_taxonomy\":\"resume-ir.vector-quality.v1\",",
                    "\"scope\":\"{}\"",
                    "}}"
                ),
                self.run_id,
                self.platform,
                self.dataset_kind,
                self.sample_count,
                self.candidate_count,
                self.top_k,
                format_ms(self.recall_at_k),
                format_ms(self.mrr),
                format_ms(self.ndcg_at_k),
                self.zero_recall_queries,
                self.target_claim,
                boundary.manifests.dataset_manifest_sha256,
                boundary.manifests.annotation_manifest_sha256,
                boundary.manifests.model_manifest_sha256,
                PRIVATE_BUSINESS_VECTOR_QUALITY_SCOPE,
            );
        }
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct PrivateVectorQualityBoundary {
    manifests: PrivateVectorQualityManifestDigests,
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
            .field("private_boundary", &self.private_boundary.is_some())
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
    index
        .replace_documents((0..config.document_count).map(synthetic_document))
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

pub fn run_private_query_benchmark(
    config: PrivateQueryBenchmarkConfig,
) -> Result<PrivateQueryBenchmarkReport> {
    let queries = load_private_query_set(&config.query_set, config.max_queries)?;
    let scratch = create_private_query_scratch_dir()?;
    let _scratch_guard = PrivateQueryScratchGuard(scratch.clone());
    let query_file = scratch.join("query.txt");

    let mut latencies = Vec::with_capacity(queries.len());
    let query_batch_started = Instant::now();
    let mut total_hits = 0_usize;
    let mut zero_result_queries = 0_usize;
    for query in &queries {
        write_private_query_file(&query_file, query)?;
        let query_started = Instant::now();
        let hits = run_private_query_command(
            &config.command,
            &query_file,
            config.top_k,
            config.timeout_ms,
        )?;
        latencies.push(elapsed_ms(query_started));
        if hits == 0 {
            zero_result_queries += 1;
        }
        total_hits = total_hits.saturating_add(hits);
    }
    let query_total_ms = elapsed_ms(query_batch_started);

    Ok(PrivateQueryBenchmarkReport {
        run_id: generate_run_id(),
        platform: platform_label(),
        document_count: config.corpus_summary.document_count(),
        searchable_document_count: config.corpus_summary.searchable_document_count(),
        vector_indexed_document_count: config.corpus_summary.vector_indexed_document_count(),
        corpus_summary_sha256: config.corpus_summary.sha256().to_string(),
        query_count: queries.len(),
        top_k: config.top_k,
        query_total_ms,
        index_size_bytes: config.index_size_bytes,
        zero_result_queries,
        total_hits,
        latency: LatencySummary::from_samples(latencies)?,
        million_scale_verified: config.corpus_summary.document_count() >= 1_000_000,
        percentile_confidence: private_query_percentile_confidence(
            config.corpus_summary.document_count(),
            queries.len(),
        ),
        manifests: config.manifests,
    })
}

struct PrivateQueryScratchGuard(PathBuf);

impl Drop for PrivateQueryScratchGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
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

pub fn run_private_ocr_throughput_benchmark(
    config: PrivateOcrThroughputConfig,
) -> Result<PrivateOcrThroughputReport> {
    let engine_kind = config.ocr_engine.engine_kind();
    let client: Box<dyn OcrClient> = match &config.ocr_engine {
        PrivateOcrBenchmarkEngine::LocalCommand { command } => {
            let spec =
                LocalOcrCommandSpec::new(command, Vec::<String>::new(), &config.engine_profile)
                    .map_err(BenchmarkError::ocr)?;
            Box::new(LocalOcrCommandClient::new(spec))
        }
        PrivateOcrBenchmarkEngine::Tesseract { command } => {
            let spec = TesseractOcrSpec::new(command, &config.engine_profile)
                .map_err(BenchmarkError::ocr)?;
            Box::new(TesseractOcrClient::new(spec))
        }
    };
    let renderer = PrivatePdfRendererClient::new(&config.renderer)?;
    let budget = OcrWorkerBudget::new(config.page_timeout_ms).map_err(BenchmarkError::ocr)?;
    let options =
        OcrOptions::new(&config.ocr_lang, &config.engine_profile).map_err(BenchmarkError::ocr)?;
    let cancellation = CancellationToken::new();
    let documents = collect_private_pdf_documents(&config.root, config.max_documents)?;
    if documents.is_empty() {
        return Err(BenchmarkError::invalid_config("private_ocr_pdf_documents"));
    }

    let run_started = Instant::now();
    let mut page_latencies = Vec::new();
    let mut document_count = 0_usize;
    let mut scanned_document_count = 0_usize;
    let mut failed_document_count = 0_usize;
    let mut render_failure_count = 0_usize;
    let mut ocr_failure_count = 0_usize;
    let mut run_budget_exhausted = false;

    for document_path in documents {
        if page_latencies.len() >= config.max_pages || run_budget_exhausted {
            break;
        }
        document_count += 1;
        let document_bytes = fs::read(&document_path).map_err(BenchmarkError::io)?;
        let mut document_pages = 0_usize;
        let mut document_failed = false;

        for page_index in 0..config.pages_per_document {
            if page_latencies.len() >= config.max_pages {
                break;
            }
            let page_no = u32::try_from(page_index + 1)
                .map_err(|_| BenchmarkError::invalid_config("private_ocr_page_no"))?;
            let page_started = Instant::now();
            let rendered_page = match renderer.render_page(
                &document_bytes,
                page_no,
                config.render_dpi,
                budget,
                &cancellation,
            ) {
                Ok(rendered_page) => rendered_page,
                Err(_) => {
                    render_failure_count += 1;
                    document_failed = true;
                    break;
                }
            };
            let request =
                OcrPageRequest::new(rendered_page, options.clone()).map_err(BenchmarkError::ocr)?;
            if client
                .recognize_page(request, budget, &cancellation)
                .is_err()
            {
                ocr_failure_count += 1;
                document_failed = true;
                break;
            }
            page_latencies.push(elapsed_ms(page_started));
            document_pages += 1;
            if private_ocr_run_budget_exhausted(run_started, config.max_run_ms) {
                run_budget_exhausted = true;
                break;
            }
        }

        if document_pages > 0 {
            scanned_document_count += 1;
        }
        if document_failed {
            failed_document_count += 1;
        }
    }

    if page_latencies.is_empty() || document_count == 0 || scanned_document_count == 0 {
        return Err(BenchmarkError::invalid_config("private_ocr_page_count"));
    }

    Ok(PrivateOcrThroughputReport {
        run_id: generate_run_id(),
        platform: platform_label(),
        page_count: page_latencies.len(),
        document_count,
        scanned_document_count,
        failed_document_count,
        render_failure_count,
        ocr_failure_count,
        run_budget_exhausted,
        engine_kind,
        total_ms: elapsed_ms(run_started),
        latency: LatencySummary::from_samples(page_latencies)?,
        manifests: config.manifests,
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
        private_boundary: None,
    })
}

pub fn run_private_business_vector_quality_jsonl(
    dataset_jsonl: &str,
    config: VectorQualityConfig,
    manifests: PrivateVectorQualityManifestDigests,
) -> Result<VectorQualityReport> {
    let mut report = run_vector_quality_jsonl(dataset_jsonl, config)?;
    report.dataset_kind = "private-business-labeled";
    report.target_claim = PRIVATE_BUSINESS_VECTOR_QUALITY_TARGET_CLAIM;
    report.private_boundary = Some(PrivateVectorQualityBoundary { manifests });
    Ok(report)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BenchmarkGateConfig {
    min_documents: usize,
    min_queries: usize,
    max_p95_ms: f64,
    max_zero_result_queries: usize,
    allow_synthetic: bool,
    allow_smoke_confidence: bool,
    require_private_real_corpus: bool,
    require_million_scale: bool,
}

impl BenchmarkGateConfig {
    pub fn new(min_documents: usize, min_queries: usize, max_p95_ms: f64) -> Self {
        Self {
            min_documents,
            min_queries,
            max_p95_ms,
            max_zero_result_queries: 0,
            allow_synthetic: false,
            allow_smoke_confidence: false,
            require_private_real_corpus: false,
            require_million_scale: false,
        }
    }

    pub fn allow_synthetic(mut self) -> Self {
        self.allow_synthetic = true;
        self
    }

    pub fn allow_smoke_confidence(mut self) -> Self {
        self.allow_smoke_confidence = true;
        self
    }

    pub fn require_private_real_corpus(mut self) -> Self {
        self.require_private_real_corpus = true;
        self
    }

    pub fn require_million_scale(mut self) -> Self {
        self.require_million_scale = true;
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
    require_private_real_corpus: bool,
}

impl OcrThroughputGateConfig {
    pub fn new(min_pages: usize, max_p95_ms: f64, min_pages_per_second: f64) -> Self {
        Self {
            min_pages,
            max_p95_ms,
            min_pages_per_second,
            allow_synthetic: false,
            require_private_real_corpus: false,
        }
    }

    pub fn allow_synthetic(mut self) -> Self {
        self.allow_synthetic = true;
        self
    }

    pub fn require_private_real_corpus(mut self) -> Self {
        self.require_private_real_corpus = true;
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
    require_private_business_labeled: bool,
}

impl FieldQualityGateConfig {
    pub fn new(min_precision: f64, min_recall: f64, min_f1: f64) -> Self {
        Self {
            min_precision,
            min_recall,
            min_f1,
            min_samples: 1,
            require_private_business_labeled: false,
        }
    }

    pub fn with_min_samples(mut self, min_samples: usize) -> Self {
        self.min_samples = min_samples;
        self
    }

    pub fn require_private_business_labeled(mut self) -> Self {
        self.require_private_business_labeled = true;
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
pub struct DedupeQualityGateConfig {
    min_precision: f64,
    min_recall: f64,
    min_f1: f64,
    min_pairs: usize,
    min_positive_pairs: usize,
    require_private_business_labeled: bool,
}

impl DedupeQualityGateConfig {
    pub fn new(min_precision: f64, min_recall: f64, min_f1: f64) -> Self {
        Self {
            min_precision,
            min_recall,
            min_f1,
            min_pairs: 1,
            min_positive_pairs: 1,
            require_private_business_labeled: false,
        }
    }

    pub fn with_min_pairs(mut self, min_pairs: usize) -> Self {
        self.min_pairs = min_pairs;
        self
    }

    pub fn with_min_positive_pairs(mut self, min_positive_pairs: usize) -> Self {
        self.min_positive_pairs = min_positive_pairs;
        self
    }

    pub fn require_private_business_labeled(mut self) -> Self {
        self.require_private_business_labeled = true;
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DedupeQualityGateEvaluation {
    dataset_kind: String,
    pair_count: usize,
    precision: f64,
    recall: f64,
    f1: f64,
}

impl DedupeQualityGateEvaluation {
    pub fn dataset_kind(&self) -> &str {
        &self.dataset_kind
    }

    pub fn pair_count(&self) -> usize {
        self.pair_count
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
    require_private_business_labeled: bool,
}

impl VectorQualityGateConfig {
    pub fn new(min_samples: usize, min_recall_at_k: f64, min_mrr: f64, min_ndcg_at_k: f64) -> Self {
        Self {
            min_samples,
            min_recall_at_k,
            min_mrr,
            min_ndcg_at_k,
            max_zero_recall_queries: 0,
            require_private_business_labeled: false,
        }
    }

    pub fn with_max_zero_recall_queries(mut self, max_zero_recall_queries: usize) -> Self {
        self.max_zero_recall_queries = max_zero_recall_queries;
        self
    }

    pub fn require_private_business_labeled(mut self) -> Self {
        self.require_private_business_labeled = true;
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct PrivateFieldQualityBoundary {
    manifests: PrivateFieldQualityManifestDigests,
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
    private_boundary: Option<PrivateFieldQualityBoundary>,
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
        let private_boundary_json = self
            .private_boundary
            .as_ref()
            .map(|boundary| {
                format!(
                    concat!(
                        "\"corpus_origin\":\"private_local\",",
                        "\"privacy_boundary\":\"redacted_local_aggregate\",",
                        "\"contains_raw_resume_text\":false,",
                        "\"contains_resume_paths\":false,",
                        "\"contains_field_values\":false,",
                        "\"contains_sample_ids\":false,",
                        "\"dataset_manifest_sha256\":\"{}\",",
                        "\"annotation_manifest_sha256\":\"{}\",",
                        "\"field_taxonomy\":\"resume-ir.fields.v1\","
                    ),
                    boundary.manifests.dataset_manifest_sha256,
                    boundary.manifests.annotation_manifest_sha256,
                )
            })
            .unwrap_or_default();
        let scope = if self.private_boundary.is_some() {
            PRIVATE_BUSINESS_FIELD_QUALITY_SCOPE
        } else {
            "labeled field extraction quality; no raw resume text, paths, sample ids, or field values included"
        };
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
                "{}",
                "\"scope\":\"{}\"",
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
            private_boundary_json,
            scope,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DedupeQualityCounts {
    true_positive: usize,
    false_positive: usize,
    false_negative: usize,
    true_negative: usize,
}

impl DedupeQualityCounts {
    fn record(&mut self, expected_duplicate: bool, predicted_duplicate: bool) {
        match (expected_duplicate, predicted_duplicate) {
            (true, true) => self.true_positive += 1,
            (false, true) => self.false_positive += 1,
            (true, false) => self.false_negative += 1,
            (false, false) => self.true_negative += 1,
        }
    }

    fn pair_count(self) -> usize {
        self.true_positive + self.false_positive + self.false_negative + self.true_negative
    }

    fn positive_pair_count(self) -> usize {
        self.true_positive + self.false_negative
    }

    fn predicted_duplicate_pairs(self) -> usize {
        self.true_positive + self.false_positive
    }

    fn precision(self) -> f64 {
        let denominator = self.true_positive + self.false_positive;
        if denominator == 0 {
            0.0
        } else {
            self.true_positive as f64 / denominator as f64
        }
    }

    fn recall(self) -> f64 {
        let denominator = self.true_positive + self.false_negative;
        if denominator == 0 {
            0.0
        } else {
            self.true_positive as f64 / denominator as f64
        }
    }

    fn f1(self) -> f64 {
        let precision = self.precision();
        let recall = self.recall();
        if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DedupeQualityReport {
    run_id: String,
    platform: String,
    dataset_kind: &'static str,
    counts: DedupeQualityCounts,
    target_claim: &'static str,
    private_boundary: Option<PrivateDedupeQualityBoundary>,
}

impl DedupeQualityReport {
    pub fn dataset_kind(&self) -> &'static str {
        self.dataset_kind
    }

    pub fn pair_count(&self) -> usize {
        self.counts.pair_count()
    }

    pub fn positive_pair_count(&self) -> usize {
        self.counts.positive_pair_count()
    }

    pub fn precision(&self) -> f64 {
        self.counts.precision()
    }

    pub fn recall(&self) -> f64 {
        self.counts.recall()
    }

    pub fn f1(&self) -> f64 {
        self.counts.f1()
    }

    pub fn to_redacted_json(&self) -> String {
        let private_boundary_json = self
            .private_boundary
            .as_ref()
            .map(|boundary| {
                format!(
                    concat!(
                        "\"corpus_origin\":\"private_local\",",
                        "\"privacy_boundary\":\"redacted_local_aggregate\",",
                        "\"contains_raw_resume_text\":false,",
                        "\"contains_resume_paths\":false,",
                        "\"contains_profile_values\":false,",
                        "\"contains_sample_ids\":false,",
                        "\"contains_document_ids\":false,",
                        "\"dataset_manifest_sha256\":\"{}\",",
                        "\"annotation_manifest_sha256\":\"{}\",",
                        "\"dedupe_taxonomy\":\"resume-ir.dedupe.v1\","
                    ),
                    boundary.manifests.dataset_manifest_sha256,
                    boundary.manifests.annotation_manifest_sha256,
                )
            })
            .unwrap_or_default();
        let scope = if self.private_boundary.is_some() {
            PRIVATE_BUSINESS_DEDUPE_QUALITY_SCOPE
        } else {
            "labeled dedupe quality; no names, schools, companies, skills, sample ids, document ids, paths, or raw resume text included"
        };
        format!(
            concat!(
                "{{",
                "\"schema_version\":\"dedupe-quality.v1\",",
                "\"run_id\":\"{}\",",
                "\"platform\":\"{}\",",
                "\"dataset_kind\":\"{}\",",
                "\"pair_count\":{},",
                "\"positive_pair_count\":{},",
                "\"predicted_duplicate_pairs\":{},",
                "\"true_positive\":{},",
                "\"false_positive\":{},",
                "\"false_negative\":{},",
                "\"true_negative\":{},",
                "\"precision\":{},",
                "\"recall\":{},",
                "\"f1\":{},",
                "\"target_claim\":\"{}\",",
                "{}",
                "\"scope\":\"{}\"",
                "}}"
            ),
            self.run_id,
            self.platform,
            self.dataset_kind,
            self.counts.pair_count(),
            self.counts.positive_pair_count(),
            self.counts.predicted_duplicate_pairs(),
            self.counts.true_positive,
            self.counts.false_positive,
            self.counts.false_negative,
            self.counts.true_negative,
            format_ms(self.counts.precision()),
            format_ms(self.counts.recall()),
            format_ms(self.counts.f1()),
            self.target_claim,
            private_boundary_json,
            scope,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PrivateDedupeQualityBoundary {
    manifests: PrivateDedupeQualityManifestDigests,
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
        private_boundary: None,
    })
}

pub fn run_private_business_field_quality_jsonl(
    dataset_jsonl: &str,
    manifests: PrivateFieldQualityManifestDigests,
) -> Result<FieldQualityReport> {
    let mut report = run_field_quality_jsonl(dataset_jsonl)?;
    report.dataset_kind = "private-business-labeled";
    report.target_claim = PRIVATE_BUSINESS_FIELD_QUALITY_TARGET_CLAIM;
    report.private_boundary = Some(PrivateFieldQualityBoundary { manifests });
    Ok(report)
}

pub fn run_dedupe_quality_jsonl(dataset_jsonl: &str) -> Result<DedupeQualityReport> {
    let mut counts = DedupeQualityCounts::default();

    for line in dataset_jsonl
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let sample = parse_dedupe_quality_sample(line)?;
        let predicted_duplicate =
            soft_dedupe_score(&sample.left, &sample.right).is_some_and(|score| {
                score.confidence() >= DEDUPE_QUALITY_DUPLICATE_CONFIDENCE_THRESHOLD
            });
        counts.record(sample.duplicate, predicted_duplicate);
    }

    if counts.pair_count() == 0 {
        return Err(BenchmarkError::invalid_config("dedupe_quality_pairs"));
    }
    if counts.positive_pair_count() == 0 {
        return Err(BenchmarkError::invalid_config(
            "dedupe_quality.positive_pairs",
        ));
    }

    Ok(DedupeQualityReport {
        run_id: generate_run_id(),
        platform: platform_label(),
        dataset_kind: "labeled",
        counts,
        target_claim: "not_evaluated",
        private_boundary: None,
    })
}

pub fn run_private_business_dedupe_quality_jsonl(
    dataset_jsonl: &str,
    manifests: PrivateDedupeQualityManifestDigests,
) -> Result<DedupeQualityReport> {
    let mut report = run_dedupe_quality_jsonl(dataset_jsonl)?;
    report.dataset_kind = "private-business-labeled";
    report.target_claim = PRIVATE_BUSINESS_DEDUPE_QUALITY_TARGET_CLAIM;
    report.private_boundary = Some(PrivateDedupeQualityBoundary { manifests });
    Ok(report)
}

const DEDUPE_QUALITY_DUPLICATE_CONFIDENCE_THRESHOLD: f32 = 0.70;
const PRIVATE_BUSINESS_FIELD_QUALITY_SCOPE: &str =
    "private business field-quality benchmark; aggregate redacted report only";
const PRIVATE_BUSINESS_FIELD_QUALITY_TARGET_CLAIM: &str = "field_quality_target_met";
const FIELD_QUALITY_SCORE_TOLERANCE: f64 = 0.000_5;
const PRODUCTION_FIELD_QUALITY_THRESHOLDS: &[(&str, f64)] = &[
    ("name", 0.95),
    ("email", 0.995),
    ("phone", 0.995),
    ("wechat", 0.99),
    ("school", 0.93),
    ("school_tier", 0.90),
    ("degree", 0.95),
    ("major", 0.90),
    ("company", 0.90),
    ("title", 0.88),
    ("location", 0.90),
    ("skill", 0.92),
    ("certificate", 0.90),
    ("date_range", 0.93),
    ("years_experience", 0.90),
];

pub fn evaluate_field_quality_gate_json(
    report_json: &str,
    config: FieldQualityGateConfig,
) -> std::result::Result<FieldQualityGateEvaluation, BenchmarkGateError> {
    reject_duplicate_json_object_keys(report_json)?;
    let report: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| BenchmarkGateError::invalid_json())?;

    let schema_version = required_str(&report, "schema_version")?;
    if schema_version != "field-quality.v1" {
        return Err(BenchmarkGateError::failed(
            "unsupported field quality schema",
        ));
    }
    let dataset_kind = required_str(&report, "dataset_kind")?;
    match dataset_kind {
        "labeled" | "private-business-labeled" => {}
        _ => {
            return Err(BenchmarkGateError::failed(
                "field quality requires labeled dataset",
            ));
        }
    }
    if config.require_private_business_labeled && dataset_kind != "private-business-labeled" {
        return Err(BenchmarkGateError::failed(
            "private business field-quality benchmark required",
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

    if dataset_kind == "private-business-labeled" {
        validate_private_business_field_quality_boundary(&report, target_claim)?;
    }
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
    if dataset_kind == "labeled" && target_claim != "not_evaluated" {
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

fn validate_private_business_field_quality_boundary(
    report: &serde_json::Value,
    target_claim: &str,
) -> std::result::Result<(), BenchmarkGateError> {
    validate_private_business_field_quality_shape(report)?;
    if private_field_quality_str(report, "corpus_origin")? != "private_local"
        || private_field_quality_str(report, "privacy_boundary")? != "redacted_local_aggregate"
        || private_field_quality_bool(report, "contains_raw_resume_text")?
        || private_field_quality_bool(report, "contains_resume_paths")?
        || private_field_quality_bool(report, "contains_field_values")?
        || private_field_quality_bool(report, "contains_sample_ids")?
        || !is_sha256_hex(private_field_quality_str(
            report,
            "dataset_manifest_sha256",
        )?)
        || !is_sha256_hex(private_field_quality_str(
            report,
            "annotation_manifest_sha256",
        )?)
        || private_field_quality_str(report, "field_taxonomy")? != "resume-ir.fields.v1"
        || private_field_quality_str(report, "scope")? != PRIVATE_BUSINESS_FIELD_QUALITY_SCOPE
    {
        return Err(private_field_quality_boundary_error());
    }
    if target_claim != PRIVATE_BUSINESS_FIELD_QUALITY_TARGET_CLAIM {
        return Err(BenchmarkGateError::failed(
            "private business field quality requires target claim",
        ));
    }
    if !is_safe_benchmark_token(private_field_quality_str(report, "run_id")?) {
        return Err(private_field_quality_boundary_error());
    }
    if !is_safe_platform_label(private_field_quality_str(report, "platform")?) {
        return Err(private_field_quality_boundary_error());
    }

    Ok(())
}

fn validate_private_business_field_quality_shape(
    report: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let Some(object) = report.as_object() else {
        return Err(private_field_quality_boundary_error());
    };
    for key in object.keys() {
        if !is_allowed_private_business_field_quality_key(key) {
            return Err(BenchmarkGateError::failed(
                "unsupported private business field quality field",
            ));
        }
    }
    private_field_quality_str(report, "schema_version")?;
    private_field_quality_str(report, "run_id")?;
    private_field_quality_str(report, "platform")?;
    private_field_quality_str(report, "dataset_kind")?;
    private_field_quality_usize(report, "sample_count")?;
    private_field_quality_usize(report, "expected_mentions")?;
    private_field_quality_usize(report, "predicted_mentions")?;
    private_field_quality_str(report, "target_claim")?;
    private_field_quality_str(report, "corpus_origin")?;
    private_field_quality_str(report, "privacy_boundary")?;
    private_field_quality_bool(report, "contains_raw_resume_text")?;
    private_field_quality_bool(report, "contains_resume_paths")?;
    private_field_quality_bool(report, "contains_field_values")?;
    private_field_quality_bool(report, "contains_sample_ids")?;
    private_field_quality_str(report, "dataset_manifest_sha256")?;
    private_field_quality_str(report, "annotation_manifest_sha256")?;
    private_field_quality_str(report, "field_taxonomy")?;
    private_field_quality_str(report, "scope")?;
    let overall = report
        .get("overall")
        .ok_or_else(private_field_quality_boundary_error)?;
    validate_private_business_field_quality_metric_shape(overall)?;
    let fields = report
        .get("fields")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(private_business_field_metric_error)?;
    validate_private_business_field_metrics(fields)?;
    Ok(())
}

fn validate_private_business_field_metrics(
    fields: &serde_json::Map<String, serde_json::Value>,
) -> std::result::Result<(), BenchmarkGateError> {
    for field_name in fields.keys() {
        if !PRODUCTION_FIELD_QUALITY_THRESHOLDS
            .iter()
            .any(|(required_field, _)| required_field == field_name)
        {
            return Err(BenchmarkGateError::failed(
                "unsupported private business field quality field",
            ));
        }
    }

    for (field_name, min_score) in PRODUCTION_FIELD_QUALITY_THRESHOLDS {
        let metric = fields
            .get(*field_name)
            .ok_or_else(private_business_field_metric_error)?;
        let metric = validate_private_business_field_quality_metric_shape(metric)?;
        let precision = metric.precision();
        let recall = metric.recall();
        let f1 = metric.f1();
        if precision < *min_score || recall < *min_score || f1 < *min_score {
            return Err(BenchmarkGateError::failed(
                "private business field quality below production field threshold",
            ));
        }
    }

    Ok(())
}

fn validate_private_business_field_quality_metric_shape(
    metric: &serde_json::Value,
) -> std::result::Result<FieldQualityMetric, BenchmarkGateError> {
    let Some(object) = metric.as_object() else {
        return Err(private_field_quality_boundary_error());
    };
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "true_positive" | "false_positive" | "false_negative" | "precision" | "recall" | "f1"
        ) {
            return Err(BenchmarkGateError::failed(
                "unsupported private business field quality metric field",
            ));
        }
    }
    let counts = FieldCounts {
        true_positive: private_field_quality_usize(metric, "true_positive")?,
        false_positive: private_field_quality_usize(metric, "false_positive")?,
        false_negative: private_field_quality_usize(metric, "false_negative")?,
    };
    let reported = FieldQualityMetric {
        true_positive: counts.true_positive,
        false_positive: counts.false_positive,
        false_negative: counts.false_negative,
        precision: private_field_quality_number(metric, "precision")?,
        recall: private_field_quality_number(metric, "recall")?,
        f1: private_field_quality_number(metric, "f1")?,
    };
    if counts.true_positive + counts.false_negative == 0 {
        return Err(BenchmarkGateError::failed(
            "private business field quality requires production field support",
        ));
    }
    let expected = counts.metric();
    if !field_quality_score_matches(reported.precision(), expected.precision())
        || !field_quality_score_matches(reported.recall(), expected.recall())
        || !field_quality_score_matches(reported.f1(), expected.f1())
    {
        return Err(BenchmarkGateError::failed(
            "private business field quality metric counts do not match scores",
        ));
    }
    Ok(reported)
}

fn field_quality_score_matches(reported: f64, expected: f64) -> bool {
    (reported - expected).abs() <= FIELD_QUALITY_SCORE_TOLERANCE
}

fn is_allowed_private_business_field_quality_key(key: &str) -> bool {
    matches!(
        key,
        "schema_version"
            | "run_id"
            | "platform"
            | "dataset_kind"
            | "sample_count"
            | "expected_mentions"
            | "predicted_mentions"
            | "overall"
            | "fields"
            | "target_claim"
            | "corpus_origin"
            | "privacy_boundary"
            | "contains_raw_resume_text"
            | "contains_resume_paths"
            | "contains_field_values"
            | "contains_sample_ids"
            | "dataset_manifest_sha256"
            | "annotation_manifest_sha256"
            | "field_taxonomy"
            | "scope"
    )
}

fn private_field_quality_str<'a>(
    value: &'a serde_json::Value,
    field: &'static str,
) -> std::result::Result<&'a str, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(private_field_quality_boundary_error)
}

fn private_field_quality_bool(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<bool, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(private_field_quality_boundary_error)
}

fn private_field_quality_usize(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<usize, BenchmarkGateError> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(private_field_quality_boundary_error)?;
    usize::try_from(number).map_err(|_| private_field_quality_boundary_error())
}

fn private_field_quality_number(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<f64, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
        .ok_or_else(private_field_quality_boundary_error)
}

fn private_field_quality_boundary_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private business field quality requires redacted local boundary")
}

fn private_business_field_metric_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private business field quality requires production field metrics")
}

const PRIVATE_BUSINESS_DEDUPE_QUALITY_SCOPE: &str =
    "private business dedupe-quality benchmark; aggregate redacted report only";
const PRIVATE_BUSINESS_DEDUPE_QUALITY_TARGET_CLAIM: &str = "dedupe_quality_target_met";
const DEDUPE_QUALITY_SCORE_TOLERANCE: f64 = 0.000_5;

pub fn evaluate_dedupe_quality_gate_json(
    report_json: &str,
    config: DedupeQualityGateConfig,
) -> std::result::Result<DedupeQualityGateEvaluation, BenchmarkGateError> {
    reject_duplicate_json_object_keys(report_json)?;
    let report: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| BenchmarkGateError::invalid_json())?;

    let schema_version = required_str(&report, "schema_version")?;
    if schema_version != "dedupe-quality.v1" {
        return Err(BenchmarkGateError::failed(
            "unsupported dedupe quality schema",
        ));
    }
    let dataset_kind = required_str(&report, "dataset_kind")?;
    match dataset_kind {
        "labeled" | "private-business-labeled" => {}
        _ => {
            return Err(BenchmarkGateError::failed(
                "dedupe quality requires labeled dataset",
            ));
        }
    }
    if config.require_private_business_labeled && dataset_kind != "private-business-labeled" {
        return Err(BenchmarkGateError::failed(
            "private business dedupe-quality benchmark required",
        ));
    }

    let pair_count = required_usize(&report, "pair_count")?;
    let positive_pair_count = required_usize(&report, "positive_pair_count")?;
    let precision = required_f64(&report, "precision")?;
    let recall = required_f64(&report, "recall")?;
    let f1 = required_f64(&report, "f1")?;
    let target_claim = required_str(&report, "target_claim")?;

    if dataset_kind == "private-business-labeled" {
        validate_private_business_dedupe_quality_boundary(&report, target_claim)?;
    }
    if pair_count < config.min_pairs {
        return Err(BenchmarkGateError::failed(
            "dedupe pair count below gate minimum",
        ));
    }
    if positive_pair_count < config.min_positive_pairs {
        return Err(BenchmarkGateError::failed(
            "dedupe positive pair count below gate minimum",
        ));
    }
    if precision < config.min_precision {
        return Err(BenchmarkGateError::failed(
            "dedupe precision below threshold",
        ));
    }
    if recall < config.min_recall {
        return Err(BenchmarkGateError::failed("dedupe recall below threshold"));
    }
    if f1 < config.min_f1 {
        return Err(BenchmarkGateError::failed("dedupe f1 below threshold"));
    }
    if dataset_kind == "labeled" && target_claim != "not_evaluated" {
        return Err(BenchmarkGateError::failed(
            "dedupe target claim is not proven",
        ));
    }

    Ok(DedupeQualityGateEvaluation {
        dataset_kind: dataset_kind.to_string(),
        pair_count,
        precision,
        recall,
        f1,
    })
}

fn validate_private_business_dedupe_quality_boundary(
    report: &serde_json::Value,
    target_claim: &str,
) -> std::result::Result<(), BenchmarkGateError> {
    validate_private_business_dedupe_quality_shape(report)?;
    if private_dedupe_quality_str(report, "corpus_origin")? != "private_local"
        || private_dedupe_quality_str(report, "privacy_boundary")? != "redacted_local_aggregate"
        || private_dedupe_quality_bool(report, "contains_raw_resume_text")?
        || private_dedupe_quality_bool(report, "contains_resume_paths")?
        || private_dedupe_quality_bool(report, "contains_profile_values")?
        || private_dedupe_quality_bool(report, "contains_sample_ids")?
        || private_dedupe_quality_bool(report, "contains_document_ids")?
        || !is_sha256_hex(private_dedupe_quality_str(
            report,
            "dataset_manifest_sha256",
        )?)
        || !is_sha256_hex(private_dedupe_quality_str(
            report,
            "annotation_manifest_sha256",
        )?)
        || private_dedupe_quality_str(report, "dedupe_taxonomy")? != "resume-ir.dedupe.v1"
        || private_dedupe_quality_str(report, "scope")? != PRIVATE_BUSINESS_DEDUPE_QUALITY_SCOPE
    {
        return Err(private_dedupe_quality_boundary_error());
    }
    if target_claim != PRIVATE_BUSINESS_DEDUPE_QUALITY_TARGET_CLAIM {
        return Err(BenchmarkGateError::failed(
            "private business dedupe quality requires target claim",
        ));
    }
    if !is_safe_benchmark_token(private_dedupe_quality_str(report, "run_id")?) {
        return Err(private_dedupe_quality_boundary_error());
    }
    if !is_safe_platform_label(private_dedupe_quality_str(report, "platform")?) {
        return Err(private_dedupe_quality_boundary_error());
    }
    Ok(())
}

fn validate_private_business_dedupe_quality_shape(
    report: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let Some(object) = report.as_object() else {
        return Err(private_dedupe_quality_boundary_error());
    };
    for key in object.keys() {
        if !is_allowed_private_business_dedupe_quality_key(key) {
            return Err(BenchmarkGateError::failed(
                "unsupported private business dedupe quality field",
            ));
        }
    }
    private_dedupe_quality_str(report, "schema_version")?;
    private_dedupe_quality_str(report, "run_id")?;
    private_dedupe_quality_str(report, "platform")?;
    private_dedupe_quality_str(report, "dataset_kind")?;
    let pair_count = private_dedupe_quality_usize(report, "pair_count")?;
    let positive_pair_count = private_dedupe_quality_usize(report, "positive_pair_count")?;
    let predicted_duplicate_pairs =
        private_dedupe_quality_usize(report, "predicted_duplicate_pairs")?;
    let counts = DedupeQualityCounts {
        true_positive: private_dedupe_quality_usize(report, "true_positive")?,
        false_positive: private_dedupe_quality_usize(report, "false_positive")?,
        false_negative: private_dedupe_quality_usize(report, "false_negative")?,
        true_negative: private_dedupe_quality_usize(report, "true_negative")?,
    };
    if pair_count != counts.pair_count()
        || positive_pair_count != counts.positive_pair_count()
        || predicted_duplicate_pairs != counts.predicted_duplicate_pairs()
    {
        return Err(BenchmarkGateError::failed(
            "private business dedupe quality counts are inconsistent",
        ));
    }
    let precision = private_dedupe_quality_number(report, "precision")?;
    let recall = private_dedupe_quality_number(report, "recall")?;
    let f1 = private_dedupe_quality_number(report, "f1")?;
    if !dedupe_quality_score_matches(precision, counts.precision())
        || !dedupe_quality_score_matches(recall, counts.recall())
        || !dedupe_quality_score_matches(f1, counts.f1())
    {
        return Err(BenchmarkGateError::failed(
            "private business dedupe quality metric counts do not match scores",
        ));
    }
    private_dedupe_quality_str(report, "target_claim")?;
    private_dedupe_quality_str(report, "corpus_origin")?;
    private_dedupe_quality_str(report, "privacy_boundary")?;
    private_dedupe_quality_bool(report, "contains_raw_resume_text")?;
    private_dedupe_quality_bool(report, "contains_resume_paths")?;
    private_dedupe_quality_bool(report, "contains_profile_values")?;
    private_dedupe_quality_bool(report, "contains_sample_ids")?;
    private_dedupe_quality_bool(report, "contains_document_ids")?;
    private_dedupe_quality_str(report, "dataset_manifest_sha256")?;
    private_dedupe_quality_str(report, "annotation_manifest_sha256")?;
    private_dedupe_quality_str(report, "dedupe_taxonomy")?;
    private_dedupe_quality_str(report, "scope")?;
    Ok(())
}

fn dedupe_quality_score_matches(reported: f64, expected: f64) -> bool {
    (reported - expected).abs() <= DEDUPE_QUALITY_SCORE_TOLERANCE
}

fn is_allowed_private_business_dedupe_quality_key(key: &str) -> bool {
    matches!(
        key,
        "schema_version"
            | "run_id"
            | "platform"
            | "dataset_kind"
            | "pair_count"
            | "positive_pair_count"
            | "predicted_duplicate_pairs"
            | "true_positive"
            | "false_positive"
            | "false_negative"
            | "true_negative"
            | "precision"
            | "recall"
            | "f1"
            | "target_claim"
            | "corpus_origin"
            | "privacy_boundary"
            | "contains_raw_resume_text"
            | "contains_resume_paths"
            | "contains_profile_values"
            | "contains_sample_ids"
            | "contains_document_ids"
            | "dataset_manifest_sha256"
            | "annotation_manifest_sha256"
            | "dedupe_taxonomy"
            | "scope"
    )
}

fn private_dedupe_quality_str<'a>(
    value: &'a serde_json::Value,
    field: &'static str,
) -> std::result::Result<&'a str, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(private_dedupe_quality_boundary_error)
}

fn private_dedupe_quality_bool(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<bool, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(private_dedupe_quality_boundary_error)
}

fn private_dedupe_quality_usize(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<usize, BenchmarkGateError> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(private_dedupe_quality_boundary_error)?;
    usize::try_from(number).map_err(|_| private_dedupe_quality_boundary_error())
}

fn private_dedupe_quality_number(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<f64, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
        .ok_or_else(private_dedupe_quality_boundary_error)
}

fn private_dedupe_quality_boundary_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private business dedupe quality requires redacted local boundary")
}

pub fn evaluate_vector_quality_gate_json(
    report_json: &str,
    config: VectorQualityGateConfig,
) -> std::result::Result<VectorQualityGateEvaluation, BenchmarkGateError> {
    reject_duplicate_json_object_keys(report_json)?;
    let report: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| BenchmarkGateError::invalid_json())?;

    let schema_version = required_str(&report, "schema_version")?;
    if schema_version != "vector-quality.v1" {
        return Err(BenchmarkGateError::failed(
            "unsupported vector quality schema",
        ));
    }
    let dataset_kind = required_str(&report, "dataset_kind")?;
    match dataset_kind {
        "labeled" | "private-business-labeled" => {}
        _ => {
            return Err(BenchmarkGateError::failed(
                "vector quality requires labeled dataset",
            ));
        }
    }
    if config.require_private_business_labeled && dataset_kind != "private-business-labeled" {
        return Err(BenchmarkGateError::failed(
            "private business vector-quality benchmark required",
        ));
    }
    let sample_count = required_usize(&report, "sample_count")?;
    let recall_at_k = required_f64(&report, "recall_at_k")?;
    let mrr = required_f64(&report, "mrr")?;
    let ndcg_at_k = required_f64(&report, "ndcg_at_k")?;
    let zero_recall_queries = required_usize(&report, "zero_recall_queries")?;
    let target_claim = required_str(&report, "target_claim")?;

    if dataset_kind == "private-business-labeled" {
        validate_private_business_vector_quality_boundary(&report, target_claim)?;
    }
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
    if dataset_kind == "labeled" && target_claim != "not_evaluated" {
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

const PRIVATE_BUSINESS_VECTOR_QUALITY_SCOPE: &str =
    "private business vector-quality benchmark; aggregate redacted report only";
const PRIVATE_BUSINESS_VECTOR_QUALITY_TARGET_CLAIM: &str = "vector_quality_target_met";
const VECTOR_QUALITY_SCORE_TOLERANCE: f64 = 0.000_5;

fn validate_private_business_vector_quality_boundary(
    report: &serde_json::Value,
    target_claim: &str,
) -> std::result::Result<(), BenchmarkGateError> {
    validate_private_business_vector_quality_shape(report)?;
    if private_vector_quality_str(report, "corpus_origin")? != "private_local"
        || private_vector_quality_str(report, "privacy_boundary")? != "redacted_local_aggregate"
        || private_vector_quality_bool(report, "contains_raw_queries")?
        || private_vector_quality_bool(report, "contains_candidate_text")?
        || private_vector_quality_bool(report, "contains_resume_paths")?
        || private_vector_quality_bool(report, "contains_sample_ids")?
        || private_vector_quality_bool(report, "contains_candidate_ids")?
        || private_vector_quality_bool(report, "contains_vectors")?
        || !is_sha256_hex(private_vector_quality_str(
            report,
            "dataset_manifest_sha256",
        )?)
        || !is_sha256_hex(private_vector_quality_str(
            report,
            "annotation_manifest_sha256",
        )?)
        || !is_sha256_hex(private_vector_quality_str(report, "model_manifest_sha256")?)
        || private_vector_quality_str(report, "vector_taxonomy")? != "resume-ir.vector-quality.v1"
        || private_vector_quality_str(report, "scope")? != PRIVATE_BUSINESS_VECTOR_QUALITY_SCOPE
    {
        return Err(private_vector_quality_boundary_error());
    }
    if target_claim != PRIVATE_BUSINESS_VECTOR_QUALITY_TARGET_CLAIM {
        return Err(BenchmarkGateError::failed(
            "private business vector quality requires target claim",
        ));
    }
    if !is_safe_benchmark_token(private_vector_quality_str(report, "run_id")?) {
        return Err(private_vector_quality_boundary_error());
    }
    if !is_safe_platform_label(private_vector_quality_str(report, "platform")?) {
        return Err(private_vector_quality_boundary_error());
    }

    Ok(())
}

fn validate_private_business_vector_quality_shape(
    report: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let Some(object) = report.as_object() else {
        return Err(private_vector_quality_boundary_error());
    };
    for key in object.keys() {
        if !is_allowed_private_business_vector_quality_key(key) {
            return Err(BenchmarkGateError::failed(
                "unsupported private business vector quality field",
            ));
        }
    }
    private_vector_quality_str(report, "schema_version")?;
    private_vector_quality_str(report, "run_id")?;
    private_vector_quality_str(report, "platform")?;
    private_vector_quality_str(report, "dataset_kind")?;
    let sample_count = private_vector_quality_usize(report, "sample_count")?;
    let candidate_count = private_vector_quality_usize(report, "candidate_count")?;
    let top_k = private_vector_quality_usize(report, "top_k")?;
    let recall_at_k = private_vector_quality_number(report, "recall_at_k")?;
    private_vector_quality_number(report, "mrr")?;
    private_vector_quality_number(report, "ndcg_at_k")?;
    let zero_recall_queries = private_vector_quality_usize(report, "zero_recall_queries")?;
    if sample_count == 0
        || candidate_count == 0
        || top_k == 0
        || candidate_count < sample_count
        || top_k > candidate_count
        || zero_recall_queries > sample_count
    {
        return Err(private_vector_quality_counts_error());
    }
    let max_possible_recall = (sample_count - zero_recall_queries) as f64 / sample_count as f64;
    if recall_at_k > max_possible_recall + VECTOR_QUALITY_SCORE_TOLERANCE {
        return Err(private_vector_quality_metric_error());
    }
    private_vector_quality_str(report, "target_claim")?;
    private_vector_quality_str(report, "corpus_origin")?;
    private_vector_quality_str(report, "privacy_boundary")?;
    private_vector_quality_bool(report, "contains_raw_queries")?;
    private_vector_quality_bool(report, "contains_candidate_text")?;
    private_vector_quality_bool(report, "contains_resume_paths")?;
    private_vector_quality_bool(report, "contains_sample_ids")?;
    private_vector_quality_bool(report, "contains_candidate_ids")?;
    private_vector_quality_bool(report, "contains_vectors")?;
    private_vector_quality_str(report, "dataset_manifest_sha256")?;
    private_vector_quality_str(report, "annotation_manifest_sha256")?;
    private_vector_quality_str(report, "model_manifest_sha256")?;
    private_vector_quality_str(report, "vector_taxonomy")?;
    private_vector_quality_str(report, "scope")?;
    Ok(())
}

fn is_allowed_private_business_vector_quality_key(key: &str) -> bool {
    matches!(
        key,
        "schema_version"
            | "run_id"
            | "platform"
            | "dataset_kind"
            | "sample_count"
            | "candidate_count"
            | "top_k"
            | "recall_at_k"
            | "mrr"
            | "ndcg_at_k"
            | "zero_recall_queries"
            | "target_claim"
            | "corpus_origin"
            | "privacy_boundary"
            | "contains_raw_queries"
            | "contains_candidate_text"
            | "contains_resume_paths"
            | "contains_sample_ids"
            | "contains_candidate_ids"
            | "contains_vectors"
            | "dataset_manifest_sha256"
            | "annotation_manifest_sha256"
            | "model_manifest_sha256"
            | "vector_taxonomy"
            | "scope"
    )
}

fn private_vector_quality_str<'a>(
    value: &'a serde_json::Value,
    field: &'static str,
) -> std::result::Result<&'a str, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(private_vector_quality_boundary_error)
}

fn private_vector_quality_bool(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<bool, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(private_vector_quality_boundary_error)
}

fn private_vector_quality_usize(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<usize, BenchmarkGateError> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(private_vector_quality_boundary_error)?;
    usize::try_from(number).map_err(|_| private_vector_quality_boundary_error())
}

fn private_vector_quality_number(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<f64, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|number| number.is_finite() && (0.0..=1.0).contains(number))
        .ok_or_else(private_vector_quality_boundary_error)
}

fn private_vector_quality_boundary_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private business vector quality requires redacted local boundary")
}

fn private_vector_quality_counts_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private business vector quality counts are inconsistent")
}

fn private_vector_quality_metric_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private business vector quality metric counts do not match scores")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FieldQualitySample {
    text: String,
    expected: Vec<FieldQualityMention>,
}

#[derive(Clone, Debug, PartialEq)]
struct DedupeQualitySample {
    left: DedupeProfile,
    right: DedupeProfile,
    duplicate: bool,
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

fn parse_dedupe_quality_sample(line: &str) -> Result<DedupeQualitySample> {
    let value = serde_json::from_str::<serde_json::Value>(line)
        .map_err(|_| BenchmarkError::invalid_config("dedupe_quality_jsonl"))?;
    let left = value
        .get("left")
        .ok_or_else(|| BenchmarkError::invalid_config("dedupe_quality.left"))
        .and_then(parse_dedupe_quality_profile)?;
    let right = value
        .get("right")
        .ok_or_else(|| BenchmarkError::invalid_config("dedupe_quality.right"))
        .and_then(parse_dedupe_quality_profile)?;
    let duplicate = value
        .get("duplicate")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| BenchmarkError::invalid_config("dedupe_quality.duplicate"))?;

    Ok(DedupeQualitySample {
        left,
        right,
        duplicate,
    })
}

fn parse_dedupe_quality_profile(value: &serde_json::Value) -> Result<DedupeProfile> {
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| BenchmarkError::invalid_config("dedupe_quality.profile.id"))?;
    let name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| BenchmarkError::invalid_config("dedupe_quality.profile.name"))?;
    Ok(DedupeProfile::new(id)
        .with_name(name)
        .with_schools(parse_optional_string_array(
            value,
            "schools",
            "dedupe_quality.profile.schools",
        )?)
        .with_companies(parse_optional_string_array(
            value,
            "companies",
            "dedupe_quality.profile.companies",
        )?)
        .with_skills(parse_optional_string_array(
            value,
            "skills",
            "dedupe_quality.profile.skills",
        )?))
}

fn parse_optional_string_array(
    value: &serde_json::Value,
    field: &'static str,
    error_field: &'static str,
) -> Result<Vec<String>> {
    let Some(items) = value.get(field) else {
        return Ok(Vec::new());
    };
    let items = items
        .as_array()
        .ok_or_else(|| BenchmarkError::invalid_config(error_field))?;
    let mut parsed = Vec::with_capacity(items.len());
    for item in items {
        let text = item
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| BenchmarkError::invalid_config(error_field))?;
        parsed.push(text.to_string());
    }
    Ok(parsed)
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
        "wechat" => Some("wechat"),
        "date_range" => Some("date_range"),
        "school" => Some("school"),
        "school_tier" => Some("school_tier"),
        "degree" => Some("degree"),
        "major" => Some("major"),
        "company" => Some("company"),
        "title" => Some("title"),
        "location" => Some("location"),
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
        FieldType::WeChat => "wechat",
        FieldType::DateRange => "date_range",
        FieldType::School => "school",
        FieldType::SchoolTier => "school_tier",
        FieldType::Degree => "degree",
        FieldType::Major => "major",
        FieldType::Company => "company",
        FieldType::Title => "title",
        FieldType::Location => "location",
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
    reject_duplicate_json_object_keys(report_json)?;
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

    match dataset_kind {
        "synthetic" => {
            if !config.allow_synthetic {
                return Err(BenchmarkGateError::failed(
                    "synthetic benchmark requires explicit allowance",
                ));
            }
        }
        "private-real-corpus" => {}
        _ => return Err(BenchmarkGateError::failed("unsupported benchmark dataset")),
    }
    if config.require_private_real_corpus && dataset_kind != "private-real-corpus" {
        return Err(BenchmarkGateError::failed(
            "private real-corpus benchmark required",
        ));
    }
    if dataset_kind == "private-real-corpus" {
        validate_private_real_benchmark_boundary(
            &report,
            target_claim,
            config.allow_smoke_confidence,
        )?;
    }
    if document_count < config.min_documents {
        return Err(BenchmarkGateError::failed(
            "document count below gate minimum",
        ));
    }
    if dataset_kind == "private-real-corpus" {
        validate_private_real_hot_index_document_floor(&report, config.min_documents)?;
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
    if million_scale_verified
        && (dataset_kind != "private-real-corpus" || document_count < 1_000_000)
    {
        return Err(BenchmarkGateError::failed(
            "million-scale claim is not proven",
        ));
    }
    if config.require_million_scale && (!million_scale_verified || document_count < 1_000_000) {
        return Err(BenchmarkGateError::failed(
            "million-scale benchmark required",
        ));
    }
    if config.require_million_scale && required_str(&report, "percentile_confidence")? != "release"
    {
        return Err(BenchmarkGateError::failed(
            "million-scale release benchmark requires release confidence",
        ));
    }
    if config.require_private_real_corpus
        && !config.allow_smoke_confidence
        && (query_count < PRIVATE_REAL_RELEASE_QUERY_SAMPLE_MIN
            || samples < PRIVATE_REAL_RELEASE_QUERY_SAMPLE_MIN)
    {
        return Err(BenchmarkGateError::failed(
            "private real-corpus benchmark requires release query sample count",
        ));
    }
    if target_claim != "not_evaluated" && !config.require_private_real_corpus {
        return Err(BenchmarkGateError::failed("target claim is not proven"));
    }

    Ok(BenchmarkGateEvaluation {
        dataset_kind: dataset_kind.to_string(),
        document_count,
        query_count,
        p95_ms,
    })
}

fn validate_private_real_benchmark_boundary(
    report: &serde_json::Value,
    target_claim: &str,
    allow_smoke_confidence: bool,
) -> std::result::Result<(), BenchmarkGateError> {
    validate_private_real_report_shape(report)?;
    validate_private_real_benchmark_consistency(report)?;
    if private_real_str(report, "corpus_origin")? != "private_local"
        || private_real_str(report, "privacy_boundary")? != "redacted_local_aggregate"
        || private_real_bool(report, "contains_raw_resume_text")?
        || private_real_bool(report, "contains_resume_paths")?
        || private_real_bool(report, "contains_queries")?
        || !is_sha256_hex(private_real_str(report, "dataset_manifest_sha256")?)
        || !is_sha256_hex(private_real_str(report, "query_set_sha256")?)
        || !is_sha256_hex(private_real_str(report, "model_manifest_sha256")?)
        || !is_sha256_hex(private_real_str(report, "corpus_summary_sha256")?)
        || private_real_str(report, "scope")?
            != "private local real-corpus query benchmark; aggregate redacted report only"
    {
        return Err(private_real_boundary_error());
    }
    if !matches!(
        target_claim,
        "benchmark_baseline_observed" | "query_latency_target_met"
    ) {
        return Err(BenchmarkGateError::failed(
            "private real-corpus benchmark requires baseline or query latency target claim",
        ));
    }
    validate_private_real_hot_hybrid_evidence(report)?;
    validate_private_real_hot_index_document_counts(report)?;
    if !is_safe_benchmark_token(private_real_str(report, "run_id")?) {
        return Err(private_real_boundary_error());
    }
    if !is_safe_platform_label(private_real_str(report, "platform")?) {
        return Err(private_real_boundary_error());
    }
    let percentile_confidence = private_real_str(report, "percentile_confidence")?;
    let confidence_allowed = matches!(percentile_confidence, "sampled" | "release")
        || (allow_smoke_confidence && percentile_confidence == "smoke");
    if !confidence_allowed {
        return Err(private_real_boundary_error());
    }

    Ok(())
}

const PRIVATE_REAL_BENCHMARK_SCORE_TOLERANCE: f64 = 0.000_5;

fn validate_private_real_benchmark_consistency(
    report: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let document_count = private_real_usize(report, "document_count")?;
    let query_count = private_real_usize(report, "query_count")?;
    let top_k = private_real_usize(report, "top_k")?;
    let query_total_ms = private_real_number(report, "query_total_ms")?;
    let qps = private_real_number(report, "qps")?;
    let zero_result_queries = private_real_usize(report, "zero_result_queries")?;
    let total_hits = private_real_usize(report, "total_hits")?;
    let latency = report
        .get("query_latency_ms")
        .ok_or_else(private_real_boundary_error)?;
    let samples = private_real_usize(latency, "samples")?;
    let min = private_real_number(latency, "min")?;
    let mean = private_real_number(latency, "mean")?;
    let p50 = private_real_number(latency, "p50")?;
    let p95 = private_real_number(latency, "p95")?;
    let p99 = private_real_number(latency, "p99")?;
    let max = private_real_number(latency, "max")?;
    let max_hits = query_count
        .checked_mul(top_k)
        .ok_or_else(private_real_counts_error)?;

    if document_count == 0
        || query_count == 0
        || top_k == 0
        || samples != query_count
        || zero_result_queries > query_count
        || total_hits > max_hits
        || query_total_ms <= 0.0
        || !latency_summary_is_ordered(min, mean, p50, p95, p99, max)
    {
        return Err(private_real_counts_error());
    }

    let expected_qps = query_count as f64 / (query_total_ms / 1000.0);
    if (qps - expected_qps).abs() > PRIVATE_REAL_BENCHMARK_SCORE_TOLERANCE {
        return Err(private_real_metric_error());
    }

    Ok(())
}

fn latency_summary_is_ordered(min: f64, mean: f64, p50: f64, p95: f64, p99: f64, max: f64) -> bool {
    min <= mean && mean <= max && min <= p50 && p50 <= p95 && p95 <= p99 && p99 <= max
}

fn validate_private_real_report_shape(
    report: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let Some(object) = report.as_object() else {
        return Err(private_real_boundary_error());
    };
    for (key, value) in object {
        if !is_allowed_private_real_report_key(key) {
            return Err(BenchmarkGateError::failed(
                "unsupported private real-corpus benchmark field",
            ));
        }
        if key == "query_latency_ms" {
            validate_private_real_latency_shape(value)?;
        }
    }
    private_real_str(report, "schema_version")?;
    private_real_str(report, "run_id")?;
    private_real_str(report, "platform")?;
    private_real_str(report, "dataset_kind")?;
    private_real_usize(report, "document_count")?;
    private_real_usize(report, "query_count")?;
    private_real_usize(report, "top_k")?;
    private_real_number(report, "build_ms")?;
    private_real_number(report, "query_total_ms")?;
    private_real_number(report, "qps")?;
    private_real_usize(report, "index_size_bytes")?;
    private_real_usize(report, "zero_result_queries")?;
    private_real_usize(report, "total_hits")?;
    private_real_bool(report, "million_scale_verified")?;
    private_real_str(report, "percentile_confidence")?;
    private_real_str(report, "target_claim")?;
    private_real_str(report, "corpus_origin")?;
    private_real_str(report, "privacy_boundary")?;
    private_real_bool(report, "contains_raw_resume_text")?;
    private_real_bool(report, "contains_resume_paths")?;
    private_real_bool(report, "contains_queries")?;
    private_real_str(report, "dataset_manifest_sha256")?;
    private_real_str(report, "query_set_sha256")?;
    private_real_str(report, "model_manifest_sha256").map_err(|_| {
        BenchmarkGateError::failed("private real-corpus benchmark requires model manifest digest")
    })?;
    private_real_str(report, "corpus_summary_sha256")?;
    private_real_str(report, "scope")?;
    validate_private_real_hot_hybrid_evidence(report)?;
    Ok(())
}

fn validate_private_real_hot_hybrid_evidence(
    report: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let error = || {
        BenchmarkGateError::failed(
            "private real-corpus benchmark requires hot-index hybrid query evidence",
        )
    };
    let query_mode = report
        .get("query_mode")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(error)?;
    let query_protocol = report
        .get("query_protocol")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            BenchmarkGateError::failed(
                "private real-corpus benchmark requires query protocol attestation",
            )
        })?;
    let retrieval_layers = report
        .get("retrieval_layers")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(error)?;
    let hot_index = report
        .get("hot_index")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(error)?;
    let hot_path_ocr = report
        .get("hot_path_ocr")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(error)?;
    let hot_path_parsing = report
        .get("hot_path_parsing")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(error)?;
    let hot_path_heavy_model_inference = report
        .get("hot_path_heavy_model_inference")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(error)?;

    if query_protocol != "resume-ir-query-v1" {
        return Err(BenchmarkGateError::failed(
            "private real-corpus benchmark requires query protocol attestation",
        ));
    }

    if query_mode != "hybrid"
        || retrieval_layers != "fulltext+field+vector+rrf"
        || !hot_index
        || hot_path_ocr
        || hot_path_parsing
        || hot_path_heavy_model_inference
    {
        return Err(error());
    }
    Ok(())
}

fn private_real_hot_index_document_coverage(
    report: &serde_json::Value,
) -> std::result::Result<(usize, usize), BenchmarkGateError> {
    let error = private_real_hot_index_document_coverage_error;
    let searchable_document_count = report
        .get("searchable_document_count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|number| usize::try_from(number).ok())
        .ok_or_else(error)?;
    let vector_indexed_document_count = report
        .get("vector_indexed_document_count")
        .and_then(serde_json::Value::as_u64)
        .and_then(|number| usize::try_from(number).ok())
        .ok_or_else(error)?;
    Ok((searchable_document_count, vector_indexed_document_count))
}

fn validate_private_real_hot_index_document_counts(
    report: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let document_count = private_real_usize(report, "document_count")?;
    let (searchable_document_count, vector_indexed_document_count) =
        private_real_hot_index_document_coverage(report)?;
    if searchable_document_count == 0
        || vector_indexed_document_count == 0
        || searchable_document_count > document_count
        || vector_indexed_document_count > document_count
    {
        return Err(private_real_counts_error());
    }
    Ok(())
}

fn validate_private_real_hot_index_document_floor(
    report: &serde_json::Value,
    min_documents: usize,
) -> std::result::Result<(), BenchmarkGateError> {
    let (searchable_document_count, vector_indexed_document_count) =
        private_real_hot_index_document_coverage(report)?;
    if searchable_document_count < min_documents || vector_indexed_document_count < min_documents {
        return Err(private_real_hot_index_document_coverage_error());
    }
    Ok(())
}

fn validate_private_real_latency_shape(
    latency: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let Some(object) = latency.as_object() else {
        return Err(private_real_boundary_error());
    };
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "samples" | "min" | "mean" | "p50" | "p95" | "p99" | "max"
        ) {
            return Err(BenchmarkGateError::failed(
                "unsupported private real-corpus benchmark field",
            ));
        }
    }
    private_real_usize(latency, "samples")?;
    private_real_number(latency, "min")?;
    private_real_number(latency, "mean")?;
    private_real_number(latency, "p50")?;
    private_real_number(latency, "p95")?;
    private_real_number(latency, "p99")?;
    private_real_number(latency, "max")?;
    Ok(())
}

fn reject_duplicate_json_object_keys(
    report_json: &str,
) -> std::result::Result<(), BenchmarkGateError> {
    let mut deserializer = serde_json::Deserializer::from_str(report_json);
    DuplicateKeyDetector
        .deserialize(&mut deserializer)
        .map_err(|error| {
            if error.to_string().contains("duplicate JSON object key") {
                BenchmarkGateError::failed("duplicate JSON object key")
            } else {
                BenchmarkGateError::invalid_json()
            }
        })?;
    deserializer
        .end()
        .map_err(|_| BenchmarkGateError::invalid_json())?;
    Ok(())
}

struct DuplicateKeyDetector;

impl<'de> DeserializeSeed<'de> for DuplicateKeyDetector {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> std::result::Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(DuplicateKeyVisitor)
    }
}

struct DuplicateKeyVisitor;

impl<'de> Visitor<'de> for DuplicateKeyVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, _value: bool) -> std::result::Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> std::result::Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> std::result::Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> std::result::Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_str<E>(self, _value: &str) -> std::result::Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_string<E>(self, _value: String) -> std::result::Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_unit<E>(self) -> std::result::Result<(), E>
    where
        E: de::Error,
    {
        Ok(())
    }

    fn visit_seq<A>(self, mut access: A) -> std::result::Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        while let Some(()) = access.next_element_seed(DuplicateKeyDetector)? {}
        Ok(())
    }

    fn visit_map<A>(self, mut access: A) -> std::result::Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::<String>::new();
        while let Some(key) = access.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            access.next_value_seed(DuplicateKeyDetector)?;
        }
        Ok(())
    }
}

pub fn evaluate_ocr_throughput_gate_json(
    report_json: &str,
    config: OcrThroughputGateConfig,
) -> std::result::Result<OcrThroughputGateEvaluation, BenchmarkGateError> {
    reject_duplicate_json_object_keys(report_json)?;
    let report: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| BenchmarkGateError::invalid_json())?;

    let schema_version = required_str(&report, "schema_version")?;
    if schema_version != "ocr-throughput.v1" {
        return Err(BenchmarkGateError::failed(
            "unsupported OCR throughput schema",
        ));
    }

    let dataset_kind = required_str(&report, "dataset_kind")?;
    match dataset_kind {
        "synthetic" | "private-real-corpus" => {}
        _ => return Err(BenchmarkGateError::failed("unsupported OCR dataset")),
    }
    if config.require_private_real_corpus && dataset_kind != "private-real-corpus" {
        return Err(BenchmarkGateError::failed(
            "private real-corpus OCR benchmark required",
        ));
    }
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
    if dataset_kind == "private-real-corpus" {
        validate_private_real_ocr_throughput_boundary(&report)?;
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
    if dataset_kind == "private-real-corpus"
        && target_claim != PRIVATE_REAL_OCR_THROUGHPUT_TARGET_CLAIM
    {
        return Err(BenchmarkGateError::failed(
            "private real-corpus OCR benchmark requires throughput target claim",
        ));
    }
    if dataset_kind == "synthetic" && target_claim != "not_evaluated" {
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

const PRIVATE_REAL_OCR_THROUGHPUT_SCOPE: &str =
    "private real-corpus OCR throughput benchmark; aggregate redacted report only";
const PRIVATE_REAL_OCR_THROUGHPUT_TARGET_CLAIM: &str = "ocr_throughput_target_met";
const PRIVATE_REAL_OCR_THROUGHPUT_RELEASE_MIN_PAGES: usize = 500;
const PRIVATE_REAL_OCR_THROUGHPUT_RELEASE_MAX_P95_MS: f64 = 1_000.0;
const PRIVATE_REAL_OCR_THROUGHPUT_RELEASE_MIN_PAGES_PER_SECOND: f64 = 1.0;
const OCR_THROUGHPUT_SCORE_TOLERANCE: f64 = 0.000_5;

fn validate_private_real_ocr_throughput_boundary(
    report: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    validate_private_real_ocr_throughput_shape(report)?;
    let page_count = private_ocr_usize(report, "page_count")?;
    let document_count = private_ocr_usize(report, "document_count")?;
    let scanned_document_count = private_ocr_usize(report, "scanned_document_count")?;
    let failed_document_count = private_ocr_usize(report, "failed_document_count")?;
    let render_failure_count = private_ocr_usize(report, "render_failure_count")?;
    let ocr_failure_count = private_ocr_usize(report, "ocr_failure_count")?;
    let run_budget_exhausted = private_ocr_bool(report, "run_budget_exhausted")?;
    let total_ms = private_ocr_number(report, "total_ms")?;
    let pages_per_second = private_ocr_number(report, "pages_per_second")?;
    let latency = report
        .get("page_latency_ms")
        .ok_or_else(private_ocr_boundary_error)?;
    let samples = private_ocr_usize(latency, "samples")?;
    if page_count == 0
        || document_count == 0
        || scanned_document_count == 0
        || scanned_document_count > document_count
        || failed_document_count > document_count
        || render_failure_count + ocr_failure_count != failed_document_count
        || scanned_document_count > page_count
        || samples != page_count
        || total_ms <= 0.0
    {
        return Err(private_ocr_counts_error());
    }
    if run_budget_exhausted {
        return Err(BenchmarkGateError::failed(
            "private real-corpus OCR benchmark run budget exhausted",
        ));
    }
    let expected_pages_per_second = page_count as f64 / (total_ms / 1000.0);
    if (pages_per_second - expected_pages_per_second).abs() > OCR_THROUGHPUT_SCORE_TOLERANCE {
        return Err(private_ocr_metric_error());
    }
    if private_ocr_str(report, "corpus_origin")? != "private_local"
        || private_ocr_str(report, "privacy_boundary")? != "redacted_local_aggregate"
        || private_ocr_bool(report, "contains_raw_ocr_text")?
        || private_ocr_bool(report, "contains_page_images")?
        || private_ocr_bool(report, "contains_resume_paths")?
        || private_ocr_bool(report, "contains_document_ids")?
        || private_ocr_bool(report, "contains_page_ids")?
        || private_ocr_bool(report, "contains_command_paths")?
        || !is_sha256_hex(private_ocr_str(report, "dataset_manifest_sha256")?)
        || !is_sha256_hex(private_ocr_str(report, "ocr_runtime_manifest_sha256")?)
        || !is_sha256_hex(private_ocr_str(report, "renderer_manifest_sha256")?)
        || !is_sha256_hex(private_ocr_str(report, "language_pack_manifest_sha256")?)
        || private_ocr_str(report, "scope")? != PRIVATE_REAL_OCR_THROUGHPUT_SCOPE
    {
        return Err(private_ocr_boundary_error());
    }
    if !is_safe_benchmark_token(private_ocr_str(report, "run_id")?)
        || !is_safe_platform_label(private_ocr_str(report, "platform")?)
        || !is_safe_benchmark_token(private_ocr_str(report, "engine_kind")?)
    {
        return Err(private_ocr_boundary_error());
    }

    Ok(())
}

fn validate_private_real_ocr_throughput_shape(
    report: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let Some(object) = report.as_object() else {
        return Err(private_ocr_boundary_error());
    };
    for key in object.keys() {
        if !is_allowed_private_real_ocr_key(key) {
            return Err(BenchmarkGateError::failed(
                "unsupported private real-corpus OCR benchmark field",
            ));
        }
    }
    private_ocr_str(report, "schema_version")?;
    private_ocr_str(report, "run_id")?;
    private_ocr_str(report, "platform")?;
    private_ocr_str(report, "dataset_kind")?;
    private_ocr_usize(report, "page_count")?;
    private_ocr_usize(report, "document_count")?;
    private_ocr_usize(report, "scanned_document_count")?;
    private_ocr_usize(report, "failed_document_count")?;
    private_ocr_usize(report, "render_failure_count")?;
    private_ocr_usize(report, "ocr_failure_count")?;
    private_ocr_bool(report, "run_budget_exhausted")?;
    private_ocr_str(report, "engine_kind")?;
    private_ocr_number(report, "total_ms")?;
    let latency = report
        .get("page_latency_ms")
        .ok_or_else(private_ocr_boundary_error)?;
    validate_private_real_ocr_latency_shape(latency)?;
    private_ocr_number(report, "pages_per_second")?;
    private_ocr_str(report, "target_claim")?;
    private_ocr_str(report, "corpus_origin")?;
    private_ocr_str(report, "privacy_boundary")?;
    private_ocr_bool(report, "contains_raw_ocr_text")?;
    private_ocr_bool(report, "contains_page_images")?;
    private_ocr_bool(report, "contains_resume_paths")?;
    private_ocr_bool(report, "contains_document_ids")?;
    private_ocr_bool(report, "contains_page_ids")?;
    private_ocr_bool(report, "contains_command_paths")?;
    private_ocr_str(report, "dataset_manifest_sha256")?;
    private_ocr_str(report, "ocr_runtime_manifest_sha256")?;
    private_ocr_str(report, "renderer_manifest_sha256")?;
    private_ocr_str(report, "language_pack_manifest_sha256")?;
    private_ocr_str(report, "scope")?;
    Ok(())
}

fn validate_private_real_ocr_latency_shape(
    latency: &serde_json::Value,
) -> std::result::Result<(), BenchmarkGateError> {
    let Some(object) = latency.as_object() else {
        return Err(private_ocr_boundary_error());
    };
    for key in object.keys() {
        if !matches!(key.as_str(), "samples" | "p50" | "p95" | "p99") {
            return Err(BenchmarkGateError::failed(
                "unsupported private real-corpus OCR latency field",
            ));
        }
    }
    private_ocr_usize(latency, "samples")?;
    let p50 = private_ocr_number(latency, "p50")?;
    let p95 = private_ocr_number(latency, "p95")?;
    let p99 = private_ocr_number(latency, "p99")?;
    if p50 > p95 || p95 > p99 {
        return Err(private_ocr_boundary_error());
    }
    Ok(())
}

fn is_allowed_private_real_ocr_key(key: &str) -> bool {
    matches!(
        key,
        "schema_version"
            | "run_id"
            | "platform"
            | "dataset_kind"
            | "page_count"
            | "document_count"
            | "scanned_document_count"
            | "failed_document_count"
            | "render_failure_count"
            | "ocr_failure_count"
            | "run_budget_exhausted"
            | "engine_kind"
            | "total_ms"
            | "page_latency_ms"
            | "pages_per_second"
            | "target_claim"
            | "corpus_origin"
            | "privacy_boundary"
            | "contains_raw_ocr_text"
            | "contains_page_images"
            | "contains_resume_paths"
            | "contains_document_ids"
            | "contains_page_ids"
            | "contains_command_paths"
            | "dataset_manifest_sha256"
            | "ocr_runtime_manifest_sha256"
            | "renderer_manifest_sha256"
            | "language_pack_manifest_sha256"
            | "scope"
    )
}

fn private_ocr_str<'a>(
    value: &'a serde_json::Value,
    field: &'static str,
) -> std::result::Result<&'a str, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(private_ocr_boundary_error)
}

fn private_ocr_bool(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<bool, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(private_ocr_boundary_error)
}

fn private_ocr_usize(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<usize, BenchmarkGateError> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(private_ocr_boundary_error)?;
    usize::try_from(number).map_err(|_| private_ocr_boundary_error())
}

fn private_ocr_number(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<f64, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .ok_or_else(private_ocr_boundary_error)
}

fn private_ocr_boundary_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private real-corpus OCR benchmark requires redacted local boundary")
}

fn private_ocr_counts_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private real-corpus OCR throughput counts are inconsistent")
}

fn private_ocr_metric_error() -> BenchmarkGateError {
    BenchmarkGateError::failed(
        "private real-corpus OCR throughput metric counts do not match scores",
    )
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

fn private_real_str<'a>(
    value: &'a serde_json::Value,
    field: &'static str,
) -> std::result::Result<&'a str, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(private_real_boundary_error)
}

fn private_real_bool(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<bool, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(private_real_boundary_error)
}

fn private_real_usize(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<usize, BenchmarkGateError> {
    let number = value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(private_real_boundary_error)?;
    usize::try_from(number).map_err(|_| private_real_boundary_error())
}

fn private_real_number(
    value: &serde_json::Value,
    field: &'static str,
) -> std::result::Result<f64, BenchmarkGateError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .ok_or_else(private_real_boundary_error)
}

fn private_real_boundary_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private real-corpus benchmark requires redacted local boundary")
}

fn private_real_counts_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private real-corpus benchmark counts are inconsistent")
}

fn private_real_metric_error() -> BenchmarkGateError {
    BenchmarkGateError::failed("private real-corpus benchmark metric counts do not match scores")
}

fn private_real_hot_index_document_coverage_error() -> BenchmarkGateError {
    BenchmarkGateError::failed(
        "private real-corpus benchmark requires hot-index document coverage evidence",
    )
}

fn validate_private_query_corpus_summary_shape(report: &serde_json::Value) -> Result<()> {
    let Some(object) = report.as_object() else {
        return Err(BenchmarkError::invalid_config(
            "private_query_corpus_summary_boundary",
        ));
    };
    for key in object.keys() {
        if !is_allowed_private_query_corpus_summary_key(key) {
            return Err(BenchmarkError::invalid_config(
                "private_query_corpus_summary_boundary",
            ));
        }
    }

    private_query_corpus_summary_str(report, "schema_version")?;
    private_query_corpus_summary_str(report, "privacy_boundary")?;
    private_query_corpus_summary_usize(report, "document_count")?;
    private_query_corpus_summary_usize(report, "searchable_document_count")?;
    private_query_corpus_summary_usize(report, "vector_indexed_document_count")?;
    private_query_corpus_summary_usize(report, "active_vector_document_count")?;
    private_query_corpus_summary_usize(report, "vector_count")?;
    private_query_corpus_summary_usize(report, "vector_deleted_count")?;
    private_query_corpus_summary_str(report, "vector_index_state")?;
    private_query_corpus_summary_str(report, "vector_search_backend")?;
    private_query_corpus_summary_bool(report, "hot_index_fully_covered")?;
    private_query_corpus_summary_bool(report, "contains_raw_resume_text")?;
    private_query_corpus_summary_bool(report, "contains_resume_paths")?;
    private_query_corpus_summary_bool(report, "contains_queries")?;
    private_query_corpus_summary_bool(report, "contains_sample_ids")?;
    Ok(())
}

fn private_query_corpus_summary_str<'a>(
    report: &'a serde_json::Value,
    key: &'static str,
) -> Result<&'a str> {
    report
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| BenchmarkError::invalid_config("private_query_corpus_summary_boundary"))
}

fn private_query_corpus_summary_bool(
    report: &serde_json::Value,
    key: &'static str,
) -> Result<bool> {
    report
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| BenchmarkError::invalid_config("private_query_corpus_summary_boundary"))
}

fn private_query_corpus_summary_usize(
    report: &serde_json::Value,
    key: &'static str,
) -> Result<usize> {
    report
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| BenchmarkError::invalid_config("private_query_corpus_summary_boundary"))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn is_allowed_private_query_corpus_summary_key(key: &str) -> bool {
    matches!(
        key,
        "schema_version"
            | "privacy_boundary"
            | "document_count"
            | "searchable_document_count"
            | "vector_indexed_document_count"
            | "active_vector_document_count"
            | "vector_count"
            | "vector_deleted_count"
            | "vector_index_state"
            | "vector_search_backend"
            | "hot_index_fully_covered"
            | "document_status_counts"
            | "ingest_job_status_counts"
            | "ingest_job_kind_status_counts"
            | "ingest_job_failure_counts"
            | "contains_raw_resume_text"
            | "contains_resume_paths"
            | "contains_queries"
            | "contains_sample_ids"
    )
}

fn is_allowed_private_real_report_key(key: &str) -> bool {
    matches!(
        key,
        "schema_version"
            | "run_id"
            | "platform"
            | "dataset_kind"
            | "document_count"
            | "searchable_document_count"
            | "vector_indexed_document_count"
            | "query_count"
            | "top_k"
            | "build_ms"
            | "query_total_ms"
            | "qps"
            | "index_size_bytes"
            | "query_latency_ms"
            | "zero_result_queries"
            | "total_hits"
            | "million_scale_verified"
            | "percentile_confidence"
            | "target_claim"
            | "query_protocol"
            | "query_mode"
            | "retrieval_layers"
            | "hot_index"
            | "hot_path_ocr"
            | "hot_path_parsing"
            | "hot_path_heavy_model_inference"
            | "corpus_origin"
            | "privacy_boundary"
            | "contains_raw_resume_text"
            | "contains_resume_paths"
            | "contains_queries"
            | "dataset_manifest_sha256"
            | "query_set_sha256"
            | "model_manifest_sha256"
            | "corpus_summary_sha256"
            | "scope"
    )
}

fn is_safe_benchmark_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn is_safe_platform_label(value: &str) -> bool {
    let mut parts = value.split('/');
    let Some(os) = parts.next() else {
        return false;
    };
    let Some(arch) = parts.next() else {
        return false;
    };
    parts.next().is_none() && is_safe_benchmark_token(os) && is_safe_benchmark_token(arch)
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

enum PrivatePdfRendererClient {
    LocalCommand(LocalPdfRenderCommandClient),
    Pdftoppm(PdftoppmPdfRenderer),
}

impl PrivatePdfRendererClient {
    fn new(engine: &PrivatePdfRenderEngine) -> Result<Self> {
        match engine {
            PrivatePdfRenderEngine::LocalCommand { command } => {
                let spec = LocalPdfRenderCommandSpec::new(command, Vec::<String>::new())
                    .map_err(BenchmarkError::ocr)?;
                Ok(Self::LocalCommand(LocalPdfRenderCommandClient::new(spec)))
            }
            PrivatePdfRenderEngine::Pdftoppm { command } => {
                let spec = PdftoppmRenderSpec::new(command).map_err(BenchmarkError::ocr)?;
                Ok(Self::Pdftoppm(PdftoppmPdfRenderer::new(spec)))
            }
        }
    }

    fn render_page(
        &self,
        document_bytes: &[u8],
        page_no: u32,
        render_dpi: u32,
        budget: OcrWorkerBudget,
        cancellation: &CancellationToken,
    ) -> std::result::Result<RenderedPage, ocr_client::OcrError> {
        match self {
            Self::LocalCommand(renderer) => {
                renderer.render_page(document_bytes, page_no, render_dpi, budget, cancellation)
            }
            Self::Pdftoppm(renderer) => {
                renderer.render_page(document_bytes, page_no, render_dpi, budget, cancellation)
            }
        }
    }
}

fn private_pdf_renderer_kind(engine: &PrivatePdfRenderEngine) -> &'static str {
    match engine {
        PrivatePdfRenderEngine::LocalCommand { .. } => "local-command",
        PrivatePdfRenderEngine::Pdftoppm { .. } => "pdftoppm",
    }
}

fn collect_private_pdf_documents(root: &Path, max_documents: usize) -> Result<Vec<PathBuf>> {
    if max_documents == 0 {
        return Err(BenchmarkError::invalid_config("private_ocr_max_documents"));
    }
    let metadata = fs::symlink_metadata(root).map_err(BenchmarkError::io)?;
    if !metadata.is_dir() {
        return Err(BenchmarkError::invalid_config("private_ocr_root"));
    }

    let mut selected = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let mut entries = fs::read_dir(&directory)
            .map_err(BenchmarkError::io)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(BenchmarkError::io)?;
        entries.sort_by_key(|entry| entry.path());

        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(BenchmarkError::io)?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if private_ocr_is_pdf_path(&path) {
                selected.push(path);
                if selected.len() >= max_documents {
                    return Ok(selected);
                }
            }
        }
    }

    Ok(selected)
}

fn private_ocr_is_pdf_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn private_ocr_run_budget_exhausted(started: Instant, max_run_ms: Option<u64>) -> bool {
    max_run_ms.is_some_and(|max_run_ms| elapsed_ms(started) >= max_run_ms as f64)
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

fn private_query_percentile_confidence(document_count: usize, query_count: usize) -> &'static str {
    if document_count >= 1_000_000 && query_count >= PRIVATE_REAL_RELEASE_QUERY_SAMPLE_MIN {
        "release"
    } else {
        percentile_confidence(query_count)
    }
}

fn load_private_query_set(path: &Path, max_queries: usize) -> Result<Vec<String>> {
    let content = fs::read_to_string(path).map_err(BenchmarkError::io)?;
    let mut queries = Vec::new();
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if queries.len() >= max_queries {
            break;
        }
        let value = serde_json::from_str::<serde_json::Value>(line)
            .map_err(|_| BenchmarkError::invalid_config("private_query_jsonl"))?;
        let query = value
            .get("query")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .ok_or_else(|| BenchmarkError::invalid_config("private_query.query"))?;
        queries.push(query.to_string());
    }
    if queries.is_empty() {
        return Err(BenchmarkError::invalid_config("private_query_count"));
    }
    Ok(queries)
}

fn create_private_query_scratch_dir() -> Result<PathBuf> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let path = std::env::temp_dir().join(format!(
        "resume-ir-private-query-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&path).map_err(BenchmarkError::io)?;
    set_owner_only_dir(&path)?;
    Ok(path)
}

fn write_private_query_file(path: &Path, query: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(BenchmarkError::io)?;
    set_owner_only_file(path)?;
    file.write_all(query.as_bytes())
        .map_err(BenchmarkError::io)?;
    file.write_all(b"\n").map_err(BenchmarkError::io)?;
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .map_err(BenchmarkError::io)?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions).map_err(BenchmarkError::io)
}

#[cfg(not(unix))]
fn set_owner_only_dir(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_owner_only_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .map_err(BenchmarkError::io)?
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions).map_err(BenchmarkError::io)
}

#[cfg(not(unix))]
fn set_owner_only_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn run_private_query_command(
    command: &PrivateQueryBenchmarkCommand,
    query_file: &Path,
    top_k: usize,
    timeout_ms: u64,
) -> Result<usize> {
    let started = Instant::now();
    let mut child = Command::new(&command.command)
        .args(&command.args)
        .env("RESUME_IR_QUERY_INPUT_PATH", query_file)
        .env("RESUME_IR_QUERY_TOP_K", top_k.to_string())
        .env("RESUME_IR_QUERY_MODE", "hybrid")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(BenchmarkError::io)?;
    let timeout = Duration::from_millis(timeout_ms);
    loop {
        if child.try_wait().map_err(BenchmarkError::io)?.is_some() {
            let output = child.wait_with_output().map_err(BenchmarkError::io)?;
            if !output.status.success() {
                return Err(BenchmarkError::invalid_config(
                    "private_query_command_status",
                ));
            }
            return parse_private_query_command_stdout(&output.stdout, top_k);
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(BenchmarkError::invalid_config(
                "private_query_command_timeout",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn parse_private_query_command_stdout(stdout: &[u8], top_k: usize) -> Result<usize> {
    let stdout = std::str::from_utf8(stdout)
        .map_err(|_| BenchmarkError::invalid_config("private_query_command_stdout"))?;
    let mut saw_header = false;
    let mut saw_hybrid_mode = false;
    let mut saw_hybrid_layers = false;
    let mut attested_top_k = None;
    let mut hits = None;
    for line in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if !saw_header {
            if line != "resume-ir-query-v1" {
                return Err(BenchmarkError::invalid_config(
                    "private_query_command_stdout",
                ));
            }
            saw_header = true;
            continue;
        }
        if let Some(value) = line.strip_prefix("mode=") {
            if saw_hybrid_mode || value != "hybrid" {
                return Err(BenchmarkError::invalid_config(
                    "private_query_protocol_attestation",
                ));
            }
            saw_hybrid_mode = true;
            continue;
        }
        if let Some(value) = line.strip_prefix("layers=") {
            if saw_hybrid_layers || value != "fulltext+field+vector+rrf" {
                return Err(BenchmarkError::invalid_config(
                    "private_query_protocol_attestation",
                ));
            }
            saw_hybrid_layers = true;
            continue;
        }
        if let Some(value) = line.strip_prefix("top_k=") {
            if attested_top_k.is_some() {
                return Err(BenchmarkError::invalid_config(
                    "private_query_top_k_attestation",
                ));
            }
            let parsed = value
                .parse::<usize>()
                .map_err(|_| BenchmarkError::invalid_config("private_query_top_k_attestation"))?;
            if parsed != top_k {
                return Err(BenchmarkError::invalid_config(
                    "private_query_top_k_attestation",
                ));
            }
            attested_top_k = Some(parsed);
            continue;
        }
        if let Some(value) = line.strip_prefix("hits=") {
            if hits.is_some() {
                return Err(BenchmarkError::invalid_config(
                    "private_query_command_stdout",
                ));
            }
            let parsed = value
                .parse::<usize>()
                .map_err(|_| BenchmarkError::invalid_config("private_query_hits"))?;
            if parsed > top_k {
                return Err(BenchmarkError::invalid_config("private_query_hits"));
            }
            hits = Some(parsed);
            continue;
        }
        return Err(BenchmarkError::invalid_config(
            "private_query_command_stdout",
        ));
    }
    if !saw_header {
        return Err(BenchmarkError::invalid_config(
            "private_query_command_stdout",
        ));
    }
    if !saw_hybrid_mode || !saw_hybrid_layers {
        return Err(BenchmarkError::invalid_config(
            "private_query_protocol_attestation",
        ));
    }
    if attested_top_k != Some(top_k) {
        return Err(BenchmarkError::invalid_config(
            "private_query_top_k_attestation",
        ));
    }
    hits.ok_or_else(|| BenchmarkError::invalid_config("private_query_hits"))
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

fn format_consistency_number(value: f64) -> String {
    format!("{value:.6}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_ocr_report_does_not_claim_target_when_release_latency_is_missed() {
        let report = PrivateOcrThroughputReport {
            run_id: "ocr_release_latency_probe".to_string(),
            platform: "macos/aarch64".to_string(),
            page_count: 500,
            document_count: 500,
            scanned_document_count: 500,
            failed_document_count: 0,
            render_failure_count: 0,
            ocr_failure_count: 0,
            run_budget_exhausted: false,
            engine_kind: "tesseract",
            total_ms: 1_258_375.597_167,
            latency: LatencySummary {
                samples: 500,
                min_ms: 1_200.0,
                mean_ms: 2_516.0,
                p50_ms: 2_409.907,
                p95_ms: 4_214.12,
                p99_ms: 5_109.623,
                max_ms: 5_500.0,
            },
            manifests: PrivateOcrManifestDigests::new(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                "1111111111111111111111111111111111111111111111111111111111111111",
                "2222222222222222222222222222222222222222222222222222222222222222",
            )
            .unwrap(),
        };

        let json = report.to_redacted_json();

        assert!(json.contains("\"target_claim\":\"not_evaluated\""));
        assert!(!json.contains("\"target_claim\":\"ocr_throughput_target_met\""));
    }

    #[test]
    fn private_ocr_report_claims_target_only_when_release_thresholds_are_met() {
        let report = PrivateOcrThroughputReport {
            run_id: "ocr_release_passing_probe".to_string(),
            platform: "macos/aarch64".to_string(),
            page_count: 500,
            document_count: 500,
            scanned_document_count: 500,
            failed_document_count: 0,
            render_failure_count: 0,
            ocr_failure_count: 0,
            run_budget_exhausted: false,
            engine_kind: "tesseract",
            total_ms: 200_000.0,
            latency: LatencySummary {
                samples: 500,
                min_ms: 120.0,
                mean_ms: 400.0,
                p50_ms: 250.0,
                p95_ms: 450.0,
                p99_ms: 800.0,
                max_ms: 900.0,
            },
            manifests: PrivateOcrManifestDigests::new(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
                "1111111111111111111111111111111111111111111111111111111111111111",
                "2222222222222222222222222222222222222222222222222222222222222222",
            )
            .unwrap(),
        };

        let json = report.to_redacted_json();

        assert!(json.contains("\"target_claim\":\"ocr_throughput_target_met\""));
    }
}
