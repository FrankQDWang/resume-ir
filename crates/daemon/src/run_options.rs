use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use embedder::ResidentEmbeddingClient;
use import_pipeline::{LinearPromotionPolicy, PdfImportPolicy, SearchPublicationVectorization};

use crate::daemon_error::{DaemonError, Result};
use crate::parent_lifecycle::ParentLifecycleMode;
use crate::search_runtime_config::SearchRuntimeConfig;

mod validation;

const DEFAULT_OCR_ENGINE_PROFILE: &str = "local-command";
const DEFAULT_OCR_LANG: &str = "eng";
const DEFAULT_OCR_PROFILE: &str = "balanced";
const DEFAULT_OCR_RENDER_DPI: u32 = 300;
const DEFAULT_OCR_PAGE_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_OCR_MAX_PAGES_PER_DOCUMENT: u32 = 100;
const DEFAULT_EMBEDDING_TIMEOUT_MS: u64 = 30_000;

#[cfg(feature = "resident-embedding-pool-experiment")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResidentEmbeddingPoolArm {
    I3Bulk1x4B4,
    I3Bulk2x2B4,
    I3Bulk2x3B4,
}

#[cfg(feature = "resident-embedding-pool-experiment")]
impl ResidentEmbeddingPoolArm {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "i3_bulk1x4_b4" => Some(Self::I3Bulk1x4B4),
            "i3_bulk2x2_b4" => Some(Self::I3Bulk2x2B4),
            "i3_bulk2x3_b4" => Some(Self::I3Bulk2x3B4),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct RunOptions {
    pub(crate) foreground: bool,
    pub(crate) parent_lifecycle: ParentLifecycleMode,
    pub(crate) launch_id: Option<String>,
    pub(crate) once: bool,
    pub(crate) ipc_listen: Option<SocketAddr>,
    pub(crate) expected_ipc_protocol: Option<String>,
    pub(crate) max_requests: Option<usize>,
    pub(crate) work_imports_once: bool,
    pub(crate) work_imports: bool,
    pub(crate) rescan_completed_imports: bool,
    pub(crate) watch_import_roots: bool,
    pub(crate) import_rescan_min_age_seconds: Option<i64>,
    pub(crate) stale_import_task_seconds: Option<i64>,
    pub(crate) import_retry_backoff_seconds: Option<i64>,
    pub(crate) classifier_model_configured: bool,
    pub(crate) classifier_model_path: Option<PathBuf>,
    pub(crate) linear_promotion: LinearPromotionPolicy,
    pub(crate) pdf_import: PdfImportPolicy,
    pub(crate) work_ocr_once: bool,
    pub(crate) work_ocr: bool,
    pub(crate) work_index_once: bool,
    pub(crate) work_index: bool,
    pub(crate) ocr_command: Option<PathBuf>,
    pub(crate) ocr_tesseract_command: Option<PathBuf>,
    pub(crate) pdf_render_command: Option<PathBuf>,
    pub(crate) ocr_engine_profile: String,
    pub(crate) ocr_lang: String,
    pub(crate) ocr_profile: String,
    pub(crate) ocr_render_dpi: u32,
    pub(crate) ocr_page_timeout_ms: u64,
    pub(crate) ocr_max_pages_per_document: u32,
    pub(crate) ocr_jobs_per_tick: Option<usize>,
    pub(crate) embedding_command: Option<PathBuf>,
    pub(crate) embedding_model_id: Option<String>,
    pub(crate) embedding_dimension: Option<usize>,
    pub(crate) embedding_timeout_ms: u64,
    pub(crate) resident_embedding: Option<ResidentEmbeddingClient>,
    pub(crate) publication_resident_embeddings: Vec<ResidentEmbeddingClient>,
    #[cfg(feature = "resident-embedding-pool-experiment")]
    pub(crate) resident_embedding_pool_arm: Option<ResidentEmbeddingPoolArm>,
    pub(crate) search_vectorization: SearchPublicationVectorization,
    pub(crate) worker_interval_ms: Option<u64>,
    pub(crate) max_worker_ticks: Option<usize>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            foreground: false,
            parent_lifecycle: ParentLifecycleMode::Unmanaged,
            launch_id: None,
            once: false,
            ipc_listen: None,
            expected_ipc_protocol: None,
            max_requests: None,
            work_imports_once: false,
            work_imports: false,
            rescan_completed_imports: false,
            watch_import_roots: false,
            import_rescan_min_age_seconds: None,
            stale_import_task_seconds: None,
            import_retry_backoff_seconds: None,
            classifier_model_configured: false,
            classifier_model_path: None,
            linear_promotion: LinearPromotionPolicy::default(),
            pdf_import: PdfImportPolicy::Enabled,
            work_ocr_once: false,
            work_ocr: false,
            work_index_once: false,
            work_index: false,
            ocr_command: None,
            ocr_tesseract_command: None,
            pdf_render_command: None,
            ocr_engine_profile: DEFAULT_OCR_ENGINE_PROFILE.to_string(),
            ocr_lang: DEFAULT_OCR_LANG.to_string(),
            ocr_profile: DEFAULT_OCR_PROFILE.to_string(),
            ocr_render_dpi: DEFAULT_OCR_RENDER_DPI,
            ocr_page_timeout_ms: DEFAULT_OCR_PAGE_TIMEOUT_MS,
            ocr_max_pages_per_document: DEFAULT_OCR_MAX_PAGES_PER_DOCUMENT,
            ocr_jobs_per_tick: None,
            embedding_command: None,
            embedding_model_id: None,
            embedding_dimension: None,
            embedding_timeout_ms: DEFAULT_EMBEDDING_TIMEOUT_MS,
            resident_embedding: None,
            publication_resident_embeddings: Vec::new(),
            #[cfg(feature = "resident-embedding-pool-experiment")]
            resident_embedding_pool_arm: None,
            search_vectorization: SearchPublicationVectorization::default(),
            worker_interval_ms: None,
            max_worker_ticks: None,
        }
    }
}

impl RunOptions {
    pub(crate) fn has_worker_loop(&self) -> bool {
        self.work_imports || self.work_ocr || self.work_index
    }

    pub(crate) fn search_runtime_config(&self) -> SearchRuntimeConfig {
        SearchRuntimeConfig::new(
            self.resident_embedding.clone(),
            self.embedding_model_id.clone(),
            self.embedding_dimension,
            self.embedding_timeout_ms,
        )
    }
}

pub(crate) fn parse(args: &[String]) -> Result<RunOptions> {
    let mut options = RunOptions::default();
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--foreground" => {
                options.foreground = true;
                index += 1;
            }
            "--parent-lifecycle-stdin" => {
                if options.parent_lifecycle != ParentLifecycleMode::Unmanaged {
                    return Err(DaemonError::usage(usage()));
                }
                options.parent_lifecycle = ParentLifecycleMode::Stdin;
                index += 1;
            }
            "--launch-id" => {
                if options.launch_id.is_some() {
                    return Err(DaemonError::usage(usage()));
                }
                let launch_id = non_empty(args.get(index + 1))?;
                if !valid_launch_id(&launch_id) {
                    return Err(DaemonError::usage(usage()));
                }
                options.launch_id = Some(launch_id);
                index += 2;
            }
            "--once" => {
                options.once = true;
                index += 1;
            }
            "--ipc-listen" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(usage()));
                };
                options.ipc_listen = Some(loopback_addr(value)?);
                index += 2;
            }
            "--expected-ipc-protocol" => {
                if options.expected_ipc_protocol.is_some() {
                    return Err(DaemonError::usage(usage()));
                }
                options.expected_ipc_protocol = Some(non_empty(args.get(index + 1))?);
                index += 2;
            }
            "--max-requests" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(usage()));
                };
                options.max_requests = Some(positive_usize(value)?);
                index += 2;
            }
            "--work-imports-once" => {
                options.work_imports_once = true;
                index += 1;
            }
            "--work-imports" => {
                options.work_imports = true;
                index += 1;
            }
            "--rescan-completed-imports" => {
                options.rescan_completed_imports = true;
                index += 1;
            }
            "--watch-import-roots" => {
                options.watch_import_roots = true;
                index += 1;
            }
            "--import-rescan-min-age-seconds" => {
                options.import_rescan_min_age_seconds = Some(nonnegative_i64(args.get(index + 1))?);
                index += 2;
            }
            "--stale-import-task-seconds" => {
                options.stale_import_task_seconds = Some(nonnegative_i64(args.get(index + 1))?);
                index += 2;
            }
            "--import-retry-backoff-seconds" => {
                options.import_retry_backoff_seconds = Some(nonnegative_i64(args.get(index + 1))?);
                index += 2;
            }
            "--resume-classifier-model" => {
                if options.classifier_model_configured {
                    return Err(DaemonError::usage(usage()));
                }
                let path = PathBuf::from(non_empty(args.get(index + 1))?);
                if !path.is_absolute() {
                    return Err(DaemonError::usage(usage()));
                }
                options.classifier_model_path = Some(path);
                options.classifier_model_configured = true;
                index += 2;
            }
            "--work-ocr-once" => {
                options.work_ocr_once = true;
                index += 1;
            }
            "--work-ocr" => {
                options.work_ocr = true;
                index += 1;
            }
            "--work-index-once" => {
                options.work_index_once = true;
                index += 1;
            }
            "--work-index" => {
                options.work_index = true;
                index += 1;
            }
            "--ocr-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(usage()));
                };
                if options.ocr_command.is_some() {
                    return Err(DaemonError::usage(usage()));
                }
                options.ocr_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-tesseract-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(usage()));
                };
                if options.ocr_tesseract_command.is_some() {
                    return Err(DaemonError::usage(usage()));
                }
                options.ocr_tesseract_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--pdf-render-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(usage()));
                };
                if options.pdf_render_command.is_some() {
                    return Err(DaemonError::usage(usage()));
                }
                options.pdf_render_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-engine-profile" => {
                options.ocr_engine_profile = non_empty(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-lang" => {
                options.ocr_lang = non_empty(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-profile" => {
                options.ocr_profile = non_empty(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-render-dpi" => {
                options.ocr_render_dpi = positive_u32(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-page-timeout-ms" => {
                options.ocr_page_timeout_ms = positive_u64(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-max-pages-per-document" => {
                options.ocr_max_pages_per_document = positive_u32(args.get(index + 1))?;
                index += 2;
            }
            "--ocr-jobs-per-tick" => {
                options.ocr_jobs_per_tick = Some(positive_usize_value(args.get(index + 1))?);
                index += 2;
            }
            "--embedding-command" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(DaemonError::usage(usage()));
                };
                if options.embedding_command.is_some() {
                    return Err(DaemonError::usage(usage()));
                }
                options.embedding_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--embedding-model-id" => {
                let value = non_empty(args.get(index + 1))?;
                if !valid_identifier(&value) {
                    return Err(DaemonError::usage(usage()));
                }
                options.embedding_model_id = Some(value);
                index += 2;
            }
            "--embedding-dimension" => {
                options.embedding_dimension = Some(positive_usize_value(args.get(index + 1))?);
                index += 2;
            }
            "--embedding-timeout-ms" => {
                options.embedding_timeout_ms = positive_u64(args.get(index + 1))?;
                index += 2;
            }
            #[cfg(feature = "resident-embedding-pool-experiment")]
            "--resident-embedding-pool-arm" => {
                if options.resident_embedding_pool_arm.is_some() {
                    return Err(DaemonError::usage(usage()));
                }
                let value = non_empty(args.get(index + 1))?;
                options.resident_embedding_pool_arm = ResidentEmbeddingPoolArm::parse(&value);
                if options.resident_embedding_pool_arm.is_none() {
                    return Err(DaemonError::usage(usage()));
                }
                index += 2;
            }
            "--worker-interval-ms" => {
                options.worker_interval_ms = Some(positive_u64(args.get(index + 1))?);
                index += 2;
            }
            "--max-worker-ticks" => {
                options.max_worker_ticks = Some(positive_usize_value(args.get(index + 1))?);
                index += 2;
            }
            _ => return Err(DaemonError::usage(usage())),
        }
    }

    if options.max_requests.is_some() && options.ipc_listen.is_none() {
        return Err(DaemonError::usage(
            "usage: --max-requests requires --ipc-listen",
        ));
    }
    if options.ocr_command.is_some() && options.ocr_tesseract_command.is_some() {
        return Err(DaemonError::usage(usage()));
    }
    #[cfg(feature = "resident-embedding-pool-experiment")]
    if options.resident_embedding_pool_arm.is_some()
        && (options.embedding_command.is_none()
            || options.embedding_model_id.is_none()
            || options.embedding_dimension.is_none())
    {
        return Err(DaemonError::usage(
            "usage: --resident-embedding-pool-arm requires a complete embedding runtime identity",
        ));
    }
    validation::validate(&options)?;
    Ok(options)
}

pub(crate) fn usage() -> &'static str {
    "usage: resume-daemon run --foreground [--parent-lifecycle-stdin --launch-id <64-lowercase-hex>] [--once] [--work-imports-once|--work-imports [--rescan-completed-imports] [--watch-import-roots] [--import-rescan-min-age-seconds <n>] [--stale-import-task-seconds <n>] [--import-retry-backoff-seconds <n>]] [--resume-classifier-model <absolute-path>] [--work-ocr-once|--work-ocr] [--work-index-once|--work-index] [--ocr-command <path>|--ocr-tesseract-command <path>] [--pdf-render-command <pdfium-path>] [--ocr-engine-profile <name>] [--ocr-lang <lang>] [--ocr-profile <profile>] [--ocr-render-dpi <dpi>] [--ocr-page-timeout-ms <ms>] [--ocr-max-pages-per-document <n>] [--ocr-jobs-per-tick <n>] [--embedding-command <path>] [--embedding-model-id <id>] [--embedding-dimension <n>] [--embedding-timeout-ms <ms>] [--worker-interval-ms <n>] [--max-worker-ticks <n>] [--ipc-listen <127.0.0.1:port>] [--expected-ipc-protocol <version>] [--max-requests <n>]"
}

fn non_empty(value: Option<&String>) -> Result<String> {
    let Some(value) = value else {
        return Err(DaemonError::usage(usage()));
    };
    if value.trim().is_empty() {
        return Err(DaemonError::usage(usage()));
    }
    Ok(value.clone())
}

fn positive_usize(value: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| DaemonError::usage(usage()))
}

fn positive_usize_value(value: Option<&String>) -> Result<usize> {
    positive_usize(value.ok_or_else(|| DaemonError::usage(usage()))?)
}

fn positive_u32(value: Option<&String>) -> Result<u32> {
    value
        .ok_or_else(|| DaemonError::usage(usage()))?
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| DaemonError::usage(usage()))
}

fn positive_u64(value: Option<&String>) -> Result<u64> {
    value
        .ok_or_else(|| DaemonError::usage(usage()))?
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| DaemonError::usage(usage()))
}

fn nonnegative_i64(value: Option<&String>) -> Result<i64> {
    value
        .ok_or_else(|| DaemonError::usage(usage()))?
        .parse::<i64>()
        .ok()
        .filter(|value| *value >= 0)
        .ok_or_else(|| DaemonError::usage(usage()))
}

fn valid_identifier(value: &str) -> bool {
    !value.trim().is_empty()
        && !value.contains('\n')
        && !value.contains('\r')
        && !value.contains('\t')
}

fn valid_launch_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn loopback_addr(value: &str) -> Result<SocketAddr> {
    let addr = SocketAddr::from_str(value).map_err(|_| DaemonError::usage(usage()))?;
    if !addr.ip().is_loopback() {
        return Err(DaemonError::usage("ipc listener must bind to loopback"));
    }
    Ok(addr)
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "resident-embedding-pool-experiment")]
    use super::ResidentEmbeddingPoolArm;
    use super::{parse, ParentLifecycleMode};

    #[cfg(not(feature = "resident-embedding-pool-experiment"))]
    #[test]
    fn ordinary_build_rejects_resident_pool_arm() {
        assert!(parse(&[
            "--foreground".into(),
            "--resident-embedding-pool-arm".into(),
            "i3_bulk1x4_b4".into(),
        ])
        .is_err());
    }

    #[cfg(feature = "resident-embedding-pool-experiment")]
    #[test]
    fn resident_pool_arm_parser_is_closed_and_single_assignment() {
        for (value, expected) in [
            ("i3_bulk1x4_b4", ResidentEmbeddingPoolArm::I3Bulk1x4B4),
            ("i3_bulk2x2_b4", ResidentEmbeddingPoolArm::I3Bulk2x2B4),
            ("i3_bulk2x3_b4", ResidentEmbeddingPoolArm::I3Bulk2x3B4),
        ] {
            let options = parse(&resident_pool_args(value)).unwrap();
            assert_eq!(options.resident_embedding_pool_arm, Some(expected));
        }
        for values in [vec!["unknown"], vec!["i3_bulk1x4_b4", "i3_bulk2x2_b4"]] {
            let mut args = resident_pool_args(values[0]);
            for value in values.into_iter().skip(1) {
                args.extend([
                    "--resident-embedding-pool-arm".to_string(),
                    value.to_string(),
                ]);
            }
            assert!(parse(&args).is_err());
        }
        assert!(parse(&[
            "--foreground".into(),
            "--resident-embedding-pool-arm".into(),
            "i3_bulk1x4_b4".into(),
        ])
        .is_err());
    }

    #[cfg(feature = "resident-embedding-pool-experiment")]
    fn resident_pool_args(arm: &str) -> Vec<String> {
        [
            "--foreground",
            "--embedding-command",
            "/synthetic/embedding-runtime",
            "--embedding-model-id",
            "synthetic-model",
            "--embedding-dimension",
            "4",
            "--resident-embedding-pool-arm",
            arm,
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    #[test]
    fn supervised_generation_requires_one_strict_launch_id() {
        assert!(parse(&["--foreground".into(), "--parent-lifecycle-stdin".into(),]).is_err());
        assert!(parse(&[
            "--foreground".into(),
            "--parent-lifecycle-stdin".into(),
            "--launch-id".into(),
            "A".repeat(64),
        ])
        .is_err());

        let options = parse(&[
            "--foreground".into(),
            "--parent-lifecycle-stdin".into(),
            "--launch-id".into(),
            "a".repeat(64),
        ])
        .unwrap();
        assert_eq!(options.parent_lifecycle, ParentLifecycleMode::Stdin);
        assert_eq!(options.launch_id, Some("a".repeat(64)));
    }

    #[test]
    fn once_and_supervisor_generation_are_mutually_exclusive() {
        assert!(parse(&[
            "--foreground".into(),
            "--once".into(),
            "--launch-id".into(),
            "b".repeat(64),
        ])
        .is_err());
    }
}
