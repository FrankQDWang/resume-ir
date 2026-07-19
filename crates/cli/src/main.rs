use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use benchmark_runner::{
    evaluate_benchmark_gate_json, evaluate_dedupe_quality_gate_json,
    evaluate_field_quality_gate_json, evaluate_ocr_throughput_gate_json,
    evaluate_vector_quality_gate_json, BenchmarkGateConfig, DedupeQualityGateConfig,
    FieldQualityGateConfig, OcrThroughputGateConfig, VectorQualityGateConfig,
};
use core_domain::{normalize_query_set_query, QuerySetSampleShape, QuerySetSourceKind};
use embedder::{
    Embedder, EmbeddingBudget, EmbeddingInput, LocalEmbeddingCommandEmbedder,
    LocalEmbeddingCommandSpec,
};
use fs4::fs_std::FileExt;
use fs_crawler::{crawl_directory_with_options, ScanOptions as CrawlerScanOptions};
use import_pipeline::{
    detect_ocr_page_count, import_root_with_options, index_claimed_ocr_text, ocr_preclaim_decision,
    prepare_migration_rebuild_artifacts, publish_search_projection_removals,
    rebuild_search_artifacts, reconcile_search_artifacts, ImportFailureKind, ImportHardwareTier,
    ImportMilestoneTimings, ImportOptions, ImportParseWorkers, ImportResourcePolicy, ImportSummary,
    ImportTaskOwnerLock, LinearPromotionPolicy, OcrPreclaimDecision, ScanProfile,
    SearchProjectionRemoval, SearchProjectionRemovalReason, SearchPublicationVectorization,
};
use meta_store::{
    backup_metadata_encryption_key, metadata_store_path, restore_metadata_encryption_key,
    CandidateId, ContactHash, Document, DocumentId, DocumentStatus, EntityMention, EntityType,
    FileExtension, ImportRootKind as StoreImportRootKind,
    ImportRootPreset as StoreImportRootPreset, ImportRootTaskHeadBatchOutcome,
    ImportRootTaskHeadBatchRejection, ImportRootTaskHeadOutcome, ImportRootTaskHeadRequest,
    ImportScanBudgetKind as StoreImportScanBudgetKind, ImportScanErrorSummary,
    ImportScanProfile as StoreImportScanProfile, ImportScanScope, ImportTask, ImportTaskId,
    ImportTaskStatus, IndexStateStatus, IngestJobFailureKind, IngestJobKind, IngestJobStatus,
    MetadataEncryptionState, OcrAttemptFailure, OcrPageCacheEntry, OcrPageCacheKey, OwnedMetaStore,
    PendingImportTaskByRootDiagnostic, QueryLatencySummary, ReadMetaStore, ResumeVersion,
    ResumeVersionId, SearchFilterCase, SearchProjectionFilter, SearchProjectionPredicate,
    SearchSelection, SearchSelectionDetailResolution, SearchTextBytePageRequest, UnixTimestamp,
    VectorSnapshotMode, WorkerTaskKind,
};
use ocr_client::{
    inspect_tesseract_language_availability, CancellationToken, LocalOcrCommandClient,
    LocalOcrCommandSpec, LocalPdfRenderCommandClient, LocalPdfRenderCommandSpec, OcrClient,
    OcrErrorKind, OcrOptions, OcrPageRequest, OcrWorkerBudget, PdftoppmPdfRenderer,
    PdftoppmRenderSpec, RenderedPage, TesseractLanguageAvailability, TesseractOcrClient,
    TesseractOcrSpec,
};
use privacy::{
    backup_contact_hash_key, inspect_contact_hash_key, redact_contact_values,
    restore_contact_hash_key, ContactHasher, ContactKind,
};
use rank_fusion::{
    fuse_hybrid_rrf, soft_dedupe_score, DateRange, DedupeProfile, DegreeLevel, HybridRecall,
    RankedHit, SchoolTier, SearchFilters,
};
use rusqlite::Connection;
use search_planner::plan_search;
use search_runtime::{
    FilterSelection, FullTextCandidate, HitLimit, HydratedSearchHit, QueryCoordinator, QueryScope,
    SearchRuntimeError, SearchRuntimeErrorCode, SelectionLimit, SemanticCandidate,
    SemanticContract, SemanticQueryVector,
};
use sha2::{Digest, Sha256};
use sysinfo::{
    get_current_pid, DiskRefreshKind, Disks, ProcessRefreshKind, ProcessesToUpdate, System,
};

mod import_processing;
mod purge_residual;
mod release_readiness_matrix;

use purge_residual::PurgeResidualProbe;
use release_readiness_matrix::release_readiness_goal_gap_matrix_json;

const LOCAL_DISCOVERY_ROOTS_ENV: &str = "RESUME_IR_LOCAL_DISCOVERY_ROOTS";
const LOCAL_DISCOVERY_DEFAULT_MAX_FILES: usize = 10_000;
const IPC_ENDPOINT_FILE: &str = "ipc.endpoints.json";
const IPC_AUTH_TOKEN_FILE: &str = "ipc.auth";
const IPC_ENDPOINT_SCHEMA_VERSION: &str = "resume-ir.daemon-ipc.v2";
const IPC_AUTH_SCHEMA_VERSION: &str = "resume-ir.daemon-auth.v2";
const SEARCH_IPC_REQUEST_SCHEMA_VERSION: &str = "resume-ir.ipc-request.v3";
const SEARCH_IPC_RESPONSE_SCHEMA_VERSION: &str = "resume-ir.search-response.v3";
const SEARCH_IPC_DEFAULT_DEADLINE_MS: u64 = 1_500;
const DETAIL_SCHEMA_VERSION: &str = "resume-ir.detail-response.v3";
const DETAIL_FIELD_LIMIT: usize = 256;
const SEARCH_RESULT_FILE_NAME_MAX_BYTES: usize = 160;
const DEFAULT_SERVICE_LABEL: &str = "com.resume-ir.daemon";
const DEFAULT_SERVICE_IPC_LISTEN: &str = "127.0.0.1:0";
const FAULT_PROBE_MAX_BYTES: u64 = 1024 * 1024;
const OCR_CRASH_PROBE_BYTES: &[u8] = b"SYNTHETIC OCR CRASH PROBE BYTES";
const MODEL_CHECKSUM_PROBE_BYTES: &[u8] = b"SYNTHETIC MODEL CHECKSUM PROBE\n";
const DEFAULT_OCR_MAX_PAGES_PER_DOCUMENT: u32 = 100;
const OCR_PAGE_BUDGET_REMEDIATION: &str =
    "raise OCR max pages per document or skip oversized scanned PDFs";
const OCR_LANGUAGE_REMEDIATION: &str =
    "install requested OCR language packs or choose an installed OCR language";
const METADATA_ENCRYPTION_REMEDIATION: &str =
    "enable SQLCipher metadata encryption before production release";
const DATASET_MANIFEST_SCHEMA_VERSION: &str = "resume-ir.dataset-manifest.v1";
const QUERY_SET_SCHEMA_VERSION: &str = "resume-ir.query-set.jsonl.v2";
const QUERY_SET_SUMMARY_SCHEMA_VERSION: &str = "resume-ir.query-set-summary.v2";
const QUERY_SET_TRACE_PREFLIGHT_SCHEMA_VERSION: &str = "resume-ir.query-set-trace-preflight.v1";
const QUERY_ARTIFACT_ROOT_ENV: &str = "RESUME_IR_QUERY_ARTIFACT_ROOT";
const LOCAL_EVIDENCE_DIR_ENV: &str = "RESUME_IR_LOCAL_EVIDENCE_DIR";
const QUERY_SET_TRACE_PREFLIGHT_DEFAULT_FILE: &str = "query-set-trace-preflight.local.json";
const PRIVATE_QUERY_SET_DEFAULT_FILE: &str = "private-query-set.local.jsonl";
const TRACE_QUERY_INSUFFICIENT_BASE_MESSAGE: &str =
    "query set blocked: not enough corpus-valid trace queries for the current indexed corpus";
const QUERY_BATCH_REQUEST_SCHEMA_VERSION: &str = "resume-ir.query-batch-request.v2";
const QUERY_PROTOCOL_VERSION: &str = "resume-ir-query-v2";
const TRACE_QUERY_LINE_MAX_BYTES: usize = 64 * 1024;
const QUERY_BUCKETS: [&str; 7] = [
    "single_term",
    "and_2",
    "and_3_5",
    "and_6_16",
    "field_filter",
    "hybrid",
    "semantic",
];
const D10K_TRACE_QUERY_SET_COUNT: usize = 500;
const D10K_TRACE_QUERY_BUCKET_MIN_COUNTS: [(&str, usize); 7] = [
    ("single_term", 50),
    ("and_2", 75),
    ("and_3_5", 150),
    ("and_6_16", 50),
    ("field_filter", 75),
    ("hybrid", 75),
    ("semantic", 25),
];
const MODEL_MANIFEST_SCHEMA_VERSION: &str = "resume-ir.model-manifest.v1";
const OCR_RUNTIME_MANIFEST_SCHEMA_VERSION: &str = "resume-ir.ocr-runtime-manifest.v1";
const FIELD_FILTER_CONFIDENCE_THRESHOLD: f32 = 0.75;
const WITNESS_DEFAULT_MAX_FILES: usize = 10_000;
const WITNESS_SEARCH_PROBE_LIMIT: usize = 5;
const WITNESS_SEARCH_PROBE_MAX_CANDIDATES: usize = 64;
const WITNESS_FIELD_LABELS: &[&str] = &[
    "name",
    "email",
    "phone",
    "wechat",
    "school",
    "major",
    "degree",
    "company",
    "title",
    "education",
    "skill",
    "certificate",
    "date",
    "date_range",
    "years_experience",
    "location",
];
const WITNESS_IMPORT_FAILURE_KINDS: &[ImportFailureKind] = &[
    ImportFailureKind::TextTooLarge,
    ImportFailureKind::ReadError,
    ImportFailureKind::UnsupportedExtension,
    ImportFailureKind::ParserUnsupported,
    ImportFailureKind::ParserCorrupted,
    ImportFailureKind::ParserEncrypted,
    ImportFailureKind::ParserTimeout,
    ImportFailureKind::ParserResourceExhausted,
    ImportFailureKind::ParserIo,
    ImportFailureKind::ParserCancelled,
    ImportFailureKind::ParserInternal,
    ImportFailureKind::EmptyText,
];
const TOP_LEVEL_USAGE: &str = "expected command: status, import, search, benchmark-query-set, benchmark-query-protocol, benchmark-corpus-summary, detail, delete, purge, cancel, pause, resume, ocr-worker, candidate-review, model, ocr, privacy, service, fault-simulate, witness, doctor, export-diagnostics, or release-readiness";
const TOP_LEVEL_HELP: &str = "\
resume-cli

Local-first resume import and search.

Usage:
  resume-cli [--data-dir <local-data-dir>] <command> [options]
  resume-cli --help

Core operator workflows:
  import                Import Word/PDF resume roots into local metadata and indexes.
  status                Show local task, OCR, full-text, vector, and index state.
  search                Search fulltext, field-filtered, semantic, or hybrid indexes.
  detail                Show a redacted resume detail view by document id.
  delete | purge        Hide deleted resumes from search, then purge local data.

Runtime and worker commands:
  ocr preflight         Check local OCR renderer/engine/language runtime.
  ocr-worker           Process queued scanned-PDF OCR jobs.
  model preflight       Check a local embedding command and reviewed model manifest.
  pause | resume        Pause or resume OCR work.

Diagnostics and release evidence:
  doctor                Inspect local metadata, index, runtime, and diagnostic state.
  export-diagnostics --redact
                        Emit local aggregate diagnostics without paths, queries, or resume text.
  benchmark-query-set   Preflight or freeze local private query-set evidence.
  benchmark-query-protocol
                        Run the local query protocol for benchmark evidence.
  benchmark-corpus-summary
                        Emit redacted aggregate corpus observability.
  fault-simulate        Run local-safe synthetic fault probes.
  release-readiness     Report stable-release blockers and provided evidence.

Current-stage boundary:
  Core local import/search closure can be verified locally. Stable release remains
  blocked by external evidence, credentials, platform transcripts, runtime/model
  review, and private quality data. Performance optimization is deferred.
";
const RELEASE_READINESS_PERFORMANCE_LABEL: &str = "private real-corpus performance evidence";
const RELEASE_READINESS_FIELD_QUALITY_LABEL: &str = "field extraction quality";
const RELEASE_READINESS_DEDUPE_QUALITY_LABEL: &str = "dedupe quality";
const RELEASE_READINESS_VECTOR_QUALITY_LABEL: &str = "vector quality";
const RELEASE_READINESS_OCR_THROUGHPUT_LABEL: &str = "OCR throughput";
const RELEASE_READINESS_OCR_LICENSE_LABEL: &str = "OCR runtime manifest/dependency evidence";
const RELEASE_READINESS_OCR_MANIFEST_EVIDENCE_LABEL: &str = "OCR runtime manifest evidence";
const RELEASE_READINESS_MODEL_LICENSE_LABEL: &str = "embedding model license/distribution";
const RELEASE_READINESS_MODEL_MANIFEST_EVIDENCE_LABEL: &str = "embedding model manifest evidence";
const RELEASE_READINESS_DIAGNOSTICS_LABEL: &str = "redacted diagnostics evidence";
const RELEASE_READINESS_RELEASE_ARTIFACT_MANIFEST_LABEL: &str =
    "release artifact manifest evidence";
const CURRENT_STAGE_D10K_SCALE_GATE: &str = "D10K_private_calibration";
const CURRENT_STAGE_D10K_DOCUMENT_MIN: u64 = 10_000;
const CURRENT_STAGE_D10K_SEARCHABLE_DOCUMENT_MIN: u64 = 8_000;
const CURRENT_STAGE_D10K_VECTOR_DOCUMENT_MIN: u64 = 8_000;
const CURRENT_STAGE_D10K_QUERY_MIN: u64 = 500;
const CURRENT_STAGE_D10K_REQUEST_SAMPLE_MIN: u64 = 5_000;
const CURRENT_STAGE_D10K_SAMPLES_PER_BUCKET_MIN: u64 = 500;
const RELEASE_READINESS_RELEASE_SBOM_LABEL: &str = "release SBOM evidence";
const RELEASE_READINESS_RELEASE_PUBLICATION_AUTOMATION_LABEL: &str =
    "GitHub Release publication automation evidence";
const RELEASE_READINESS_GITHUB_PUBLICATION_GATE_LABEL: &str =
    "GitHub Release publication gate evidence";
const RELEASE_READINESS_MACOS_PACKAGE_MANIFEST_LABEL: &str = "macOS package manifest evidence";
const RELEASE_READINESS_WINDOWS_PACKAGE_MANIFEST_LABEL: &str = "Windows package manifest evidence";
const RELEASE_READINESS_SIGNING_AUTOMATION_LABEL: &str = "signing automation evidence";
const RELEASE_READINESS_NOTARIZATION_AUTOMATION_LABEL: &str = "notarization automation evidence";
const RELEASE_READINESS_MACOS_INSTALLER_AUTOMATION_LABEL: &str =
    "macOS installer automation evidence";
const RELEASE_READINESS_WINDOWS_INSTALLER_AUTOMATION_LABEL: &str =
    "Windows installer automation evidence";
const RELEASE_READINESS_WINDOWS_SERVICE_AUTOMATION_LABEL: &str =
    "Windows service automation evidence";
const RELEASE_READINESS_MACOS_INSTALLER_LIFECYCLE_PLAN_LABEL: &str =
    "macOS installer lifecycle plan evidence";
const RELEASE_READINESS_WINDOWS_INSTALLER_LIFECYCLE_PLAN_LABEL: &str =
    "Windows installer lifecycle plan evidence";
const RELEASE_READINESS_WINDOWS_SERVICE_LIFECYCLE_PLAN_LABEL: &str =
    "Windows service lifecycle plan evidence";
const RELEASE_READINESS_CURRENT_STAGE_EVIDENCE_LABEL: &str =
    "current-stage validation evidence manifest";
const RELEASE_READINESS_CURRENT_STAGE_BLOCKED_HANDOFF_LABEL: &str = "current-stage blocked handoff";
const RELEASE_READINESS_HARDWARE_FAULT_DRILLS_LABEL: &str = "hardware fault drills";
const RELEASE_READINESS_BENCHMARK_MIN_DOCUMENTS: usize = 8_000;
struct ReleaseReadinessBlocker {
    label: &'static str,
    detail: &'static str,
    dependency_kind: &'static str,
    needed_from: &'static str,
    dependency_summary: &'static str,
    next_action: &'static str,
}

const RELEASE_READINESS_BLOCKERS: &[ReleaseReadinessBlocker] = &[
    ReleaseReadinessBlocker {
        label: "signing certificates",
        detail: "production signing certificates are not available; release evidence requires certificate chain, private key custody, and signature verification evidence for every release artifact",
        dependency_kind: "release_credentials",
        needed_from: "human_release_owner",
        dependency_summary: "production signing certificate chain, private-key custody policy, and artifact signature verification evidence",
        next_action: "provide signing certificate material through the documented CI secret interface and rerun release-readiness",
    },
    ReleaseReadinessBlocker {
        label: "macOS notarization",
        detail: "Apple Developer ID notarization credentials and ticket evidence are not available; release evidence requires notarization ticket stapling plus Gatekeeper validation on fresh macOS release artifacts",
        dependency_kind: "release_credentials",
        needed_from: "human_release_owner",
        dependency_summary: "Apple Developer ID credentials plus notarization submission, stapled ticket, and Gatekeeper transcript evidence",
        next_action: "provide Apple notarization credentials through CI secrets and run the macOS notarization release gate",
    },
    ReleaseReadinessBlocker {
        label: "Tauri v2 desktop installer composition",
        detail: "legacy CLI/daemon package automation and lifecycle dry-runs do not prove ordinary-user Tauri installers; unsigned macOS arm64 app/DMG composition, local lifecycle, real-version upgrade, and injected post-swap rollback are now verified, while release still requires a Windows per-user NSIS runtime closure plus signed and notarized macOS distribution evidence",
        dependency_kind: "local_product_implementation",
        needed_from: "desktop_runtime_composition",
        dependency_summary: "self-contained Windows per-user NSIS runtime closure and signed/notarized macOS release composition",
        next_action: "finish the Windows target-triple runtime closure and NSIS artifact, then sign and notarize the verified macOS artifact before clean-host release lifecycle testing",
    },
    ReleaseReadinessBlocker {
        label: "Windows installer lifecycle",
        detail: "legacy Windows MSI lifecycle dry-run automation exists but does not prove the target product; release evidence requires a self-contained per-user Tauri NSIS setup on a clean H0 host with install, first run, upgrade, uninstall, rollback, and recovery transcripts",
        dependency_kind: "release_platform_transcript",
        needed_from: "windows_h0_validation_host",
        dependency_summary: "clean-H0 per-user Tauri NSIS install, first-run, upgrade, uninstall, rollback, and recovery transcripts for a fresh self-contained artifact",
        next_action: "install the fresh per-user NSIS artifact on the clean H0 validation host and record bounded redacted lifecycle evidence",
    },
    ReleaseReadinessBlocker {
        label: "macOS installer lifecycle",
        detail: "legacy macOS pkg and LaunchAgent lifecycle dry-runs are not product proof; local unsigned install, first run, data-preserving uninstall, reinstall, real-version upgrade, and injected post-swap rollback are verified, while release evidence still requires the same lifecycle on a signed and notarized Tauri app/DMG with clean-host recovery and Gatekeeper validation",
        dependency_kind: "release_platform_transcript",
        needed_from: "macos_release_runner",
        dependency_summary: "fresh signed and notarized Tauri app/DMG lifecycle and Gatekeeper transcripts",
        next_action: "run the fresh Tauri app/DMG lifecycle on a clean macOS validation host and attach bounded redacted Gatekeeper/install evidence",
    },
    ReleaseReadinessBlocker {
        label: "GitHub Release publication",
        detail: "GitHub Release publication is not approved or proven; release evidence requires human release approval, a working GitHub Actions release token or Git credential path, and artifact upload evidence for fresh signed/notarized release artifacts",
        dependency_kind: "release_publication_approval",
        needed_from: "human_release_owner",
        dependency_summary: "human release approval, GitHub Actions release token or Git credential readiness, and GitHub Release artifact upload evidence",
        next_action: "approve the release publication gate, provide a working GitHub release token through CI secrets or repair Git credential access, then run the GitHub Release upload workflow and attach redacted upload evidence",
    },
    ReleaseReadinessBlocker {
        label: RELEASE_READINESS_PERFORMANCE_LABEL,
        detail: "stable-release private real-corpus hot-index hybrid benchmark evidence is not available; the current goal can close with local import/search closure evidence and a redacted current-stage handoff, while the D10K 10000/8000-document hot-index floor, 500 query samples, P50/P95/P99 metrics, P95/P99 reduction, and external 100k/1M scale validation move to the follow-up performance-optimization goal",
        dependency_kind: "local_current_stage_evidence",
        needed_from: "local_private_validation_run",
        dependency_summary: "redacted stable-release hot-index hybrid benchmark evidence over the available local private corpus",
        next_action: "carry this blocker into the performance-optimization goal, then rerun current-stage validation with reviewed local OCR/model manifests and attach full redacted benchmark evidence",
    },
    ReleaseReadinessBlocker {
        label: RELEASE_READINESS_FIELD_QUALITY_LABEL,
        detail: "private business labeled field-quality evidence is not available; release evidence requires min-samples 1000 and precision/recall/F1 >= 0.93 across required production fields",
        dependency_kind: "private_labeled_quality_dataset",
        needed_from: "business_labeling_process",
        dependency_summary: "private field-quality labeled dataset and aggregate precision/recall/F1 report for required production fields",
        next_action: "produce the private field-quality report and run the field quality gate before release-readiness",
    },
    ReleaseReadinessBlocker {
        label: RELEASE_READINESS_DEDUPE_QUALITY_LABEL,
        detail: "private business labeled dedupe-quality evidence is not available; release evidence requires min-pairs 1000, min-positive-pairs 100, and precision/recall/F1 >= 0.90",
        dependency_kind: "private_labeled_quality_dataset",
        needed_from: "business_labeling_process",
        dependency_summary: "private dedupe labeled pair dataset with enough positive pairs and aggregate precision/recall/F1 evidence",
        next_action: "produce the private dedupe-quality report and run the dedupe quality gate before release-readiness",
    },
    ReleaseReadinessBlocker {
        label: RELEASE_READINESS_VECTOR_QUALITY_LABEL,
        detail: "private business labeled vector-quality evidence is not available; release evidence requires min-samples 1000, recall@k >= 0.90, MRR >= 0.85, NDCG@k >= 0.90, and zero-recall queries blocked",
        dependency_kind: "private_labeled_quality_dataset",
        needed_from: "business_labeling_process",
        dependency_summary: "private vector-quality labeled query set with recall@k, MRR, NDCG@k, and zero-recall evidence",
        next_action: "produce the private vector-quality report and run the vector quality gate before release-readiness",
    },
    ReleaseReadinessBlocker {
        label: RELEASE_READINESS_OCR_THROUGHPUT_LABEL,
        detail: "stable-release private real-corpus OCR throughput evidence is not available; the current goal may close with local OCR runtime preflight plus a redacted blocked handoff when the OCR backlog exceeds the interaction budget, while min-pages 500, OCR page latency P50/P95/P99 metrics, pages_per_second, no run-budget exhaustion, and OCR throughput reduction move to the follow-up performance-optimization goal",
        dependency_kind: "local_current_stage_evidence",
        needed_from: "local_private_validation_run",
        dependency_summary: "redacted stable-release private real-corpus OCR throughput evidence with observed page latency percentiles and throughput",
        next_action: "carry this blocker into the performance-optimization goal, then run the OCR throughput baseline with reviewed OCR manifests and attach the redacted report",
    },
    ReleaseReadinessBlocker {
        label: RELEASE_READINESS_OCR_LICENSE_LABEL,
        detail: "Tesseract/tessdata is the accepted Apache-2.0 OCR runtime direction, and the PDF renderer must follow bundled-first packaging with external override; if Poppler/pdftoppm is bundled, release evidence requires GPL-3.0-or-later-compatible distribution review, source-offer obligations, checksums/licenses, dependency detection, and fail-closed operator guidance",
        dependency_kind: "reviewed_runtime_manifest",
        needed_from: "local_runtime_review",
        dependency_summary: "reviewed Tesseract/tessdata and PDF renderer runtime manifest with distribution mode, checksums, licenses, source-offer obligations, and dependency detection evidence",
        next_action: "generate and review the OCR runtime manifest, then pass it to release-readiness",
    },
    ReleaseReadinessBlocker {
        label: RELEASE_READINESS_MODEL_LICENSE_LABEL,
        detail: "reviewed licensed embedding model selection, model manifest, offline distribution, and license review evidence are not complete",
        dependency_kind: "reviewed_model_license",
        needed_from: "legal_model_review",
        dependency_summary: "approved local embedding model, artifact manifest, offline distribution plan, and license review evidence",
        next_action: "complete model license/distribution review and attach the reviewed model manifest",
    },
    ReleaseReadinessBlocker {
        label: "cross-platform release validation",
        detail: "hosted Rust workspace checks and legacy dry-run packaging evidence exist, but native Tauri product validation is incomplete; release evidence requires fresh self-contained macOS app/DMG and Windows per-user NSIS artifacts plus Tauri GUI and installer lifecycle proof on clean hosts",
        dependency_kind: "release_platform_transcript",
        needed_from: "macos_windows_release_runners",
        dependency_summary: "fresh Tauri desktop artifact, GUI, and installer lifecycle validation on clean macOS and Windows hosts",
        next_action: "run native Tauri desktop and installer validation on clean macOS and Windows hosts and attach bounded redacted lifecycle evidence",
    },
    ReleaseReadinessBlocker {
        label: RELEASE_READINESS_DIAGNOSTICS_LABEL,
        detail: "redacted local aggregate diagnostics evidence is not available; release evidence requires export-diagnostics --redact output with diagnostics.v1 schema, local aggregate diagnostics scope, and redacted paths, queries, and resume text",
        dependency_kind: "redacted_local_evidence",
        needed_from: "local_validation_run",
        dependency_summary: "redacted diagnostics.v1 aggregate output from the validation data directory",
        next_action: "run export-diagnostics --redact on the validation data directory and pass the redacted report to release-readiness",
    },
    ReleaseReadinessBlocker {
        label: RELEASE_READINESS_HARDWARE_FAULT_DRILLS_LABEL,
        detail: "actual ENOSPC, service-level daemon kill, battery-mode, and external-drive disconnect drills are not proven on release platforms",
        dependency_kind: "hardware_release_platform_drill",
        needed_from: "dedicated_release_platforms",
        dependency_summary: "actual ENOSPC, service kill, battery-mode, and external-drive disconnect drill transcripts from release platforms",
        next_action: "run actual hardware fault drills on dedicated release platforms and attach redacted transcript digests",
    },
];

fn main() {
    if let Err(error) = run() {
        eprintln!("resume-cli: {error}");
        std::process::exit(error.exit_code());
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();

    if args == ["--identity"] {
        println!("resume-cli");
        return Ok(());
    }

    if is_top_level_help(&args) {
        print_top_level_help();
        return Ok(());
    }
    if let Some(topic) = command_help_topic(&args) {
        print_command_help(topic)?;
        return Ok(());
    }

    let data_dir = take_data_dir(&mut args)?;
    if is_top_level_help(&args) {
        print_top_level_help();
        return Ok(());
    }
    if let Some(topic) = command_help_topic(&args) {
        print_command_help(topic)?;
        return Ok(());
    }

    let Some(command) = args.first().map(String::as_str) else {
        return Err(CliError::usage(TOP_LEVEL_USAGE));
    };

    match command {
        "status" => status_command(&data_dir, &args[1..]),
        "import" => import_command(&data_dir, &args[1..]),
        "search" => search_command(&data_dir, &args[1..]),
        "benchmark-query-set" => benchmark_query_set_command(&data_dir, &args[1..]),
        "benchmark-query-protocol" => benchmark_query_protocol_command(&data_dir, &args[1..]),
        "benchmark-corpus-summary" => benchmark_corpus_summary_command(&data_dir, &args[1..]),
        "detail" => detail_command(&data_dir, &args[1..]),
        "delete" => delete_command(&data_dir, &args[1..]),
        "purge" => purge_command(&data_dir, &args[1..]),
        "cancel" => cancel_command(&data_dir, &args[1..]),
        "pause" => task_control_command(&data_dir, &args[1..], true),
        "resume" => task_control_command(&data_dir, &args[1..], false),
        "ocr-worker" => ocr_worker_command(&data_dir, &args[1..]),
        "candidate-review" => candidate_review_command(&data_dir, &args[1..]),
        "model" => model_command(&args[1..]),
        "ocr" => ocr_command(&args[1..]),
        "privacy" => privacy_command(&data_dir, &args[1..]),
        "service" => service_command(&data_dir, &args[1..]),
        "fault-simulate" => fault_simulate_command(&data_dir, &args[1..]),
        "witness" => witness_command(&args[1..]),
        "doctor" => doctor_command(&data_dir, &args[1..]),
        "export-diagnostics" => export_diagnostics_command(&data_dir, &args[1..]),
        "release-readiness" => release_readiness_command(&args[1..]),
        _ => Err(CliError::usage(TOP_LEVEL_USAGE)),
    }
}

fn is_top_level_help(args: &[String]) -> bool {
    matches!(args, [arg] if is_help_flag(arg) || arg == "help")
}

fn is_help_flag(value: &str) -> bool {
    value == "--help" || value == "-h"
}

fn command_help_topic(args: &[String]) -> Option<&str> {
    match args {
        [command, topic] if command == "help" => Some(topic.as_str()),
        [command, ..] if command == "--data-dir" => None,
        [command, rest @ ..] if command != "help" && rest.iter().any(|arg| is_help_flag(arg)) => {
            Some(command.as_str())
        }
        _ => None,
    }
}

fn print_top_level_help() {
    print!("{TOP_LEVEL_HELP}");
}

fn print_command_help(topic: &str) -> Result<()> {
    let usage = command_usage(topic).ok_or_else(|| CliError::usage(TOP_LEVEL_USAGE))?;
    println!("{usage}");
    Ok(())
}

fn command_usage(topic: &str) -> Option<&'static str> {
    match topic {
        "status" => Some(status_usage()),
        "import" => Some(import_usage_text()),
        "search" => Some(search_usage()),
        "benchmark-query-set" => Some(benchmark_query_set_usage()),
        "benchmark-query-protocol" => Some(benchmark_query_protocol_usage()),
        "benchmark-corpus-summary" => Some(benchmark_corpus_summary_usage()),
        "detail" => Some(detail_usage()),
        "delete" => Some(delete_usage()),
        "purge" => Some(purge_usage()),
        "cancel" => Some(cancel_usage_text()),
        "pause" | "resume" => Some(task_control_usage_text()),
        "ocr-worker" => Some(ocr_worker_usage_text()),
        "candidate-review" => Some(candidate_review_usage()),
        "model" => Some(model_usage()),
        "ocr" => Some(ocr_usage()),
        "privacy" => Some(privacy_usage()),
        "service" => Some(service_usage()),
        "fault-simulate" => Some(fault_simulate_usage()),
        "witness" => Some(witness_usage_text()),
        "doctor" => Some(doctor_usage()),
        "export-diagnostics" => Some(export_diagnostics_usage()),
        "release-readiness" => Some(release_readiness_usage()),
        _ => None,
    }
}

fn release_readiness_command(args: &[String]) -> Result<()> {
    let args = parse_release_readiness_args(args)?;
    let provided_evidence = validate_release_readiness_evidence(&args.evidence)?;

    if args.json {
        let provided_labels = provided_evidence
            .iter()
            .map(|evidence| evidence.label)
            .collect::<BTreeSet<_>>();
        let blockers = RELEASE_READINESS_BLOCKERS
            .iter()
            .filter(|blocker| !provided_labels.contains(blocker.label))
            .map(|blocker| {
                serde_json::json!({
                    "label": blocker.label,
                    "status": "blocked",
                    "detail": blocker.detail,
                    "blocked_dependency": {
                        "kind": blocker.dependency_kind,
                        "needed_from": blocker.needed_from,
                        "summary": blocker.dependency_summary,
                    },
                    "next_action": blocker.next_action,
                })
            })
            .collect::<Vec<_>>();
        let provided_evidence = provided_evidence
            .iter()
            .map(|evidence| {
                serde_json::json!({
                    "label": evidence.label,
                    "status": "provided",
                    "privacy_boundary": evidence.privacy_boundary,
                    "detail": evidence.detail,
                })
            })
            .collect::<Vec<_>>();
        let report = serde_json::json!({
            "schema_version": "release-readiness.v1",
            "stable_release": "blocked",
            "local_dry_run_artifacts": "evidence_only",
            "provided_evidence": provided_evidence,
            "blockers": blockers,
            "goal_gap_matrix": release_readiness_goal_gap_matrix_json(),
            "next_gate": "keep release blocked until every item has current local evidence",
        });
        let report = serde_json::to_string_pretty(&report)
            .map_err(|_| CliError::user("release readiness report unavailable"))?;
        println!("{report}");
        return Err(CliError::user(
            "release readiness blocked: stable release criteria are not met",
        ));
    }

    println!("resume-ir release readiness");
    println!("stable release: blocked");
    println!("local dry-run artifacts: evidence only");
    if !provided_evidence.is_empty() {
        println!("provided local evidence:");
        for evidence in &provided_evidence {
            println!("- {}: provided ({})", evidence.label, evidence.detail);
        }
    }
    println!("blocked evidence:");
    let provided_labels = provided_evidence
        .iter()
        .map(|evidence| evidence.label)
        .collect::<BTreeSet<_>>();
    for blocker in RELEASE_READINESS_BLOCKERS {
        if provided_labels.contains(blocker.label) {
            continue;
        }
        println!("- {}: blocked ({})", blocker.label, blocker.detail);
        println!(
            "  needs: {} from {} ({})",
            blocker.dependency_kind, blocker.needed_from, blocker.dependency_summary
        );
        println!("  next action: {}", blocker.next_action);
    }
    println!("next gate: keep release blocked until every item has current local evidence");

    Err(CliError::user(
        "release readiness blocked: stable release criteria are not met",
    ))
}

fn release_readiness_usage() -> &'static str {
    "usage: resume-cli release-readiness [--json] [--benchmark-report <path>] [--field-quality-report <path>] [--dedupe-quality-report <path>] [--vector-quality-report <path>] [--ocr-throughput-report <path>] [--model-manifest <path>] [--ocr-runtime-manifest <path>] [--diagnostics-report <path>] [--current-stage-evidence <path>] [--current-stage-blocked-summary <path>] [--release-artifact-manifest <path>] [--release-sbom <path>] [--release-publication-evidence <path>] [--github-release-publication-gate <path>] [--macos-package-manifest <path>] [--windows-package-manifest <path>] [--signing-evidence <path>] [--notarization-evidence <path>] [--macos-installer-evidence <path>] [--windows-installer-evidence <path>] [--windows-service-evidence <path>] [--macos-installer-lifecycle-plan <path>] [--windows-installer-lifecycle-plan <path>] [--windows-service-lifecycle-plan <path>] [--hardware-fault-evidence <path>]"
}

#[derive(Default)]
struct ReleaseReadinessEvidenceArgs {
    benchmark_report: Option<PathBuf>,
    field_quality_report: Option<PathBuf>,
    dedupe_quality_report: Option<PathBuf>,
    vector_quality_report: Option<PathBuf>,
    ocr_throughput_report: Option<PathBuf>,
    model_manifest: Option<PathBuf>,
    ocr_runtime_manifest: Option<PathBuf>,
    diagnostics_report: Option<PathBuf>,
    current_stage_evidence: Option<PathBuf>,
    current_stage_blocked_summary: Option<PathBuf>,
    release_artifact_manifest: Option<PathBuf>,
    release_sbom: Option<PathBuf>,
    release_publication_evidence: Option<PathBuf>,
    github_release_publication_gate: Option<PathBuf>,
    macos_package_manifest: Option<PathBuf>,
    windows_package_manifest: Option<PathBuf>,
    signing_evidence: Option<PathBuf>,
    notarization_evidence: Option<PathBuf>,
    macos_installer_evidence: Option<PathBuf>,
    windows_installer_evidence: Option<PathBuf>,
    windows_service_evidence: Option<PathBuf>,
    macos_installer_lifecycle_plan: Option<PathBuf>,
    windows_installer_lifecycle_plan: Option<PathBuf>,
    windows_service_lifecycle_plan: Option<PathBuf>,
    hardware_fault_evidence: Option<PathBuf>,
}

struct ReleaseReadinessArgs {
    json: bool,
    evidence: ReleaseReadinessEvidenceArgs,
}

struct ReleaseReadinessProvidedEvidence {
    label: &'static str,
    privacy_boundary: &'static str,
    detail: &'static str,
}

struct ReleaseAutomationEvidenceSpec {
    label: &'static str,
    schema_version: &'static str,
    status_key: &'static str,
    evidence_boundary: &'static str,
    digest_key: &'static str,
    require_planned_actions: bool,
}

fn parse_release_readiness_args(args: &[String]) -> Result<ReleaseReadinessArgs> {
    let mut parsed = ReleaseReadinessArgs {
        json: false,
        evidence: ReleaseReadinessEvidenceArgs::default(),
    };
    let mut index = 0_usize;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                parsed.json = true;
                index += 1;
            }
            "--benchmark-report" => {
                parsed.evidence.benchmark_report =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--field-quality-report" => {
                parsed.evidence.field_quality_report =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--dedupe-quality-report" => {
                parsed.evidence.dedupe_quality_report =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--vector-quality-report" => {
                parsed.evidence.vector_quality_report =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--ocr-throughput-report" => {
                parsed.evidence.ocr_throughput_report =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--model-manifest" => {
                parsed.evidence.model_manifest =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--ocr-runtime-manifest" => {
                parsed.evidence.ocr_runtime_manifest =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--diagnostics-report" => {
                parsed.evidence.diagnostics_report =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--current-stage-evidence" => {
                parsed.evidence.current_stage_evidence =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--current-stage-blocked-summary" => {
                parsed.evidence.current_stage_blocked_summary =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--release-artifact-manifest" => {
                parsed.evidence.release_artifact_manifest =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--release-sbom" => {
                parsed.evidence.release_sbom = Some(take_release_readiness_path(args, &mut index)?);
            }
            "--release-publication-evidence" => {
                parsed.evidence.release_publication_evidence =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--github-release-publication-gate" => {
                parsed.evidence.github_release_publication_gate =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--macos-package-manifest" => {
                parsed.evidence.macos_package_manifest =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--windows-package-manifest" => {
                parsed.evidence.windows_package_manifest =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--signing-evidence" => {
                parsed.evidence.signing_evidence =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--notarization-evidence" => {
                parsed.evidence.notarization_evidence =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--macos-installer-evidence" => {
                parsed.evidence.macos_installer_evidence =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--windows-installer-evidence" => {
                parsed.evidence.windows_installer_evidence =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--windows-service-evidence" => {
                parsed.evidence.windows_service_evidence =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--macos-installer-lifecycle-plan" => {
                parsed.evidence.macos_installer_lifecycle_plan =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--windows-installer-lifecycle-plan" => {
                parsed.evidence.windows_installer_lifecycle_plan =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--windows-service-lifecycle-plan" => {
                parsed.evidence.windows_service_lifecycle_plan =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            "--hardware-fault-evidence" => {
                parsed.evidence.hardware_fault_evidence =
                    Some(take_release_readiness_path(args, &mut index)?);
            }
            _ => return Err(CliError::usage(release_readiness_usage())),
        }
    }
    Ok(parsed)
}

fn take_release_readiness_path(args: &[String], index: &mut usize) -> Result<PathBuf> {
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(release_readiness_usage()));
    };
    *index += 2;
    Ok(PathBuf::from(value))
}

fn validate_release_readiness_evidence(
    args: &ReleaseReadinessEvidenceArgs,
) -> Result<Vec<ReleaseReadinessProvidedEvidence>> {
    if args.current_stage_evidence.is_some() && args.current_stage_blocked_summary.is_some() {
        return Err(CliError::user(
            "current-stage evidence conflict: use either --current-stage-evidence or --current-stage-blocked-summary, not both",
        ));
    }
    let mut provided = Vec::new();
    if let Some(path) = &args.benchmark_report {
        let report = read_release_readiness_evidence_report(path)?;
        let config = BenchmarkGateConfig::new(
            RELEASE_READINESS_BENCHMARK_MIN_DOCUMENTS,
            500,
            f64::INFINITY,
        )
        .with_max_zero_result_queries(0)
        .require_private_real_corpus();
        evaluate_benchmark_gate_json(&report, config).map_err(|error| {
            release_readiness_evidence_error(RELEASE_READINESS_PERFORMANCE_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_PERFORMANCE_LABEL,
            privacy_boundary: "redacted_local_aggregate",
            detail: "private real-corpus hot-index hybrid benchmark baseline passed reproducibility and redaction checks",
        });
    }
    if let Some(path) = &args.field_quality_report {
        let report = read_release_readiness_evidence_report(path)?;
        let config = FieldQualityGateConfig::new(0.93, 0.93, 0.93)
            .with_min_samples(1000)
            .require_private_business_labeled();
        evaluate_field_quality_gate_json(&report, config).map_err(|error| {
            release_readiness_evidence_error(RELEASE_READINESS_FIELD_QUALITY_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_FIELD_QUALITY_LABEL,
            privacy_boundary: "redacted_local_aggregate",
            detail: "private business field-quality report passed the local release gate",
        });
    }
    if let Some(path) = &args.dedupe_quality_report {
        let report = read_release_readiness_evidence_report(path)?;
        let config = DedupeQualityGateConfig::new(0.90, 0.90, 0.90)
            .with_min_pairs(1000)
            .with_min_positive_pairs(100)
            .require_private_business_labeled();
        evaluate_dedupe_quality_gate_json(&report, config).map_err(|error| {
            release_readiness_evidence_error(RELEASE_READINESS_DEDUPE_QUALITY_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_DEDUPE_QUALITY_LABEL,
            privacy_boundary: "redacted_local_aggregate",
            detail: "private business dedupe-quality report passed the local release gate",
        });
    }
    if let Some(path) = &args.vector_quality_report {
        let report = read_release_readiness_evidence_report(path)?;
        let config = VectorQualityGateConfig::new(1000, 0.90, 0.85, 0.90)
            .with_max_zero_recall_queries(0)
            .require_private_business_labeled();
        evaluate_vector_quality_gate_json(&report, config).map_err(|error| {
            release_readiness_evidence_error(RELEASE_READINESS_VECTOR_QUALITY_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_VECTOR_QUALITY_LABEL,
            privacy_boundary: "redacted_local_aggregate",
            detail: "private business vector-quality report passed the local release gate",
        });
    }
    if let Some(path) = &args.ocr_throughput_report {
        let report = read_release_readiness_evidence_report(path)?;
        let config = OcrThroughputGateConfig::current_stage_baseline(500);
        evaluate_ocr_throughput_gate_json(&report, config).map_err(|error| {
            release_readiness_evidence_error(RELEASE_READINESS_OCR_THROUGHPUT_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_OCR_THROUGHPUT_LABEL,
            privacy_boundary: "redacted_local_aggregate",
            detail:
                "private real-corpus OCR baseline report passed the current-stage evidence gate",
        });
    }
    if let Some(path) = &args.model_manifest {
        let validation = validate_model_manifest(path).map_err(|error| {
            release_readiness_manifest_error(RELEASE_READINESS_MODEL_LICENSE_LABEL, error)
        })?;
        if !validation
            .models
            .iter()
            .any(|model| model.model_type == "embedding")
        {
            return Err(release_readiness_manifest_error(
                RELEASE_READINESS_MODEL_LICENSE_LABEL,
                CliError::user("model manifest blocked: embedding model is not present"),
            ));
        }
        if release_readiness_model_distribution_is_packaged(args, &validation)? {
            provided.push(ReleaseReadinessProvidedEvidence {
                label: RELEASE_READINESS_MODEL_MANIFEST_EVIDENCE_LABEL,
                privacy_boundary: "reviewed_local_manifest",
                detail: "reviewed embedding model manifest passed checksum and license validation",
            });
            provided.push(ReleaseReadinessProvidedEvidence {
                label: RELEASE_READINESS_MODEL_LICENSE_LABEL,
                privacy_boundary: "blocked_release_evidence_manifest",
                detail: "reviewed embedding model manifest is bound to a matching packaged runtime payload",
            });
        } else {
            provided.push(ReleaseReadinessProvidedEvidence {
                label: RELEASE_READINESS_MODEL_MANIFEST_EVIDENCE_LABEL,
                privacy_boundary: "reviewed_local_manifest",
                detail: "model manifest is reviewed but package payload does not prove matching offline model distribution",
            });
        }
    }
    if let Some(path) = &args.ocr_runtime_manifest {
        let validation = validate_ocr_runtime_manifest(path).map_err(|error| {
            release_readiness_manifest_error(RELEASE_READINESS_OCR_LICENSE_LABEL, error)
        })?;
        validate_release_readiness_ocr_manifest_coverage(&validation)?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_OCR_MANIFEST_EVIDENCE_LABEL,
            privacy_boundary: "reviewed_local_manifest",
            detail: "reviewed bundled-first OCR runtime manifest with external override passed checksum, license, and component coverage validation",
        });
        if release_readiness_ocr_runtime_distribution_is_packaged(args, &validation)? {
            provided.push(ReleaseReadinessProvidedEvidence {
                label: RELEASE_READINESS_OCR_LICENSE_LABEL,
                privacy_boundary: "blocked_release_evidence_manifest",
                detail: "reviewed OCR runtime manifest is bound to a matching bundled runtime package payload",
            });
        }
    }
    if let Some(path) = &args.diagnostics_report {
        let report = read_release_readiness_evidence_report(path)?;
        validate_release_readiness_diagnostics_report(&report).map_err(|error| {
            release_readiness_manifest_error(RELEASE_READINESS_DIAGNOSTICS_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_DIAGNOSTICS_LABEL,
            privacy_boundary: "redacted_local_aggregate",
            detail: "diagnostics.v1 report passed local aggregate redaction and scope checks",
        });
    }
    if let Some(path) = &args.current_stage_evidence {
        let report = read_release_readiness_evidence_report(path)?;
        let digests = validate_current_stage_evidence_manifest(&report).map_err(|error| {
            release_readiness_manifest_error(RELEASE_READINESS_CURRENT_STAGE_EVIDENCE_LABEL, error)
        })?;
        validate_current_stage_evidence_bundle_digests(args, &digests).map_err(|error| {
            release_readiness_manifest_error(RELEASE_READINESS_CURRENT_STAGE_EVIDENCE_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_CURRENT_STAGE_EVIDENCE_LABEL,
            privacy_boundary: "local_only_redacted_evidence_manifest",
            detail:
                "current-stage validation evidence manifest passed redacted schema and digest checks",
        });
    }
    if let Some(path) = &args.current_stage_blocked_summary {
        let report = read_release_readiness_evidence_report(path)?;
        let digests =
            validate_current_stage_blocked_summary_manifest(&report).map_err(|error| {
                release_readiness_manifest_error(
                    RELEASE_READINESS_CURRENT_STAGE_BLOCKED_HANDOFF_LABEL,
                    error,
                )
            })?;
        validate_current_stage_blocked_summary_bundle_digests(args, &digests).map_err(|error| {
            release_readiness_manifest_error(
                RELEASE_READINESS_CURRENT_STAGE_BLOCKED_HANDOFF_LABEL,
                error,
            )
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_CURRENT_STAGE_BLOCKED_HANDOFF_LABEL,
            privacy_boundary: "local_only_redacted_blocked_summary",
            detail: "current-stage blocked summary passed redacted handoff checks; it does not clear full baseline evidence",
        });
    }
    if let Some(path) = &args.release_artifact_manifest {
        let report = read_release_readiness_evidence_report(path)?;
        validate_release_artifact_manifest_report(&report).map_err(|error| {
            release_readiness_manifest_error(
                RELEASE_READINESS_RELEASE_ARTIFACT_MANIFEST_LABEL,
                error,
            )
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_RELEASE_ARTIFACT_MANIFEST_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail:
                "release.artifacts.v1 dry-run manifest passed schema and artifact boundary checks",
        });
    }
    if let Some(path) = &args.release_sbom {
        let report = read_release_readiness_evidence_report(path)?;
        validate_release_sbom_report(&report).map_err(|error| {
            release_readiness_manifest_error(RELEASE_READINESS_RELEASE_SBOM_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_RELEASE_SBOM_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "SPDX-2.3 release dry-run SBOM passed redaction and package boundary checks",
        });
    }
    if let Some(path) = &args.release_publication_evidence {
        let report = read_release_readiness_evidence_report(path)?;
        validate_release_publication_evidence_report(&report).map_err(|error| {
            release_readiness_manifest_error(
                RELEASE_READINESS_RELEASE_PUBLICATION_AUTOMATION_LABEL,
                error,
            )
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_RELEASE_PUBLICATION_AUTOMATION_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "release.publication_evidence.v1 blocked dry-run evidence passed schema and publication boundary checks",
        });
    }
    if let Some(path) = &args.github_release_publication_gate {
        let report = read_release_readiness_evidence_report(path)?;
        let (privacy_boundary, detail) = validate_github_release_publication_gate_report(&report)
            .map_err(|error| {
            release_readiness_manifest_error(RELEASE_READINESS_GITHUB_PUBLICATION_GATE_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_GITHUB_PUBLICATION_GATE_LABEL,
            privacy_boundary,
            detail,
        });
    }
    if let Some(path) = &args.macos_package_manifest {
        let report = read_release_readiness_evidence_report(path)?;
        validate_macos_package_manifest_report(&report).map_err(|error| {
            release_readiness_manifest_error(RELEASE_READINESS_MACOS_PACKAGE_MANIFEST_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_MACOS_PACKAGE_MANIFEST_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail:
                "release.macos_package.v1 unsigned dry-run manifest passed package boundary checks",
        });
    }
    if let Some(path) = &args.windows_package_manifest {
        let report = read_release_readiness_evidence_report(path)?;
        validate_windows_package_manifest_report(&report).map_err(|error| {
            release_readiness_manifest_error(
                RELEASE_READINESS_WINDOWS_PACKAGE_MANIFEST_LABEL,
                error,
            )
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_WINDOWS_PACKAGE_MANIFEST_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail:
                "release.windows_package.v1 unsigned dry-run manifest passed package boundary checks",
        });
    }
    if let Some(path) = &args.signing_evidence {
        validate_release_automation_evidence(path, &SIGNING_AUTOMATION_EVIDENCE_SPEC)?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_SIGNING_AUTOMATION_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "release.signing_evidence.v1 blocked dry-run evidence passed schema and boundary checks",
        });
    }
    if let Some(path) = &args.notarization_evidence {
        validate_release_automation_evidence(path, &NOTARIZATION_AUTOMATION_EVIDENCE_SPEC)?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_NOTARIZATION_AUTOMATION_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "release.notarization_evidence.v1 blocked dry-run evidence passed schema and boundary checks",
        });
    }
    if let Some(path) = &args.macos_installer_evidence {
        validate_release_automation_evidence(path, &MACOS_INSTALLER_AUTOMATION_EVIDENCE_SPEC)?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_MACOS_INSTALLER_AUTOMATION_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "release.macos_installer_evidence.v1 blocked dry-run evidence passed schema and boundary checks",
        });
    }
    if let Some(path) = &args.windows_installer_evidence {
        validate_release_automation_evidence(path, &WINDOWS_INSTALLER_AUTOMATION_EVIDENCE_SPEC)?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_WINDOWS_INSTALLER_AUTOMATION_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "release.windows_installer_evidence.v1 blocked dry-run evidence passed schema and boundary checks",
        });
    }
    if let Some(path) = &args.windows_service_evidence {
        validate_release_automation_evidence(path, &WINDOWS_SERVICE_AUTOMATION_EVIDENCE_SPEC)?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_WINDOWS_SERVICE_AUTOMATION_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "release.windows_service_evidence.v1 blocked dry-run evidence passed schema and boundary checks",
        });
    }
    if let Some(path) = &args.macos_installer_lifecycle_plan {
        let report = read_release_readiness_evidence_report(path)?;
        validate_macos_installer_lifecycle_plan_report(&report).map_err(|error| {
            release_readiness_manifest_error(
                RELEASE_READINESS_MACOS_INSTALLER_LIFECYCLE_PLAN_LABEL,
                error,
            )
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_MACOS_INSTALLER_LIFECYCLE_PLAN_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "release.macos_installer_lifecycle_plan.v1 dry-run operator plan passed schema and boundary checks",
        });
    }
    if let Some(path) = &args.windows_installer_lifecycle_plan {
        let report = read_release_readiness_evidence_report(path)?;
        validate_windows_installer_lifecycle_plan_report(&report).map_err(|error| {
            release_readiness_manifest_error(
                RELEASE_READINESS_WINDOWS_INSTALLER_LIFECYCLE_PLAN_LABEL,
                error,
            )
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_WINDOWS_INSTALLER_LIFECYCLE_PLAN_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "release.windows_installer_lifecycle_plan.v1 dry-run operator plan passed schema and boundary checks",
        });
    }
    if let Some(path) = &args.windows_service_lifecycle_plan {
        let report = read_release_readiness_evidence_report(path)?;
        validate_windows_service_lifecycle_plan_report(&report).map_err(|error| {
            release_readiness_manifest_error(
                RELEASE_READINESS_WINDOWS_SERVICE_LIFECYCLE_PLAN_LABEL,
                error,
            )
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_WINDOWS_SERVICE_LIFECYCLE_PLAN_LABEL,
            privacy_boundary: "blocked_release_evidence_manifest",
            detail: "release.windows_service_lifecycle_plan.v1 dry-run operator plan passed schema and boundary checks",
        });
    }
    if let Some(path) = &args.hardware_fault_evidence {
        let report = read_release_readiness_evidence_report(path)?;
        validate_hardware_fault_drill_evidence_report(&report).map_err(|error| {
            release_readiness_manifest_error(RELEASE_READINESS_HARDWARE_FAULT_DRILLS_LABEL, error)
        })?;
        provided.push(ReleaseReadinessProvidedEvidence {
            label: RELEASE_READINESS_HARDWARE_FAULT_DRILLS_LABEL,
            privacy_boundary: "redacted_release_hardware_fault_drills",
            detail: "release.hardware_fault_drills.v1 actual release-platform drill evidence passed schema and redaction checks",
        });
    }
    Ok(provided)
}

fn release_readiness_model_distribution_is_packaged(
    args: &ReleaseReadinessEvidenceArgs,
    validation: &ModelManifestValidation,
) -> Result<bool> {
    let embedding_model_sha256 = validation
        .models
        .iter()
        .filter(|model| model.model_type == "embedding")
        .map(|model| model.sha256.as_str())
        .collect::<BTreeSet<_>>();
    if embedding_model_sha256.is_empty() {
        return Ok(false);
    }

    for path in [&args.macos_package_manifest, &args.windows_package_manifest]
        .into_iter()
        .flatten()
    {
        let report = read_release_readiness_evidence_report(path)?;
        if release_package_runtime_payload_contains_embedding_model(
            &report,
            &embedding_model_sha256,
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn release_package_runtime_payload_contains_embedding_model(
    report: &str,
    embedding_model_sha256: &BTreeSet<&str>,
) -> Result<bool> {
    const CONTEXT: &str = "embedding model distribution package payload";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "embedding model distribution blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("embedding model distribution blocked: invalid JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        CliError::user("embedding model distribution blocked: expected JSON object")
    })?;
    let Some(payload_value) = object.get("runtime_payload") else {
        return Ok(false);
    };
    let payload = payload_value
        .as_object()
        .ok_or_else(|| release_evidence_invalid(CONTEXT, "runtime_payload"))?;
    require_release_evidence_string(
        payload,
        "schema_version",
        "release.runtime_package_payload.v1",
        CONTEXT,
    )?;
    require_release_evidence_string(payload, "runtime_distribution_mode", "bundled", CONTEXT)?;
    require_release_evidence_bool(payload, "runtime_package_binaries_included", true, CONTEXT)?;
    let components = require_release_evidence_array(payload, "components", CONTEXT)?;
    for component in components {
        let component = component
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "components"))?;
        let kind = require_release_evidence_string_value(component, "kind", CONTEXT)?;
        let file = require_release_evidence_string_value(component, "file", CONTEXT)?;
        if !is_release_evidence_basename(file) {
            return Err(release_evidence_invalid(CONTEXT, "file"));
        }
        let sha256 = require_release_evidence_sha256_value(component, "sha256", CONTEXT)?;
        if kind == "embedding-model" && embedding_model_sha256.contains(sha256) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn release_readiness_ocr_runtime_distribution_is_packaged(
    args: &ReleaseReadinessEvidenceArgs,
    validation: &OcrRuntimeManifestValidation,
) -> Result<bool> {
    let engine_sha256 = validation
        .components
        .iter()
        .filter(|component| component.kind == "ocr-engine")
        .map(|component| component.sha256.as_str())
        .collect::<BTreeSet<_>>();
    let renderer_sha256 = validation
        .components
        .iter()
        .filter(|component| component.kind == "pdf-renderer")
        .map(|component| component.sha256.as_str())
        .collect::<BTreeSet<_>>();
    let language_pack_sha256 = validation
        .components
        .iter()
        .filter(|component| component.kind == "ocr-language-pack")
        .map(|component| component.sha256.as_str())
        .chain(
            validation
                .languages
                .iter()
                .map(|language| language.sha256.as_str()),
        )
        .collect::<BTreeSet<_>>();

    if engine_sha256.is_empty() || renderer_sha256.is_empty() || language_pack_sha256.is_empty() {
        return Ok(false);
    }

    for path in [&args.macos_package_manifest, &args.windows_package_manifest]
        .into_iter()
        .flatten()
    {
        let report = read_release_readiness_evidence_report(path)?;
        if release_package_runtime_payload_contains_ocr_runtime(
            &report,
            &engine_sha256,
            &renderer_sha256,
            &language_pack_sha256,
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn release_package_runtime_payload_contains_ocr_runtime(
    report: &str,
    engine_sha256: &BTreeSet<&str>,
    renderer_sha256: &BTreeSet<&str>,
    language_pack_sha256: &BTreeSet<&str>,
) -> Result<bool> {
    const CONTEXT: &str = "OCR runtime distribution package payload";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "OCR runtime distribution blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("OCR runtime distribution blocked: invalid JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| CliError::user("OCR runtime distribution blocked: expected JSON object"))?;
    let Some(payload_value) = object.get("runtime_payload") else {
        return Ok(false);
    };
    let payload = payload_value
        .as_object()
        .ok_or_else(|| release_evidence_invalid(CONTEXT, "runtime_payload"))?;
    require_release_evidence_string(
        payload,
        "schema_version",
        "release.runtime_package_payload.v1",
        CONTEXT,
    )?;
    require_release_evidence_string(payload, "runtime_distribution_mode", "bundled", CONTEXT)?;
    require_release_evidence_bool(payload, "runtime_package_binaries_included", true, CONTEXT)?;
    let components = require_release_evidence_array(payload, "components", CONTEXT)?;

    let mut has_engine = false;
    let mut has_renderer = false;
    let mut has_language_pack = false;
    for component in components {
        let component = component
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "components"))?;
        let kind = require_release_evidence_string_value(component, "kind", CONTEXT)?;
        let file = require_release_evidence_string_value(component, "file", CONTEXT)?;
        if !is_release_evidence_basename(file) {
            return Err(release_evidence_invalid(CONTEXT, "file"));
        }
        let sha256 = require_release_evidence_sha256_value(component, "sha256", CONTEXT)?;
        match kind {
            "ocr-engine" if engine_sha256.contains(sha256) => has_engine = true,
            "pdf-renderer" if renderer_sha256.contains(sha256) => has_renderer = true,
            "ocr-language-pack" if language_pack_sha256.contains(sha256) => {
                has_language_pack = true
            }
            _ => {}
        }
    }

    Ok(has_engine && has_renderer && has_language_pack)
}

const SIGNING_AUTOMATION_EVIDENCE_SPEC: ReleaseAutomationEvidenceSpec =
    ReleaseAutomationEvidenceSpec {
        label: RELEASE_READINESS_SIGNING_AUTOMATION_LABEL,
        schema_version: "release.signing_evidence.v1",
        status_key: "signing_status",
        evidence_boundary: "dry_run_no_signing_material",
        digest_key: "artifact_manifest_sha256",
        require_planned_actions: false,
    };

const NOTARIZATION_AUTOMATION_EVIDENCE_SPEC: ReleaseAutomationEvidenceSpec =
    ReleaseAutomationEvidenceSpec {
        label: RELEASE_READINESS_NOTARIZATION_AUTOMATION_LABEL,
        schema_version: "release.notarization_evidence.v1",
        status_key: "notarization_status",
        evidence_boundary: "dry_run_no_notarization_credentials",
        digest_key: "macos_package_manifest_sha256",
        require_planned_actions: false,
    };

const MACOS_INSTALLER_AUTOMATION_EVIDENCE_SPEC: ReleaseAutomationEvidenceSpec =
    ReleaseAutomationEvidenceSpec {
        label: RELEASE_READINESS_MACOS_INSTALLER_AUTOMATION_LABEL,
        schema_version: "release.macos_installer_evidence.v1",
        status_key: "installer_lifecycle_status",
        evidence_boundary: "dry_run_no_macos_installer_execution",
        digest_key: "macos_package_manifest_sha256",
        require_planned_actions: true,
    };

const WINDOWS_INSTALLER_AUTOMATION_EVIDENCE_SPEC: ReleaseAutomationEvidenceSpec =
    ReleaseAutomationEvidenceSpec {
        label: RELEASE_READINESS_WINDOWS_INSTALLER_AUTOMATION_LABEL,
        schema_version: "release.windows_installer_evidence.v1",
        status_key: "installer_lifecycle_status",
        evidence_boundary: "dry_run_no_windows_installer_execution",
        digest_key: "windows_package_manifest_sha256",
        require_planned_actions: true,
    };

const WINDOWS_SERVICE_AUTOMATION_EVIDENCE_SPEC: ReleaseAutomationEvidenceSpec =
    ReleaseAutomationEvidenceSpec {
        label: RELEASE_READINESS_WINDOWS_SERVICE_AUTOMATION_LABEL,
        schema_version: "release.windows_service_evidence.v1",
        status_key: "service_lifecycle_status",
        evidence_boundary: "dry_run_no_windows_service_registration",
        digest_key: "windows_package_manifest_sha256",
        require_planned_actions: true,
    };

fn validate_release_automation_evidence(
    path: &Path,
    spec: &ReleaseAutomationEvidenceSpec,
) -> Result<()> {
    let report = read_release_readiness_evidence_report(path)?;
    validate_release_automation_evidence_report(&report, spec)
        .map_err(|error| release_readiness_manifest_error(spec.label, error))
}

fn validate_release_automation_evidence_report(
    report: &str,
    spec: &ReleaseAutomationEvidenceSpec,
) -> Result<()> {
    if release_readiness_diagnostics_report_contains_private_marker(report) {
        return Err(CliError::user(
            "release automation evidence blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("release automation evidence blocked: invalid JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        CliError::user("release automation evidence blocked: expected JSON object")
    })?;

    validate_release_automation_evidence_allowed_keys(object, spec)?;
    require_release_json_string(object, "schema_version", spec.schema_version)?;
    require_release_json_string(object, spec.status_key, "blocked")?;
    require_release_json_string(object, "evidence_boundary", spec.evidence_boundary)?;
    require_release_json_sha256(object, spec.digest_key)?;
    require_release_json_non_empty_array(object, "required_evidence")?;
    require_release_json_non_empty_array(object, "blocked_release_steps")?;
    if spec.require_planned_actions {
        require_release_blocked_planned_actions(object)?;
    }

    Ok(())
}

fn validate_release_automation_evidence_allowed_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    spec: &ReleaseAutomationEvidenceSpec,
) -> Result<()> {
    const CONTEXT: &str = "release automation evidence";
    let allowed_keys = match spec.schema_version {
        "release.signing_evidence.v1" => &[
            "schema_version",
            "version",
            "signing_status",
            "evidence_boundary",
            "artifact_manifest_sha256",
            "artifacts",
            "required_evidence",
            "blocked_release_steps",
            "prohibited_public_material",
            "notes",
        ][..],
        "release.notarization_evidence.v1" => &[
            "schema_version",
            "version",
            "notarization_status",
            "evidence_boundary",
            "macos_package_manifest_sha256",
            "artifacts",
            "required_evidence",
            "blocked_release_steps",
            "prohibited_public_material",
            "notes",
        ][..],
        "release.macos_installer_evidence.v1" => &[
            "schema_version",
            "version",
            "installer_lifecycle_status",
            "evidence_boundary",
            "macos_package_manifest_sha256",
            "installer_tool",
            "installer_supporting_tools",
            "admin_elevation",
            "installation_status",
            "rollback_validation_status",
            "launch_agent_validation_status",
            "installer_artifacts",
            "planned_actions",
            "required_evidence",
            "blocked_release_steps",
            "prohibited_public_material",
            "notes",
        ][..],
        "release.windows_installer_evidence.v1" => &[
            "schema_version",
            "version",
            "installer_lifecycle_status",
            "evidence_boundary",
            "windows_package_manifest_sha256",
            "installer_engine",
            "admin_elevation",
            "installation_status",
            "rollback_validation_status",
            "installer_artifacts",
            "planned_actions",
            "required_evidence",
            "blocked_release_steps",
            "prohibited_public_material",
            "notes",
        ][..],
        "release.windows_service_evidence.v1" => &[
            "schema_version",
            "version",
            "service_lifecycle_status",
            "evidence_boundary",
            "windows_package_manifest_sha256",
            "service_manager",
            "admin_elevation",
            "registration_status",
            "recovery_validation_status",
            "installer_artifacts",
            "planned_actions",
            "required_evidence",
            "blocked_release_steps",
            "prohibited_public_material",
            "notes",
        ][..],
        _ => {
            return Err(CliError::user(
                "release automation evidence blocked: schema_version is invalid",
            ))
        }
    };
    validate_release_evidence_allowed_keys(object, allowed_keys, CONTEXT)?;
    validate_release_automation_object_array_keys(
        object,
        "artifacts",
        &[
            "name",
            "kind",
            "file",
            "artifact_sha256",
            "bytes",
            "signature_status",
            "verification_status",
            "ticket_status",
            "staple_status",
            "gatekeeper_status",
        ],
    )?;
    validate_release_automation_object_array_keys(
        object,
        "installer_artifacts",
        &["kind", "file", "artifact_sha256", "bytes"],
    )?;
    validate_release_automation_object_array_keys(
        object,
        "planned_actions",
        &["action", "action_status", "required_evidence"],
    )
}

fn validate_release_automation_object_array_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    allowed_keys: &[&str],
) -> Result<()> {
    const CONTEXT: &str = "release automation evidence";
    let Some(values) = object.get(key) else {
        return Ok(());
    };
    let values = values
        .as_array()
        .ok_or_else(|| CliError::user(format!("{CONTEXT} blocked: {key} is invalid")))?;
    for value in values {
        let value = value
            .as_object()
            .ok_or_else(|| CliError::user(format!("{CONTEXT} blocked: {key} is invalid")))?;
        validate_release_evidence_allowed_keys(value, allowed_keys, CONTEXT)?;
    }
    Ok(())
}

fn validate_release_publication_evidence_report(report: &str) -> Result<()> {
    const CONTEXT: &str = "release publication evidence";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "release publication evidence blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("release publication evidence blocked: invalid JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        CliError::user("release publication evidence blocked: expected JSON object")
    })?;
    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "version",
            "publication_status",
            "evidence_boundary",
            "artifact_manifest_sha256",
            "artifacts",
            "required_evidence",
            "blocked_release_steps",
            "prohibited_public_material",
            "notes",
        ],
        CONTEXT,
    )?;

    require_release_evidence_string(
        object,
        "schema_version",
        "release.publication_evidence.v1",
        CONTEXT,
    )?;
    let version = require_release_evidence_non_empty_string(object, "version", CONTEXT)?;
    validate_release_evidence_version(version, CONTEXT)?;
    require_release_evidence_string(object, "publication_status", "blocked", CONTEXT)?;
    require_release_evidence_string(
        object,
        "evidence_boundary",
        "dry_run_no_release_publication",
        CONTEXT,
    )?;
    require_release_evidence_sha256(object, "artifact_manifest_sha256", CONTEXT)?;
    for expected in [
        "human_release_approval",
        "github_actions_release_token",
        "github_release_upload_evidence",
    ] {
        require_release_evidence_array_contains_string(
            object,
            "required_evidence",
            expected,
            CONTEXT,
        )?;
    }
    for expected in ["github_release_create", "github_release_upload"] {
        require_release_evidence_array_contains_string(
            object,
            "blocked_release_steps",
            expected,
            CONTEXT,
        )?;
    }
    require_release_evidence_exact_string_array(
        object,
        "prohibited_public_material",
        &[
            "github_token",
            "release_pat",
            "local_paths",
            "raw_resume_data",
            "diagnostic_packages",
            "model_caches",
        ],
        CONTEXT,
    )?;
    require_release_evidence_non_empty_string(object, "notes", CONTEXT)?;
    validate_release_publication_artifacts(object)?;
    Ok(())
}

fn validate_release_publication_artifacts(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    const CONTEXT: &str = "release publication evidence";
    let artifacts = require_release_evidence_array(object, "artifacts", CONTEXT)?;
    let mut seen = BTreeSet::new();
    for artifact in artifacts {
        let artifact = artifact
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "artifacts"))?;
        validate_release_evidence_allowed_keys(
            artifact,
            &["name", "file", "artifact_sha256", "bytes", "upload_status"],
            CONTEXT,
        )?;
        let name = require_release_evidence_non_empty_string(artifact, "name", CONTEXT)?;
        if !matches!(name, "resume-cli" | "resume-daemon" | "resume-benchmark")
            || !seen.insert(name.to_string())
        {
            return Err(release_evidence_invalid(CONTEXT, "artifacts"));
        }
        let file = require_release_evidence_non_empty_string(artifact, "file", CONTEXT)?;
        if !is_release_evidence_basename(file) {
            return Err(release_evidence_invalid(CONTEXT, "file"));
        }
        require_release_evidence_sha256(artifact, "artifact_sha256", CONTEXT)?;
        require_release_evidence_positive_u64(artifact, "bytes", CONTEXT)?;
        require_release_evidence_string(artifact, "upload_status", "blocked", CONTEXT)?;
    }
    for required in ["resume-cli", "resume-daemon", "resume-benchmark"] {
        if !seen.contains(required) {
            return Err(release_evidence_invalid(CONTEXT, "artifacts"));
        }
    }
    Ok(())
}

fn validate_github_release_publication_gate_report(
    report: &str,
) -> Result<(&'static str, &'static str)> {
    const CONTEXT: &str = "GitHub Release publication gate";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "GitHub Release publication gate blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("GitHub Release publication gate blocked: invalid JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        CliError::user("GitHub Release publication gate blocked: expected JSON object")
    })?;
    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "version",
            "repo",
            "execution_mode",
            "evidence_boundary",
            "publication_status",
            "approval_gate",
            "secret_interface",
            "artifact_manifest_sha256",
            "publication_evidence_sha256",
            "planned_steps",
            "artifacts",
            "prohibited_public_material",
            "notes",
        ],
        CONTEXT,
    )?;

    require_release_evidence_string(
        object,
        "schema_version",
        "release.github_publication_gate.v1",
        CONTEXT,
    )?;
    let version = require_release_evidence_non_empty_string(object, "version", CONTEXT)?;
    validate_release_evidence_version(version, CONTEXT)?;
    let repo = require_release_evidence_non_empty_string(object, "repo", CONTEXT)?;
    if !is_github_repo_slug(repo) {
        return Err(release_evidence_invalid(CONTEXT, "repo"));
    }
    let execution_mode =
        require_release_evidence_non_empty_string(object, "execution_mode", CONTEXT)?;
    let (publication_status, artifact_publish_status, privacy_boundary, detail) =
        match execution_mode {
        "dry_run" => (
            "blocked",
            "blocked",
            "blocked_release_evidence_manifest",
            "release.github_publication_gate.v1 fail-closed dry-run gate passed schema and publication boundary checks",
        ),
        "execute" => (
            "published_verified",
            "uploaded_verified",
            "verified_release_evidence_manifest",
            "release.github_publication_gate.v1 verified execute gate passed upload and download evidence checks",
        ),
        _ => return Err(release_evidence_invalid(CONTEXT, "execution_mode")),
    };
    require_release_evidence_string(object, "evidence_boundary", privacy_boundary, CONTEXT)?;
    require_release_evidence_string(object, "publication_status", publication_status, CONTEXT)?;
    require_release_evidence_string(
        object,
        "approval_gate",
        "human_release_approval_required",
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "secret_interface",
        "GITHUB_TOKEN_or_GH_TOKEN_required_for_execute",
        CONTEXT,
    )?;
    require_release_evidence_sha256(object, "artifact_manifest_sha256", CONTEXT)?;
    require_release_evidence_sha256(object, "publication_evidence_sha256", CONTEXT)?;
    require_release_evidence_exact_string_array(
        object,
        "planned_steps",
        &[
            "validate_release_artifact_manifest",
            "validate_publication_evidence_manifest",
            "gh_release_create",
            "gh_release_upload",
            "gh_release_download_verify",
        ],
        CONTEXT,
    )?;
    require_release_evidence_exact_string_array(
        object,
        "prohibited_public_material",
        &[
            "github_token",
            "release_pat",
            "local_paths",
            "raw_resume_data",
            "diagnostic_packages",
            "model_caches",
        ],
        CONTEXT,
    )?;
    let notes = require_release_evidence_non_empty_string(object, "notes", CONTEXT)?;
    if execution_mode == "execute" {
        let normalized_notes = notes.to_ascii_lowercase();
        if normalized_notes.contains("synthetic")
            || normalized_notes.contains("fixture")
            || normalized_notes.contains("no real")
        {
            return Err(release_evidence_invalid(CONTEXT, "notes"));
        }
    }
    validate_github_release_publication_gate_artifacts(object, artifact_publish_status)?;
    Ok((privacy_boundary, detail))
}

fn validate_github_release_publication_gate_artifacts(
    object: &serde_json::Map<String, serde_json::Value>,
    expected_publish_status: &str,
) -> Result<()> {
    const CONTEXT: &str = "GitHub Release publication gate";
    let artifacts = require_release_evidence_array(object, "artifacts", CONTEXT)?;
    let mut seen = BTreeSet::new();
    for artifact in artifacts {
        let artifact = artifact
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "artifacts"))?;
        validate_release_evidence_allowed_keys(
            artifact,
            &["name", "file", "artifact_sha256", "bytes", "publish_status"],
            CONTEXT,
        )?;
        let name = require_release_evidence_non_empty_string(artifact, "name", CONTEXT)?;
        if !matches!(name, "resume-cli" | "resume-daemon" | "resume-benchmark")
            || !seen.insert(name.to_string())
        {
            return Err(release_evidence_invalid(CONTEXT, "artifacts"));
        }
        let file = require_release_evidence_non_empty_string(artifact, "file", CONTEXT)?;
        if !is_release_evidence_basename(file) {
            return Err(release_evidence_invalid(CONTEXT, "file"));
        }
        require_release_evidence_sha256(artifact, "artifact_sha256", CONTEXT)?;
        require_release_evidence_positive_u64(artifact, "bytes", CONTEXT)?;
        require_release_evidence_string(
            artifact,
            "publish_status",
            expected_publish_status,
            CONTEXT,
        )?;
    }
    for required in ["resume-cli", "resume-daemon", "resume-benchmark"] {
        if !seen.contains(required) {
            return Err(release_evidence_invalid(CONTEXT, "artifacts"));
        }
    }
    Ok(())
}

fn is_github_repo_slug(value: &str) -> bool {
    let parts = value.split('/').collect::<Vec<_>>();
    parts.len() == 2
        && parts.iter().all(|part| {
            !part.is_empty()
            && part.bytes().all(
                |byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'-'),
            )
        })
}

struct CurrentStageEvidenceDigests {
    input_digests: BTreeMap<String, String>,
    redacted_outputs: BTreeMap<String, String>,
}

impl CurrentStageEvidenceDigests {
    fn input_digest(&self, key: &str) -> Option<&str> {
        self.input_digests.get(key).map(String::as_str)
    }

    fn output_digest(&self, file: &str) -> Option<&str> {
        self.redacted_outputs.get(file).map(String::as_str)
    }
}

fn validate_current_stage_evidence_manifest(report: &str) -> Result<CurrentStageEvidenceDigests> {
    const CONTEXT: &str = "current-stage validation evidence";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
        || report.contains("PRIVATE-")
        || report.contains("private fake query")
    {
        return Err(CliError::user(
            "current-stage validation evidence blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("current-stage validation evidence blocked: invalid JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        CliError::user("current-stage validation evidence blocked: expected JSON object")
    })?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "privacy_boundary",
            "current_stage_target",
            "runtime_distribution_mode",
            "runtime_package_binaries_included",
            "performance_optimization_deferred",
            "release_readiness_exit",
            "stable_release_expected_blocked",
            "input_digests",
            "parameters",
            "preflight_probes",
            "corpus_summary_observability",
            "private_query_observability",
            "steps",
            "redacted_outputs",
            "privacy_sentinels",
            "must_not_upload",
        ],
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "schema_version",
        "resume-ir.current-stage-validation-evidence.v2",
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "privacy_boundary",
        "local_only_redacted_evidence_manifest",
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "current_stage_target",
        "reproducible_local_10k_baseline",
        CONTEXT,
    )?;
    validate_current_stage_runtime_distribution(object, CONTEXT)?;
    require_release_evidence_bool(object, "performance_optimization_deferred", true, CONTEXT)?;
    require_release_evidence_u64(object, "release_readiness_exit", 1, CONTEXT)?;
    require_release_evidence_bool(object, "stable_release_expected_blocked", true, CONTEXT)?;

    let input_digests = require_release_evidence_object(object, "input_digests", CONTEXT)?;
    validate_release_evidence_allowed_keys(
        input_digests,
        &[
            "dataset_manifest_sha256",
            "query_set_sha256",
            "model_manifest_sha256",
            "ocr_runtime_manifest_sha256",
        ],
        CONTEXT,
    )?;
    let dataset_manifest_sha256 =
        require_release_evidence_sha256_value(input_digests, "dataset_manifest_sha256", CONTEXT)?;
    let query_set_sha256 =
        require_release_evidence_sha256_value(input_digests, "query_set_sha256", CONTEXT)?;
    let model_manifest_sha256 =
        require_release_evidence_sha256_value(input_digests, "model_manifest_sha256", CONTEXT)?;
    let ocr_runtime_manifest_sha256 = require_release_evidence_sha256_value(
        input_digests,
        "ocr_runtime_manifest_sha256",
        CONTEXT,
    )?;
    let expected_input_digests = BTreeMap::from([
        (
            "dataset_manifest_sha256".to_string(),
            dataset_manifest_sha256.to_string(),
        ),
        ("query_set_sha256".to_string(), query_set_sha256.to_string()),
        (
            "model_manifest_sha256".to_string(),
            model_manifest_sha256.to_string(),
        ),
        (
            "ocr_runtime_manifest_sha256".to_string(),
            ocr_runtime_manifest_sha256.to_string(),
        ),
    ]);

    let parameters = require_release_evidence_object(object, "parameters", CONTEXT)?;
    validate_release_evidence_allowed_keys(
        parameters,
        &[
            "max_files",
            "max_queries",
            "top_k",
            "private_query_timeout_ms",
            "embedding_dimension",
            "embedding_runtime_bin_dir_configured",
            "reuse_imported_corpus",
            "ocr_worker_ticks",
            "ocr_jobs_per_tick",
        ],
        CONTEXT,
    )?;
    require_release_evidence_min_u64(
        parameters,
        "max_files",
        CURRENT_STAGE_D10K_DOCUMENT_MIN,
        CONTEXT,
    )?;
    require_release_evidence_min_u64(
        parameters,
        "max_queries",
        CURRENT_STAGE_D10K_QUERY_MIN,
        CONTEXT,
    )?;
    for key in [
        "top_k",
        "private_query_timeout_ms",
        "embedding_dimension",
        "ocr_worker_ticks",
        "ocr_jobs_per_tick",
    ] {
        require_release_evidence_positive_u64(parameters, key, CONTEXT)?;
    }
    require_release_evidence_bool_value(
        parameters,
        "embedding_runtime_bin_dir_configured",
        CONTEXT,
    )?;
    require_release_evidence_bool_value(parameters, "reuse_imported_corpus", CONTEXT)?;

    let preflight_probes = require_release_evidence_object(object, "preflight_probes", CONTEXT)?;
    validate_release_evidence_allowed_keys(
        preflight_probes,
        &["ocr_runtime_probe", "embedding_protocol"],
        CONTEXT,
    )?;
    require_release_evidence_string(preflight_probes, "ocr_runtime_probe", "passed", CONTEXT)?;
    require_release_evidence_string(preflight_probes, "embedding_protocol", "passed", CONTEXT)?;

    let observability = object
        .get("corpus_summary_observability")
        .ok_or_else(|| release_evidence_invalid(CONTEXT, "corpus_summary_observability"))?;
    validate_current_stage_aggregate_observability(observability, CONTEXT)?;
    let query_observability = object
        .get("private_query_observability")
        .ok_or_else(|| release_evidence_invalid(CONTEXT, "private_query_observability"))?;
    validate_current_stage_private_query_observability(query_observability, CONTEXT)?;

    let steps = require_release_evidence_array(object, "steps", CONTEXT)?;
    require_release_evidence_exact_steps(
        steps,
        &[
            ("ocr_preflight", "success"),
            ("ocr_manifest_draft", "success"),
            ("ocr_manifest_validate", "success"),
            ("model_manifest_draft", "success"),
            ("model_manifest_validate", "success"),
            ("model_preflight", "success"),
            ("dataset_manifest", "success"),
            ("import_private_corpus", "success"),
            ("ocr_search_publication_bounded_loop", "success"),
            ("corpus_summary", "success"),
            ("query_set_prepare", "success"),
            ("private_query_baseline", "success"),
            ("baseline_shape_gate", "success"),
            ("private_ocr_throughput_baseline", "success"),
            ("ocr_throughput_baseline_gate", "success"),
            ("redacted_diagnostics", "success"),
            ("doctor", "success"),
            ("fault_simulation_smoke", "success"),
            ("fault_simulation_suite", "success"),
            ("release_readiness_intake", "expected_blocked"),
        ],
        CONTEXT,
    )?;
    require_release_evidence_step_exit_code(
        steps,
        "release_readiness_intake",
        "expected_blocked",
        1,
        CONTEXT,
    )?;

    let redacted_outputs = require_release_evidence_array(object, "redacted_outputs", CONTEXT)?;
    let required_output_files = [
        "dataset-manifest.local.json",
        "dataset-manifest.stdout.txt",
        "ocr-runtime-manifest.local.json",
        "ocr-preflight.json",
        "ocr-draft-manifest.stdout.txt",
        "ocr-validate-manifest.stdout.txt",
        "model-manifest.local.json",
        "model-draft-manifest.stdout.txt",
        "model-validate-manifest.stdout.txt",
        "model-preflight.json",
        "import.stdout.txt",
        "ocr-search-publication.stdout.txt",
        "benchmark-corpus-summary.local.json",
        "private-query-set.local.jsonl",
        "query-set-prepare.stdout.txt",
        "private-benchmark-local.json",
        "private-benchmark-gate.stdout.txt",
        "private-ocr-throughput.json",
        "ocr-throughput-gate.stdout.txt",
        "redacted-diagnostics.json",
        "doctor.out",
        "fault-simulation-storage-low.json",
        "fault-simulation-suite-local-safe.json",
        "release-readiness.json",
        "release-readiness.stderr.txt",
    ];
    let mut output_digests = BTreeMap::new();
    for output in redacted_outputs {
        let output = output
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "redacted_outputs"))?;
        validate_release_evidence_allowed_keys(output, &["file", "sha256"], CONTEXT)?;
        let file = require_release_evidence_string_value(output, "file", CONTEXT)?;
        if !is_release_evidence_basename(file) {
            return Err(release_evidence_invalid(CONTEXT, "file"));
        }
        if !required_output_files.contains(&file) {
            return Err(release_evidence_invalid(CONTEXT, "redacted_outputs"));
        }
        let sha256 = require_release_evidence_sha256_value(output, "sha256", CONTEXT)?;
        if output_digests
            .insert(file.to_string(), sha256.to_string())
            .is_some()
        {
            return Err(release_evidence_invalid(CONTEXT, "redacted_outputs"));
        }
    }
    if output_digests.len() != required_output_files.len() {
        return Err(release_evidence_invalid(CONTEXT, "redacted_outputs"));
    }
    for required_file in required_output_files {
        if !output_digests.contains_key(required_file) {
            return Err(release_evidence_invalid(CONTEXT, "redacted_outputs"));
        }
    }
    require_release_evidence_output_digest(
        &output_digests,
        "dataset-manifest.local.json",
        dataset_manifest_sha256,
        CONTEXT,
    )?;
    require_release_evidence_output_digest(
        &output_digests,
        "private-query-set.local.jsonl",
        query_set_sha256,
        CONTEXT,
    )?;
    require_release_evidence_output_digest(
        &output_digests,
        "model-manifest.local.json",
        model_manifest_sha256,
        CONTEXT,
    )?;
    require_release_evidence_output_digest(
        &output_digests,
        "ocr-runtime-manifest.local.json",
        ocr_runtime_manifest_sha256,
        CONTEXT,
    )?;

    let privacy_sentinels = require_release_evidence_object(object, "privacy_sentinels", CONTEXT)?;
    validate_release_evidence_allowed_keys(
        privacy_sentinels,
        &[
            "local_paths_included",
            "raw_resume_text_included",
            "raw_query_text_included",
            "model_bytes_included",
            "runtime_binaries_included",
            "report_bodies_included",
        ],
        CONTEXT,
    )?;
    for key in [
        "local_paths_included",
        "raw_resume_text_included",
        "raw_query_text_included",
        "model_bytes_included",
        "runtime_binaries_included",
        "report_bodies_included",
    ] {
        require_release_evidence_bool(privacy_sentinels, key, false, CONTEXT)?;
    }

    for item in [
        "raw resumes",
        "query set",
        "local manifests",
        "benchmark reports",
        "diagnostics",
        "indexes",
        "SQLite databases",
        "model caches",
        "runtime binaries",
    ] {
        require_release_evidence_array_contains_string(object, "must_not_upload", item, CONTEXT)?;
    }

    Ok(CurrentStageEvidenceDigests {
        input_digests: expected_input_digests,
        redacted_outputs: output_digests,
    })
}

fn validate_current_stage_runtime_distribution(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &'static str,
) -> Result<()> {
    let mode = require_release_evidence_string_value(object, "runtime_distribution_mode", context)?;
    let package_binaries_included =
        require_release_evidence_bool_value(object, "runtime_package_binaries_included", context)?;
    let expected_package_binaries = match mode {
        "bundled" => true,
        "external" => false,
        _ => {
            return Err(release_evidence_invalid(
                context,
                "runtime_distribution_mode",
            ))
        }
    };
    if package_binaries_included != expected_package_binaries {
        return Err(release_evidence_invalid(
            context,
            "runtime_package_binaries_included",
        ));
    }
    Ok(())
}

fn validate_current_stage_evidence_bundle_digests(
    args: &ReleaseReadinessEvidenceArgs,
    digests: &CurrentStageEvidenceDigests,
) -> Result<()> {
    if let Some(path) = &args.benchmark_report {
        require_current_stage_bundle_digest(
            path,
            digests.output_digest("private-benchmark-local.json"),
            "private-benchmark-local.json",
        )?;
    }
    if let Some(path) = &args.ocr_throughput_report {
        require_current_stage_bundle_digest(
            path,
            digests.output_digest("private-ocr-throughput.json"),
            "private-ocr-throughput.json",
        )?;
    }
    if let Some(path) = &args.diagnostics_report {
        require_current_stage_bundle_digest(
            path,
            digests.output_digest("redacted-diagnostics.json"),
            "redacted-diagnostics.json",
        )?;
    }
    if let Some(path) = &args.model_manifest {
        require_current_stage_bundle_digest(
            path,
            digests.input_digest("model_manifest_sha256"),
            "model_manifest_sha256",
        )?;
    }
    if let Some(path) = &args.ocr_runtime_manifest {
        require_current_stage_bundle_digest(
            path,
            digests.input_digest("ocr_runtime_manifest_sha256"),
            "ocr_runtime_manifest_sha256",
        )?;
    }

    Ok(())
}

fn validate_current_stage_blocked_summary_bundle_digests(
    args: &ReleaseReadinessEvidenceArgs,
    digests: &CurrentStageEvidenceDigests,
) -> Result<()> {
    if let Some(path) = &args.diagnostics_report {
        require_current_stage_bundle_digest(
            path,
            digests.output_digest("redacted-diagnostics.json"),
            "redacted-diagnostics.json",
        )?;
    }
    if let Some(path) = &args.model_manifest {
        require_current_stage_bundle_digest(
            path,
            digests.input_digest("model_manifest_sha256"),
            "model_manifest_sha256",
        )?;
    }
    if let Some(path) = &args.ocr_runtime_manifest {
        require_current_stage_bundle_digest(
            path,
            digests.input_digest("ocr_runtime_manifest_sha256"),
            "ocr_runtime_manifest_sha256",
        )?;
    }

    Ok(())
}

fn require_current_stage_bundle_digest(
    path: &Path,
    expected_sha256: Option<&str>,
    artifact: &'static str,
) -> Result<()> {
    let expected_sha256 = expected_sha256.ok_or_else(|| {
        CliError::user(format!(
            "current-stage evidence bundle blocked: {artifact} digest is missing"
        ))
    })?;
    let actual_sha256 = file_sha256_hex(path).map_err(|_| {
        CliError::user(format!(
            "current-stage evidence bundle blocked: {artifact} digest could not be verified"
        ))
    })?;
    if actual_sha256 == expected_sha256 {
        Ok(())
    } else {
        Err(CliError::user(format!(
            "current-stage evidence bundle blocked: {artifact} digest mismatch"
        )))
    }
}

fn validate_current_stage_blocked_summary_manifest(
    report: &str,
) -> Result<CurrentStageEvidenceDigests> {
    const CONTEXT: &str = "current-stage blocked summary";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
        || report.contains("PRIVATE-")
        || report.contains("private fake query")
    {
        return Err(CliError::user(
            "current-stage blocked summary blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("current-stage blocked summary blocked: invalid JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        CliError::user("current-stage blocked summary blocked: expected JSON object")
    })?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "privacy_boundary",
            "validation_profile",
            "current_stage_target",
            "runtime_distribution_mode",
            "runtime_package_binaries_included",
            "private_corpus_read",
            "full_baseline_satisfied",
            "release_readiness_evidence",
            "performance_optimization_deferred",
            "blocked_step",
            "blocked_category",
            "blocked_reason",
            "blocked_exit",
            "input_digests",
            "parameters",
            "preflight_probes",
            "corpus_summary_observability",
            "steps",
            "redacted_outputs",
            "privacy_sentinels",
            "not_completed",
            "must_not_upload",
        ],
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "schema_version",
        "resume-ir.current-stage-blocked-summary.v2",
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "privacy_boundary",
        "local_only_redacted_blocked_summary",
        CONTEXT,
    )?;
    require_release_evidence_string(object, "validation_profile", "full", CONTEXT)?;
    require_release_evidence_string(
        object,
        "current_stage_target",
        "reproducible_local_10k_baseline",
        CONTEXT,
    )?;
    validate_current_stage_runtime_distribution(object, CONTEXT)?;
    require_release_evidence_bool_value(object, "private_corpus_read", CONTEXT)?;
    require_release_evidence_bool_value(object, "full_baseline_satisfied", CONTEXT)?;
    require_release_evidence_bool(object, "release_readiness_evidence", false, CONTEXT)?;
    require_release_evidence_bool(object, "performance_optimization_deferred", true, CONTEXT)?;
    let blocked_step = require_release_evidence_non_empty_string(object, "blocked_step", CONTEXT)?;
    let blocked_category =
        require_release_evidence_non_empty_string(object, "blocked_category", CONTEXT)?;
    if ![
        "ocr",
        "embedding",
        "import/parser",
        "query-set",
        "benchmark",
        "diagnostics",
        "fault-injection",
        "release-readiness",
    ]
    .contains(&blocked_category)
    {
        return Err(release_evidence_invalid(CONTEXT, "blocked_category"));
    }
    require_release_evidence_non_empty_string(object, "blocked_reason", CONTEXT)?;
    let blocked_exit =
        require_release_evidence_positive_u64_value(object, "blocked_exit", CONTEXT)?;

    let input_digests = require_release_evidence_object(object, "input_digests", CONTEXT)?;
    validate_release_evidence_allowed_keys(
        input_digests,
        &[
            "dataset_manifest_sha256",
            "query_set_sha256",
            "model_manifest_sha256",
            "ocr_runtime_manifest_sha256",
        ],
        CONTEXT,
    )?;
    let mut expected_input_digests = BTreeMap::new();
    for key in [
        "dataset_manifest_sha256",
        "query_set_sha256",
        "model_manifest_sha256",
        "ocr_runtime_manifest_sha256",
    ] {
        if let Some(sha256) =
            require_release_evidence_optional_sha256_value(input_digests, key, CONTEXT)?
        {
            expected_input_digests.insert(key.to_string(), sha256.to_string());
        }
    }

    let parameters = require_release_evidence_object(object, "parameters", CONTEXT)?;
    validate_release_evidence_allowed_keys(
        parameters,
        &[
            "max_files",
            "max_queries",
            "top_k",
            "private_query_timeout_ms",
            "embedding_dimension",
            "embedding_runtime_bin_dir_configured",
            "reuse_imported_corpus",
            "ocr_worker_ticks",
            "ocr_jobs_per_tick",
            "query_set_min_queries",
            "baseline_min_documents",
            "baseline_min_queries",
            "ocr_throughput_min_pages",
        ],
        CONTEXT,
    )?;
    require_release_evidence_min_u64(
        parameters,
        "max_files",
        CURRENT_STAGE_D10K_DOCUMENT_MIN,
        CONTEXT,
    )?;
    require_release_evidence_min_u64(
        parameters,
        "max_queries",
        CURRENT_STAGE_D10K_QUERY_MIN,
        CONTEXT,
    )?;
    require_release_evidence_min_u64(
        parameters,
        "baseline_min_documents",
        CURRENT_STAGE_D10K_DOCUMENT_MIN,
        CONTEXT,
    )?;
    require_release_evidence_min_u64(
        parameters,
        "baseline_min_queries",
        CURRENT_STAGE_D10K_QUERY_MIN,
        CONTEXT,
    )?;
    for key in [
        "top_k",
        "private_query_timeout_ms",
        "embedding_dimension",
        "ocr_worker_ticks",
        "ocr_jobs_per_tick",
        "query_set_min_queries",
    ] {
        require_release_evidence_positive_u64(parameters, key, CONTEXT)?;
    }
    require_release_evidence_bool_value(
        parameters,
        "embedding_runtime_bin_dir_configured",
        CONTEXT,
    )?;
    require_release_evidence_bool_value(parameters, "reuse_imported_corpus", CONTEXT)?;
    if parameters.contains_key("ocr_throughput_min_pages") {
        require_release_evidence_positive_u64(parameters, "ocr_throughput_min_pages", CONTEXT)?;
    }

    let preflight_probes = require_release_evidence_object(object, "preflight_probes", CONTEXT)?;
    validate_release_evidence_allowed_keys(
        preflight_probes,
        &["ocr_runtime_probe", "embedding_protocol"],
        CONTEXT,
    )?;
    let ocr_runtime_probe =
        require_release_evidence_string_value(preflight_probes, "ocr_runtime_probe", CONTEXT)?;
    let embedding_protocol =
        require_release_evidence_string_value(preflight_probes, "embedding_protocol", CONTEXT)?;
    for (key, status) in [
        ("ocr_runtime_probe", ocr_runtime_probe),
        ("embedding_protocol", embedding_protocol),
    ] {
        if !["passed", "blocked", "not_run"].contains(&status) {
            return Err(release_evidence_invalid(CONTEXT, key));
        }
    }

    let observability = object
        .get("corpus_summary_observability")
        .ok_or_else(|| release_evidence_invalid(CONTEXT, "corpus_summary_observability"))?;
    validate_current_stage_aggregate_observability(observability, CONTEXT)?;

    let steps = require_release_evidence_array(object, "steps", CONTEXT)?;
    let mut step_statuses = BTreeMap::new();
    let mut saw_blocked_step = false;
    for step in steps {
        let step = step
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "steps"))?;
        validate_release_evidence_allowed_keys(step, &["id", "status", "exit_code"], CONTEXT)?;
        let id = require_release_evidence_non_empty_string(step, "id", CONTEXT)?;
        let status = require_release_evidence_string_value(step, "status", CONTEXT)?;
        if !["success", "blocked", "expected_blocked"].contains(&status) {
            return Err(release_evidence_invalid(CONTEXT, "steps"));
        }
        if step_statuses
            .insert(id.to_string(), status.to_string())
            .is_some()
        {
            return Err(release_evidence_invalid(CONTEXT, "steps"));
        }
        if status == "blocked" {
            let exit_code =
                require_release_evidence_positive_u64_value(step, "exit_code", CONTEXT)?;
            if id == blocked_step && exit_code == blocked_exit {
                saw_blocked_step = true;
            }
        }
    }
    if !saw_blocked_step {
        return Err(release_evidence_invalid(CONTEXT, "steps"));
    }
    validate_current_stage_blocked_preflight_consistency(
        &step_statuses,
        "ocr_preflight",
        ocr_runtime_probe,
        CONTEXT,
    )?;
    validate_current_stage_blocked_preflight_consistency(
        &step_statuses,
        "model_preflight",
        embedding_protocol,
        CONTEXT,
    )?;

    let redacted_outputs = require_release_evidence_array(object, "redacted_outputs", CONTEXT)?;
    let mut seen_outputs = BTreeSet::new();
    let mut output_digests = BTreeMap::new();
    for output in redacted_outputs {
        let output = output
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "redacted_outputs"))?;
        validate_release_evidence_allowed_keys(output, &["file", "sha256"], CONTEXT)?;
        let file = require_release_evidence_string_value(output, "file", CONTEXT)?;
        if !is_release_evidence_basename(file) || !seen_outputs.insert(file.to_string()) {
            return Err(release_evidence_invalid(CONTEXT, "redacted_outputs"));
        }
        if let Some(sha256) =
            require_release_evidence_optional_sha256_value(output, "sha256", CONTEXT)?
        {
            output_digests.insert(file.to_string(), sha256.to_string());
        }
    }
    if output_digests.is_empty() {
        return Err(release_evidence_invalid(CONTEXT, "redacted_outputs"));
    }

    let privacy_sentinels = require_release_evidence_object(object, "privacy_sentinels", CONTEXT)?;
    validate_release_evidence_allowed_keys(
        privacy_sentinels,
        &[
            "local_paths_included",
            "raw_resume_text_included",
            "raw_query_text_included",
            "model_bytes_included",
            "runtime_binaries_included",
            "report_bodies_included",
        ],
        CONTEXT,
    )?;
    for key in [
        "local_paths_included",
        "raw_resume_text_included",
        "raw_query_text_included",
        "model_bytes_included",
        "runtime_binaries_included",
        "report_bodies_included",
    ] {
        require_release_evidence_bool(privacy_sentinels, key, false, CONTEXT)?;
    }
    require_release_evidence_array_contains_string(
        object,
        "not_completed",
        "stable release readiness",
        CONTEXT,
    )?;
    for item in [
        "raw resumes",
        "query set",
        "local manifests",
        "benchmark reports",
        "diagnostics",
        "indexes",
        "SQLite databases",
        "model caches",
        "runtime binaries",
    ] {
        require_release_evidence_array_contains_string(object, "must_not_upload", item, CONTEXT)?;
    }

    Ok(CurrentStageEvidenceDigests {
        input_digests: expected_input_digests,
        redacted_outputs: output_digests,
    })
}

fn validate_current_stage_blocked_preflight_consistency(
    step_statuses: &BTreeMap<String, String>,
    step_id: &str,
    probe_status: &str,
    context: &'static str,
) -> Result<()> {
    match step_statuses.get(step_id).map(String::as_str) {
        Some("success") if probe_status == "passed" => Ok(()),
        Some("blocked") if probe_status == "blocked" => Ok(()),
        None if probe_status == "not_run" => Ok(()),
        _ => Err(release_evidence_invalid(context, "preflight_probes")),
    }
}

fn validate_release_artifact_manifest_report(report: &str) -> Result<()> {
    const CONTEXT: &str = "release artifact manifest";
    if release_readiness_diagnostics_report_contains_private_marker(report) {
        return Err(CliError::user(
            "release artifact manifest blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("release artifact manifest blocked: invalid JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| CliError::user("release artifact manifest blocked: expected JSON object"))?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "version",
            "packaging_status",
            "artifacts",
            "runtime_bundle_manifests",
            "blocked_release_steps",
            "notes",
        ],
        CONTEXT,
    )?;
    require_release_evidence_string(object, "schema_version", "release.artifacts.v1", CONTEXT)?;
    let version = require_release_evidence_string_value(object, "version", CONTEXT)?;
    validate_release_evidence_version(version, CONTEXT)?;
    require_release_evidence_string(object, "packaging_status", "blocked", CONTEXT)?;
    let artifacts = require_release_evidence_array(object, "artifacts", CONTEXT)?;
    let required_names = ["resume-cli", "resume-daemon", "resume-benchmark"];
    let mut seen_names = BTreeSet::new();

    for artifact in artifacts {
        let artifact = artifact
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "artifacts"))?;
        validate_release_evidence_allowed_keys(
            artifact,
            &["name", "file", "sha256", "bytes"],
            CONTEXT,
        )?;
        let name = require_release_evidence_string_value(artifact, "name", CONTEXT)?;
        if !required_names.contains(&name) {
            return Err(release_evidence_invalid(CONTEXT, "artifacts"));
        }
        let file = require_release_evidence_string_value(artifact, "file", CONTEXT)?;
        if !is_release_evidence_basename(file) {
            return Err(release_evidence_invalid(CONTEXT, "file"));
        }
        require_release_evidence_sha256(artifact, "sha256", CONTEXT)?;
        require_release_evidence_positive_u64(artifact, "bytes", CONTEXT)?;
        seen_names.insert(name.to_string());
    }

    if !required_names.iter().all(|name| seen_names.contains(*name)) {
        return Err(release_evidence_invalid(CONTEXT, "artifacts"));
    }
    if let Some(runtime_bundles_value) = object.get("runtime_bundle_manifests") {
        let runtime_bundles = runtime_bundles_value
            .as_array()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "runtime_bundle_manifests"))?;
        if runtime_bundles.is_empty() {
            return Err(release_evidence_invalid(
                CONTEXT,
                "runtime_bundle_manifests",
            ));
        }
        for runtime_bundle in runtime_bundles {
            let runtime_bundle = runtime_bundle
                .as_object()
                .ok_or_else(|| release_evidence_invalid(CONTEXT, "runtime_bundle_manifests"))?;
            validate_release_evidence_allowed_keys(
                runtime_bundle,
                &[
                    "file",
                    "sha256",
                    "bytes",
                    "schema_version",
                    "runtime_distribution_mode",
                    "runtime_package_binaries_included",
                    "runtime_binaries_included",
                ],
                CONTEXT,
            )?;
            let file = require_release_evidence_string_value(runtime_bundle, "file", CONTEXT)?;
            if !is_release_evidence_basename(file) {
                return Err(release_evidence_invalid(CONTEXT, "file"));
            }
            require_release_evidence_sha256(runtime_bundle, "sha256", CONTEXT)?;
            require_release_evidence_positive_u64(runtime_bundle, "bytes", CONTEXT)?;
            require_release_evidence_string(
                runtime_bundle,
                "schema_version",
                "release.runtime_bundle.v1",
                CONTEXT,
            )?;
            require_release_evidence_string(
                runtime_bundle,
                "runtime_distribution_mode",
                "bundled",
                CONTEXT,
            )?;
            require_release_evidence_bool(
                runtime_bundle,
                "runtime_package_binaries_included",
                true,
                CONTEXT,
            )?;
            require_release_evidence_bool(
                runtime_bundle,
                "runtime_binaries_included",
                false,
                CONTEXT,
            )?;
        }
    }
    for step in [
        "packaging",
        "signing",
        "notarization",
        "github_release_upload",
    ] {
        require_release_evidence_array_contains_string(
            object,
            "blocked_release_steps",
            step,
            CONTEXT,
        )?;
    }

    Ok(())
}

fn validate_release_sbom_report(report: &str) -> Result<()> {
    const CONTEXT: &str = "release SBOM";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "release SBOM blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("release SBOM blocked: invalid JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| CliError::user("release SBOM blocked: expected JSON object"))?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "spdxVersion",
            "dataLicense",
            "SPDXID",
            "name",
            "documentNamespace",
            "creationInfo",
            "packages",
            "relationships",
        ],
        CONTEXT,
    )?;
    validate_release_sbom_root_nested_objects(object)?;
    require_release_evidence_string(object, "spdxVersion", "SPDX-2.3", CONTEXT)?;
    require_release_evidence_string(object, "SPDXID", "SPDXRef-DOCUMENT", CONTEXT)?;
    let name = require_release_evidence_string_value(object, "name", CONTEXT)?;
    let Some(version) = name.strip_prefix("resume-ir-") else {
        return Err(release_evidence_invalid(CONTEXT, "name"));
    };
    validate_release_evidence_version(version, CONTEXT)?;
    let packages = require_release_evidence_array(object, "packages", CONTEXT)?;
    let required_names = ["resume-cli", "resume-daemon", "benchmark-runner"];
    let mut seen_names = BTreeSet::new();

    for package in packages {
        let package = package
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "packages"))?;
        validate_release_evidence_allowed_keys(
            package,
            &[
                "SPDXID",
                "name",
                "versionInfo",
                "supplier",
                "downloadLocation",
                "filesAnalyzed",
                "licenseConcluded",
                "licenseDeclared",
                "copyrightText",
                "checksums",
                "annotations",
                "externalRefs",
                "dependencies",
            ],
            CONTEXT,
        )?;
        validate_release_sbom_package_nested_objects(package)?;
        let name = require_release_evidence_string_value(package, "name", CONTEXT)?;
        if required_names.contains(&name) {
            seen_names.insert(name.to_string());
        }
        match package
            .get("filesAnalyzed")
            .and_then(serde_json::Value::as_bool)
        {
            Some(false) => {}
            _ => return Err(release_evidence_invalid(CONTEXT, "filesAnalyzed")),
        }
        if require_release_evidence_string_value(package, "licenseDeclared", CONTEXT)?
            .trim()
            .is_empty()
        {
            return Err(release_evidence_invalid(CONTEXT, "licenseDeclared"));
        }
        validate_release_sbom_external_refs(package)?;
    }

    if !required_names.iter().all(|name| seen_names.contains(*name)) {
        return Err(release_evidence_invalid(CONTEXT, "packages"));
    }

    Ok(())
}

fn validate_release_sbom_root_nested_objects(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    const CONTEXT: &str = "release SBOM";
    if let Some(creation_info) = object.get("creationInfo") {
        let creation_info = creation_info
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "creationInfo"))?;
        validate_release_evidence_allowed_keys(creation_info, &["created", "creators"], CONTEXT)?;
    }
    if let Some(relationships) = object.get("relationships") {
        let relationships = relationships
            .as_array()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "relationships"))?;
        for relationship in relationships {
            let relationship = relationship
                .as_object()
                .ok_or_else(|| release_evidence_invalid(CONTEXT, "relationships"))?;
            validate_release_evidence_allowed_keys(
                relationship,
                &["spdxElementId", "relationshipType", "relatedSpdxElement"],
                CONTEXT,
            )?;
        }
    }
    Ok(())
}

fn validate_release_sbom_package_nested_objects(
    package: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    const CONTEXT: &str = "release SBOM";
    if let Some(checksums) = package.get("checksums") {
        let checksums = checksums
            .as_array()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "checksums"))?;
        for checksum in checksums {
            let checksum = checksum
                .as_object()
                .ok_or_else(|| release_evidence_invalid(CONTEXT, "checksums"))?;
            validate_release_evidence_allowed_keys(
                checksum,
                &["algorithm", "checksumValue"],
                CONTEXT,
            )?;
        }
    }
    if let Some(annotations) = package.get("annotations") {
        let annotations = annotations
            .as_array()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "annotations"))?;
        for annotation in annotations {
            let annotation = annotation
                .as_object()
                .ok_or_else(|| release_evidence_invalid(CONTEXT, "annotations"))?;
            validate_release_evidence_allowed_keys(
                annotation,
                &["annotationType", "annotator", "annotationDate", "comment"],
                CONTEXT,
            )?;
        }
    }
    if let Some(external_refs) = package.get("externalRefs") {
        let external_refs = external_refs
            .as_array()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "externalRefs"))?;
        for external_ref in external_refs {
            let external_ref = external_ref
                .as_object()
                .ok_or_else(|| release_evidence_invalid(CONTEXT, "externalRefs"))?;
            validate_release_evidence_allowed_keys(
                external_ref,
                &["referenceCategory", "referenceType", "referenceLocator"],
                CONTEXT,
            )?;
        }
    }
    if let Some(dependencies) = package.get("dependencies") {
        let dependencies = dependencies
            .as_array()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "dependencies"))?;
        for dependency in dependencies {
            let dependency = dependency
                .as_object()
                .ok_or_else(|| release_evidence_invalid(CONTEXT, "dependencies"))?;
            validate_release_evidence_allowed_keys(
                dependency,
                &[
                    "name",
                    "req",
                    "kind",
                    "optional",
                    "uses_default_features",
                    "features",
                    "rename",
                    "target",
                ],
                CONTEXT,
            )?;
        }
    }
    Ok(())
}

fn validate_macos_package_manifest_report(report: &str) -> Result<()> {
    const CONTEXT: &str = "macOS package manifest";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "macOS package manifest blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("macOS package manifest blocked: invalid JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| CliError::user("macOS package manifest blocked: expected JSON object"))?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "version",
            "packaging_status",
            "install_location",
            "signing_status",
            "notarization_status",
            "runtime_payload",
            "artifacts",
            "blocked_release_steps",
            "notes",
        ],
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "schema_version",
        "release.macos_package.v1",
        CONTEXT,
    )?;
    let version = require_release_evidence_string_value(object, "version", CONTEXT)?;
    validate_release_evidence_version(version, CONTEXT)?;
    require_release_evidence_string(object, "packaging_status", "unsigned_dry_run", CONTEXT)?;
    require_release_evidence_string(object, "install_location", "/usr/local/bin", CONTEXT)?;
    require_release_evidence_string(object, "signing_status", "unsigned", CONTEXT)?;
    require_release_evidence_string(object, "notarization_status", "not_requested", CONTEXT)?;
    validate_release_package_artifacts(object, CONTEXT, &["pkg", "dmg"])?;
    validate_release_package_runtime_payload(object, CONTEXT, "/usr/local/lib/resume-ir/runtime")?;
    for step in [
        "signing",
        "notarization",
        "github_release_upload",
        "installer_lifecycle_validation",
    ] {
        require_release_evidence_array_contains_string(
            object,
            "blocked_release_steps",
            step,
            CONTEXT,
        )?;
    }

    Ok(())
}

fn validate_windows_package_manifest_report(report: &str) -> Result<()> {
    const CONTEXT: &str = "Windows package manifest";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "Windows package manifest blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("Windows package manifest blocked: invalid JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| CliError::user("Windows package manifest blocked: expected JSON object"))?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "version",
            "packaging_status",
            "installer_kind",
            "install_location",
            "signing_status",
            "runtime_payload",
            "artifacts",
            "blocked_release_steps",
            "notes",
        ],
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "schema_version",
        "release.windows_package.v1",
        CONTEXT,
    )?;
    let version = require_release_evidence_string_value(object, "version", CONTEXT)?;
    validate_release_evidence_version(version, CONTEXT)?;
    require_release_evidence_string(object, "packaging_status", "unsigned_dry_run", CONTEXT)?;
    require_release_evidence_string(object, "installer_kind", "msi", CONTEXT)?;
    require_release_evidence_string(
        object,
        "install_location",
        "ProgramFilesFolder/resume-ir",
        CONTEXT,
    )?;
    require_release_evidence_string(object, "signing_status", "unsigned", CONTEXT)?;
    validate_release_package_artifacts(object, CONTEXT, &["msi"])?;
    validate_release_package_runtime_payload(
        object,
        CONTEXT,
        "ProgramFilesFolder/resume-ir/runtime",
    )?;
    for step in [
        "signing",
        "github_release_upload",
        "installer_lifecycle_validation",
        "service_install_validation",
    ] {
        require_release_evidence_array_contains_string(
            object,
            "blocked_release_steps",
            step,
            CONTEXT,
        )?;
    }

    Ok(())
}

struct InstallerLifecycleActionExpectation {
    action: &'static str,
    command: &'static str,
    target_kind: &'static str,
}

fn validate_macos_installer_lifecycle_plan_report(report: &str) -> Result<()> {
    const CONTEXT: &str = "macOS installer lifecycle plan";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "macOS installer lifecycle plan blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("macOS installer lifecycle plan blocked: invalid JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        CliError::user("macOS installer lifecycle plan blocked: expected JSON object")
    })?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "version",
            "execution_mode",
            "installer_lifecycle_status",
            "evidence_boundary",
            "macos_package_manifest_sha256",
            "admin_elevation",
            "release_runner",
            "installer_artifacts",
            "planned_actions",
            "blocked_release_steps",
            "prohibited_public_material",
            "notes",
        ],
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "schema_version",
        "release.macos_installer_lifecycle_plan.v1",
        CONTEXT,
    )?;
    let version = require_release_evidence_string_value(object, "version", CONTEXT)?;
    validate_release_evidence_version(version, CONTEXT)?;
    require_release_evidence_string(object, "execution_mode", "dry_run", CONTEXT)?;
    require_release_evidence_string(object, "installer_lifecycle_status", "blocked", CONTEXT)?;
    require_release_evidence_string(
        object,
        "evidence_boundary",
        "dry_run_no_macos_installer_execution",
        CONTEXT,
    )?;
    require_release_evidence_sha256(object, "macos_package_manifest_sha256", CONTEXT)?;
    require_release_evidence_string(object, "admin_elevation", "required_not_observed", CONTEXT)?;
    require_release_evidence_string(
        object,
        "release_runner",
        "macos_required_not_observed",
        CONTEXT,
    )?;

    let artifacts = validate_installer_lifecycle_artifacts(object, CONTEXT, &["pkg", "dmg"])?;
    validate_installer_lifecycle_actions(
        object,
        CONTEXT,
        &artifacts,
        &[
            InstallerLifecycleActionExpectation {
                action: "install",
                command: "installer",
                target_kind: "pkg",
            },
            InstallerLifecycleActionExpectation {
                action: "upgrade",
                command: "installer",
                target_kind: "pkg",
            },
            InstallerLifecycleActionExpectation {
                action: "uninstall",
                command: "pkgutil",
                target_kind: "pkg",
            },
            InstallerLifecycleActionExpectation {
                action: "rollback",
                command: "installer",
                target_kind: "pkg",
            },
            InstallerLifecycleActionExpectation {
                action: "launch-agent-start",
                command: "launchctl",
                target_kind: "dmg",
            },
            InstallerLifecycleActionExpectation {
                action: "launch-agent-stop",
                command: "launchctl",
                target_kind: "dmg",
            },
        ],
    )?;
    for step in [
        "macos_pkg_install",
        "macos_pkg_upgrade",
        "macos_pkg_uninstall",
        "macos_pkg_rollback",
        "macos_launch_agent_start",
        "macos_launch_agent_stop",
    ] {
        require_release_evidence_array_contains_string(
            object,
            "blocked_release_steps",
            step,
            CONTEXT,
        )?;
    }
    validate_installer_lifecycle_prohibited_material(object, CONTEXT)
}

fn validate_windows_installer_lifecycle_plan_report(report: &str) -> Result<()> {
    const CONTEXT: &str = "Windows installer lifecycle plan";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "Windows installer lifecycle plan blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("Windows installer lifecycle plan blocked: invalid JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        CliError::user("Windows installer lifecycle plan blocked: expected JSON object")
    })?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "version",
            "execution_mode",
            "installer_lifecycle_status",
            "evidence_boundary",
            "windows_package_manifest_sha256",
            "installer_engine",
            "admin_elevation",
            "release_runner",
            "installation_status",
            "rollback_validation_status",
            "installer_artifacts",
            "planned_actions",
            "blocked_release_steps",
            "prohibited_public_material",
            "notes",
        ],
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "schema_version",
        "release.windows_installer_lifecycle_plan.v1",
        CONTEXT,
    )?;
    let version = require_release_evidence_string_value(object, "version", CONTEXT)?;
    validate_release_evidence_version(version, CONTEXT)?;
    require_release_evidence_string(object, "execution_mode", "dry_run", CONTEXT)?;
    require_release_evidence_string(object, "installer_lifecycle_status", "blocked", CONTEXT)?;
    require_release_evidence_string(
        object,
        "evidence_boundary",
        "dry_run_no_windows_installer_execution",
        CONTEXT,
    )?;
    require_release_evidence_sha256(object, "windows_package_manifest_sha256", CONTEXT)?;
    require_release_evidence_string(object, "installer_engine", "msiexec.exe", CONTEXT)?;
    require_release_evidence_string(object, "admin_elevation", "required_not_observed", CONTEXT)?;
    require_release_evidence_string(
        object,
        "release_runner",
        "windows_required_not_observed",
        CONTEXT,
    )?;
    require_release_evidence_string(object, "installation_status", "not_installed", CONTEXT)?;
    require_release_evidence_string(object, "rollback_validation_status", "blocked", CONTEXT)?;

    let artifacts = validate_installer_lifecycle_artifacts(object, CONTEXT, &["msi"])?;
    validate_installer_lifecycle_actions(
        object,
        CONTEXT,
        &artifacts,
        &[
            InstallerLifecycleActionExpectation {
                action: "install",
                command: "msiexec.exe",
                target_kind: "msi",
            },
            InstallerLifecycleActionExpectation {
                action: "upgrade",
                command: "msiexec.exe",
                target_kind: "msi",
            },
            InstallerLifecycleActionExpectation {
                action: "repair",
                command: "msiexec.exe",
                target_kind: "msi",
            },
            InstallerLifecycleActionExpectation {
                action: "uninstall",
                command: "msiexec.exe",
                target_kind: "msi",
            },
            InstallerLifecycleActionExpectation {
                action: "rollback",
                command: "msiexec.exe",
                target_kind: "msi",
            },
        ],
    )?;
    for step in [
        "windows_msi_install",
        "windows_msi_upgrade",
        "windows_msi_repair",
        "windows_msi_uninstall",
        "windows_msi_rollback",
    ] {
        require_release_evidence_array_contains_string(
            object,
            "blocked_release_steps",
            step,
            CONTEXT,
        )?;
    }
    validate_installer_lifecycle_prohibited_material(object, CONTEXT)
}

fn validate_installer_lifecycle_artifacts(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &'static str,
    required_kinds: &[&'static str],
) -> Result<BTreeMap<String, String>> {
    let artifacts = require_release_evidence_array(object, "installer_artifacts", context)?;
    let mut artifacts_by_file = BTreeMap::new();
    let mut seen_kinds = BTreeSet::new();
    for artifact in artifacts {
        let artifact = artifact
            .as_object()
            .ok_or_else(|| release_evidence_invalid(context, "installer_artifacts"))?;
        validate_release_evidence_allowed_keys(
            artifact,
            &["kind", "file", "artifact_sha256", "bytes"],
            context,
        )?;
        let kind = require_release_evidence_string_value(artifact, "kind", context)?;
        if !required_kinds.contains(&kind) {
            return Err(release_evidence_invalid(context, "installer_artifacts"));
        }
        let file = require_release_evidence_string_value(artifact, "file", context)?;
        if !is_release_evidence_basename(file) {
            return Err(release_evidence_invalid(context, "file"));
        }
        require_release_evidence_sha256(artifact, "artifact_sha256", context)?;
        require_release_evidence_positive_u64(artifact, "bytes", context)?;
        if artifacts_by_file
            .insert(file.to_string(), kind.to_string())
            .is_some()
        {
            return Err(release_evidence_invalid(context, "installer_artifacts"));
        }
        seen_kinds.insert(kind.to_string());
    }
    if required_kinds.iter().all(|kind| seen_kinds.contains(*kind)) {
        Ok(artifacts_by_file)
    } else {
        Err(release_evidence_invalid(context, "installer_artifacts"))
    }
}

fn validate_installer_lifecycle_actions(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &'static str,
    artifacts_by_file: &BTreeMap<String, String>,
    expected_actions: &[InstallerLifecycleActionExpectation],
) -> Result<()> {
    let actions = require_release_evidence_array(object, "planned_actions", context)?;
    if actions.len() != expected_actions.len() {
        return Err(release_evidence_invalid(context, "planned_actions"));
    }
    let mut seen_actions = BTreeSet::new();
    for (action, expected) in actions.iter().zip(expected_actions.iter()) {
        let action = action
            .as_object()
            .ok_or_else(|| release_evidence_invalid(context, "planned_actions"))?;
        validate_release_evidence_allowed_keys(
            action,
            &[
                "action",
                "command",
                "target_artifact",
                "dry_run_intent",
                "requires_approval",
                "action_status",
            ],
            context,
        )?;
        let action_name = require_release_evidence_string_value(action, "action", context)?;
        if action_name != expected.action || !seen_actions.insert(action_name.to_string()) {
            return Err(release_evidence_invalid(context, "planned_actions"));
        }
        require_release_evidence_string(action, "command", expected.command, context)?;
        let target_artifact =
            require_release_evidence_string_value(action, "target_artifact", context)?;
        if !is_release_evidence_basename(target_artifact) {
            return Err(release_evidence_invalid(context, "target_artifact"));
        }
        match artifacts_by_file.get(target_artifact) {
            Some(kind) if kind == expected.target_kind => {}
            _ => return Err(release_evidence_invalid(context, "target_artifact")),
        }
        if require_release_evidence_string_value(action, "dry_run_intent", context)?
            .trim()
            .is_empty()
        {
            return Err(release_evidence_invalid(context, "dry_run_intent"));
        }
        require_release_evidence_bool(action, "requires_approval", true, context)?;
        require_release_evidence_string(action, "action_status", "blocked", context)?;
    }
    Ok(())
}

fn validate_installer_lifecycle_prohibited_material(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &'static str,
) -> Result<()> {
    for material in [
        "installer_tokens",
        "administrator_passwords",
        "local_paths",
        "raw_installer_logs",
        "raw_resume_data",
        "diagnostic_packages",
        "model_artifact_caches",
    ] {
        require_release_evidence_array_contains_string(
            object,
            "prohibited_public_material",
            material,
            context,
        )?;
    }
    Ok(())
}

fn validate_windows_service_lifecycle_plan_report(report: &str) -> Result<()> {
    const CONTEXT: &str = "Windows service lifecycle plan";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
    {
        return Err(CliError::user(
            "Windows service lifecycle plan blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("Windows service lifecycle plan blocked: invalid JSON"))?;
    let object = value.as_object().ok_or_else(|| {
        CliError::user("Windows service lifecycle plan blocked: expected JSON object")
    })?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "version",
            "execution_mode",
            "service_lifecycle_status",
            "evidence_boundary",
            "windows_package_manifest_sha256",
            "service_manager",
            "admin_elevation",
            "release_runner",
            "registration_status",
            "recovery_validation_status",
            "rollback_validation_status",
            "service_artifacts",
            "planned_actions",
            "blocked_release_steps",
            "prohibited_public_material",
            "notes",
        ],
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "schema_version",
        "release.windows_service_lifecycle_plan.v1",
        CONTEXT,
    )?;
    let version = require_release_evidence_string_value(object, "version", CONTEXT)?;
    validate_release_evidence_version(version, CONTEXT)?;
    require_release_evidence_string(object, "execution_mode", "dry_run", CONTEXT)?;
    require_release_evidence_string(object, "service_lifecycle_status", "blocked", CONTEXT)?;
    require_release_evidence_string(
        object,
        "evidence_boundary",
        "dry_run_no_windows_service_registration",
        CONTEXT,
    )?;
    require_release_evidence_sha256(object, "windows_package_manifest_sha256", CONTEXT)?;
    require_release_evidence_string(object, "service_manager", "sc.exe", CONTEXT)?;
    require_release_evidence_string(object, "admin_elevation", "required_not_observed", CONTEXT)?;
    require_release_evidence_string(
        object,
        "release_runner",
        "windows_required_not_observed",
        CONTEXT,
    )?;
    require_release_evidence_string(object, "registration_status", "not_registered", CONTEXT)?;
    require_release_evidence_string(object, "recovery_validation_status", "blocked", CONTEXT)?;
    require_release_evidence_string(object, "rollback_validation_status", "blocked", CONTEXT)?;

    let target_artifacts = validate_windows_service_lifecycle_artifacts(object)?;
    validate_windows_service_lifecycle_actions(object, &target_artifacts)?;
    for step in [
        "windows_service_install",
        "windows_service_start",
        "windows_service_status",
        "windows_service_stop",
        "windows_service_recovery",
        "windows_service_uninstall",
        "windows_service_rollback",
    ] {
        require_release_evidence_array_contains_string(
            object,
            "blocked_release_steps",
            step,
            CONTEXT,
        )?;
    }
    for material in [
        "service_tokens",
        "administrator_passwords",
        "local_paths",
        "raw_service_logs",
        "raw_resume_data",
        "diagnostic_packages",
        "model_artifact_caches",
    ] {
        require_release_evidence_array_contains_string(
            object,
            "prohibited_public_material",
            material,
            CONTEXT,
        )?;
    }

    Ok(())
}

fn validate_windows_service_lifecycle_artifacts(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<BTreeSet<String>> {
    const CONTEXT: &str = "Windows service lifecycle plan";
    let artifacts = require_release_evidence_array(object, "service_artifacts", CONTEXT)?;
    let mut target_artifacts = BTreeSet::new();
    for artifact in artifacts {
        let artifact = artifact
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "service_artifacts"))?;
        validate_release_evidence_allowed_keys(
            artifact,
            &[
                "kind",
                "file",
                "artifact_sha256",
                "bytes",
                "service_validation_status",
            ],
            CONTEXT,
        )?;
        require_release_evidence_string(artifact, "kind", "msi", CONTEXT)?;
        let file = require_release_evidence_string_value(artifact, "file", CONTEXT)?;
        if !is_release_evidence_basename(file) {
            return Err(release_evidence_invalid(CONTEXT, "file"));
        }
        require_release_evidence_sha256(artifact, "artifact_sha256", CONTEXT)?;
        require_release_evidence_positive_u64(artifact, "bytes", CONTEXT)?;
        require_release_evidence_string(
            artifact,
            "service_validation_status",
            "not_executed",
            CONTEXT,
        )?;
        target_artifacts.insert(file.to_string());
    }
    if target_artifacts.is_empty() {
        Err(release_evidence_invalid(CONTEXT, "service_artifacts"))
    } else {
        Ok(target_artifacts)
    }
}

fn validate_windows_service_lifecycle_actions(
    object: &serde_json::Map<String, serde_json::Value>,
    target_artifacts: &BTreeSet<String>,
) -> Result<()> {
    const CONTEXT: &str = "Windows service lifecycle plan";
    let actions = require_release_evidence_array(object, "planned_actions", CONTEXT)?;
    let expected_actions = [
        "install",
        "start",
        "status",
        "stop",
        "recovery",
        "uninstall",
        "rollback",
    ];
    if actions.len() != expected_actions.len() {
        return Err(release_evidence_invalid(CONTEXT, "planned_actions"));
    }
    let mut seen_actions = BTreeSet::new();
    for (action, expected_action) in actions.iter().zip(expected_actions) {
        let action = action
            .as_object()
            .ok_or_else(|| release_evidence_invalid(CONTEXT, "planned_actions"))?;
        validate_release_evidence_allowed_keys(
            action,
            &[
                "action",
                "command",
                "target_artifact",
                "dry_run_intent",
                "requires_approval",
                "action_status",
            ],
            CONTEXT,
        )?;
        let action_name = require_release_evidence_string_value(action, "action", CONTEXT)?;
        if action_name != expected_action || !seen_actions.insert(action_name.to_string()) {
            return Err(release_evidence_invalid(CONTEXT, "planned_actions"));
        }
        require_release_evidence_string(action, "command", "sc.exe", CONTEXT)?;
        let target_artifact =
            require_release_evidence_string_value(action, "target_artifact", CONTEXT)?;
        if !is_release_evidence_basename(target_artifact)
            || !target_artifacts.contains(target_artifact)
        {
            return Err(release_evidence_invalid(CONTEXT, "target_artifact"));
        }
        if require_release_evidence_string_value(action, "dry_run_intent", CONTEXT)?
            .trim()
            .is_empty()
        {
            return Err(release_evidence_invalid(CONTEXT, "dry_run_intent"));
        }
        require_release_evidence_bool(action, "requires_approval", true, CONTEXT)?;
        require_release_evidence_string(action, "action_status", "blocked", CONTEXT)?;
    }

    Ok(())
}

fn validate_hardware_fault_drill_evidence_report(report: &str) -> Result<()> {
    const CONTEXT: &str = "hardware fault drills";
    if release_readiness_diagnostics_report_contains_private_marker(report)
        || release_evidence_report_contains_forbidden_marker(report)
        || report.contains("PRIVATE-")
    {
        return Err(CliError::user(
            "hardware fault drills blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("hardware fault drills blocked: invalid JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| CliError::user("hardware fault drills blocked: expected JSON object"))?;

    validate_release_evidence_allowed_keys(
        object,
        &[
            "schema_version",
            "evidence_boundary",
            "execution_mode",
            "artifact_manifest_sha256",
            "build_sha",
            "redacted",
            "dedicated_test_environment",
            "cleanup_verified",
            "contains_local_paths",
            "contains_raw_resume_text",
            "contains_secrets",
            "contains_diagnostics_package",
            "drills",
            "must_not_upload",
        ],
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "schema_version",
        "release.hardware_fault_drills.v1",
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "evidence_boundary",
        "redacted_release_hardware_fault_drills",
        CONTEXT,
    )?;
    require_release_evidence_string(
        object,
        "execution_mode",
        "actual_release_platform_drill",
        CONTEXT,
    )?;
    require_release_evidence_sha256(object, "artifact_manifest_sha256", CONTEXT)?;
    validate_release_hardware_fault_build_sha(object, CONTEXT)?;
    for key in ["redacted", "dedicated_test_environment", "cleanup_verified"] {
        require_release_evidence_bool(object, key, true, CONTEXT)?;
    }
    for key in [
        "contains_local_paths",
        "contains_raw_resume_text",
        "contains_secrets",
        "contains_diagnostics_package",
    ] {
        require_release_evidence_bool(object, key, false, CONTEXT)?;
    }

    let drills = require_release_evidence_array(object, "drills", CONTEXT)?;
    validate_release_hardware_fault_drills(drills, CONTEXT)?;
    for item in [
        "raw resumes",
        "local paths",
        "diagnostics packages",
        "tokens",
        "model caches",
        "indexes",
        "SQLite databases",
    ] {
        require_release_evidence_array_contains_string(object, "must_not_upload", item, CONTEXT)?;
    }
    Ok(())
}

fn validate_release_hardware_fault_build_sha(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &'static str,
) -> Result<()> {
    let build_sha = require_release_evidence_string_value(object, "build_sha", context)?;
    let is_full_git_sha = build_sha.len() == 40
        && build_sha
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if is_full_git_sha {
        Ok(())
    } else {
        Err(release_evidence_invalid(context, "build_sha"))
    }
}

fn validate_release_hardware_fault_drills(
    drills: &[serde_json::Value],
    context: &'static str,
) -> Result<()> {
    let expected_drills = [
        "actual_enospc",
        "service_daemon_kill",
        "battery_mode",
        "external_drive_disconnect",
    ];
    if drills.len() != expected_drills.len() {
        return Err(release_evidence_invalid(context, "drills"));
    }
    let mut seen_drills = BTreeSet::new();
    for (drill, expected_drill) in drills.iter().zip(expected_drills.iter()) {
        let drill = drill
            .as_object()
            .ok_or_else(|| release_evidence_invalid(context, "drills"))?;
        validate_release_evidence_allowed_keys(
            drill,
            &[
                "drill",
                "status",
                "evidence_kind",
                "platforms",
                "transcript_sha256",
                "diagnostics_sha256",
            ],
            context,
        )?;
        let drill_id = require_release_evidence_string_value(drill, "drill", context)?;
        if drill_id != *expected_drill || !seen_drills.insert(drill_id.to_string()) {
            return Err(release_evidence_invalid(context, "drills"));
        }
        require_release_evidence_string(drill, "status", "passed", context)?;
        require_release_evidence_string(
            drill,
            "evidence_kind",
            "actual_release_platform_drill",
            context,
        )?;
        let platforms = require_release_evidence_object(drill, "platforms", context)?;
        validate_release_evidence_allowed_keys(platforms, &["macos", "windows"], context)?;
        require_release_evidence_string(platforms, "macos", "passed", context)?;
        require_release_evidence_string(platforms, "windows", "passed", context)?;
        require_release_evidence_sha256(drill, "transcript_sha256", context)?;
        require_release_evidence_sha256(drill, "diagnostics_sha256", context)?;
    }
    Ok(())
}

fn validate_release_package_artifacts(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &'static str,
    required_kinds: &[&str],
) -> Result<()> {
    let artifacts = require_release_evidence_array(object, "artifacts", context)?;
    let mut seen_kinds = BTreeSet::new();
    for artifact in artifacts {
        let artifact = artifact
            .as_object()
            .ok_or_else(|| release_evidence_invalid(context, "artifacts"))?;
        validate_release_evidence_allowed_keys(
            artifact,
            &["kind", "file", "sha256", "bytes"],
            context,
        )?;
        let kind = require_release_evidence_string_value(artifact, "kind", context)?;
        if !required_kinds.contains(&kind) {
            return Err(release_evidence_invalid(context, "artifacts"));
        }
        let file = require_release_evidence_string_value(artifact, "file", context)?;
        if !is_release_evidence_basename(file) {
            return Err(release_evidence_invalid(context, "file"));
        }
        require_release_evidence_sha256(artifact, "sha256", context)?;
        require_release_evidence_positive_u64(artifact, "bytes", context)?;
        seen_kinds.insert(kind.to_string());
    }
    if required_kinds.iter().all(|kind| seen_kinds.contains(*kind)) {
        Ok(())
    } else {
        Err(release_evidence_invalid(context, "artifacts"))
    }
}

fn validate_release_package_runtime_payload(
    object: &serde_json::Map<String, serde_json::Value>,
    context: &'static str,
    expected_install_location: &str,
) -> Result<()> {
    let Some(payload) = object.get("runtime_payload") else {
        return Ok(());
    };
    let payload = payload
        .as_object()
        .ok_or_else(|| release_evidence_invalid(context, "runtime_payload"))?;
    validate_release_evidence_allowed_keys(
        payload,
        &[
            "schema_version",
            "runtime_distribution_mode",
            "runtime_package_binaries_included",
            "runtime_binaries_included_in_manifest",
            "install_location",
            "runtime_bundle_manifest",
            "components",
        ],
        context,
    )?;
    require_release_evidence_string(
        payload,
        "schema_version",
        "release.runtime_package_payload.v1",
        context,
    )?;
    require_release_evidence_string(payload, "runtime_distribution_mode", "bundled", context)?;
    require_release_evidence_bool(payload, "runtime_package_binaries_included", true, context)?;
    require_release_evidence_bool(
        payload,
        "runtime_binaries_included_in_manifest",
        false,
        context,
    )?;
    require_release_evidence_string(
        payload,
        "install_location",
        expected_install_location,
        context,
    )?;

    let bundle_manifest =
        require_release_evidence_object(payload, "runtime_bundle_manifest", context)?;
    validate_release_evidence_allowed_keys(
        bundle_manifest,
        &[
            "file",
            "sha256",
            "bytes",
            "schema_version",
            "runtime_distribution_mode",
        ],
        context,
    )?;
    let bundle_manifest_file =
        require_release_evidence_string_value(bundle_manifest, "file", context)?;
    if !is_release_evidence_basename(bundle_manifest_file) {
        return Err(release_evidence_invalid(context, "runtime_bundle_manifest"));
    }
    require_release_evidence_sha256(bundle_manifest, "sha256", context)?;
    require_release_evidence_positive_u64(bundle_manifest, "bytes", context)?;
    require_release_evidence_string(
        bundle_manifest,
        "schema_version",
        "release.runtime_bundle.v1",
        context,
    )?;
    require_release_evidence_string(
        bundle_manifest,
        "runtime_distribution_mode",
        "bundled",
        context,
    )?;

    let components = require_release_evidence_array(payload, "components", context)?;
    if components.is_empty() {
        return Err(release_evidence_invalid(context, "components"));
    }
    let mut seen_files = BTreeSet::new();
    let mut ocr_component_kinds = BTreeSet::new();
    for component in components {
        let component = component
            .as_object()
            .ok_or_else(|| release_evidence_invalid(context, "components"))?;
        validate_release_evidence_allowed_keys(
            component,
            &["id", "kind", "file", "sha256", "bytes", "license", "source"],
            context,
        )?;
        require_release_evidence_string_value(component, "id", context)?;
        let kind = require_release_evidence_string_value(component, "kind", context)?;
        if matches!(kind, "ocr-engine" | "pdf-renderer" | "ocr-language-pack") {
            ocr_component_kinds.insert(kind.to_string());
        }
        let file = require_release_evidence_string_value(component, "file", context)?;
        if !is_release_evidence_basename(file) || !seen_files.insert(file.to_string()) {
            return Err(release_evidence_invalid(context, "components"));
        }
        require_release_evidence_sha256(component, "sha256", context)?;
        require_release_evidence_positive_u64(component, "bytes", context)?;
        require_release_evidence_string_value(component, "license", context)?;
        let source = require_release_evidence_string_value(component, "source", context)?;
        if source.starts_with('/')
            || source.contains("PRIVATE-")
            || source.contains("/Users/")
            || release_readiness_diagnostics_report_contains_private_marker(source)
        {
            return Err(release_evidence_invalid(context, "source"));
        }
    }
    if !ocr_component_kinds.is_empty()
        && !["ocr-engine", "pdf-renderer", "ocr-language-pack"]
            .iter()
            .all(|kind| ocr_component_kinds.contains(*kind))
    {
        return Err(release_evidence_invalid(context, "ocr_runtime_components"));
    }

    Ok(())
}

fn validate_release_sbom_external_refs(
    package: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    if release_sbom_package_has_annotation(package, "runtime_distribution_mode=bundled") {
        return validate_release_sbom_runtime_package(package);
    }

    let external_refs = package
        .get("externalRefs")
        .and_then(serde_json::Value::as_array)
        .filter(|external_refs| !external_refs.is_empty())
        .ok_or_else(|| release_evidence_invalid("release SBOM", "externalRefs"))?;
    let has_purl = external_refs.iter().any(|external_ref| {
        let Some(external_ref) = external_ref.as_object() else {
            return false;
        };
        external_ref
            .get("referenceType")
            .and_then(serde_json::Value::as_str)
            == Some("purl")
            && external_ref
                .get("referenceLocator")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|locator| locator.starts_with("pkg:cargo/"))
    });
    if has_purl {
        Ok(())
    } else {
        Err(release_evidence_invalid("release SBOM", "externalRefs"))
    }
}

fn validate_release_sbom_runtime_package(
    package: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    let download_location =
        require_release_evidence_string_value(package, "downloadLocation", "release SBOM")?;
    if download_location == "NOASSERTION" || download_location.starts_with('/') {
        return Err(release_evidence_invalid("release SBOM", "downloadLocation"));
    }
    if release_readiness_diagnostics_report_contains_private_marker(download_location)
        || download_location.contains("PRIVATE-")
    {
        return Err(release_evidence_invalid("release SBOM", "downloadLocation"));
    }

    let checksums = require_release_evidence_array(package, "checksums", "release SBOM")?;
    let has_sha256 = checksums.iter().any(|checksum| {
        let Some(checksum) = checksum.as_object() else {
            return false;
        };
        checksum
            .get("algorithm")
            .and_then(serde_json::Value::as_str)
            == Some("SHA256")
            && checksum
                .get("checksumValue")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| {
                    value.len() == 64
                        && value
                            .bytes()
                            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
                })
    });
    if !has_sha256 {
        return Err(release_evidence_invalid("release SBOM", "checksums"));
    }

    for expected in [
        "runtime_package_binaries_included=true",
        "runtime_binaries_included=false",
    ] {
        if !release_sbom_package_has_annotation(package, expected) {
            return Err(release_evidence_invalid("release SBOM", "annotations"));
        }
    }
    if !release_sbom_package_has_annotation_prefix(package, "runtime_component_kind=")
        || !release_sbom_package_has_annotation_prefix(package, "runtime_component_file=")
        || !release_sbom_package_has_annotation_prefix(package, "source_offer_sha256=")
    {
        return Err(release_evidence_invalid("release SBOM", "annotations"));
    }

    let external_refs = package
        .get("externalRefs")
        .and_then(serde_json::Value::as_array)
        .filter(|external_refs| !external_refs.is_empty())
        .ok_or_else(|| release_evidence_invalid("release SBOM", "externalRefs"))?;
    let has_runtime_ref = external_refs.iter().any(|external_ref| {
        let Some(external_ref) = external_ref.as_object() else {
            return false;
        };
        external_ref
            .get("referenceType")
            .and_then(serde_json::Value::as_str)
            == Some("persistent-id")
            && external_ref
                .get("referenceLocator")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|locator| locator.starts_with("runtime-bundle:"))
    });
    if has_runtime_ref {
        Ok(())
    } else {
        Err(release_evidence_invalid("release SBOM", "externalRefs"))
    }
}

fn release_sbom_package_has_annotation(
    package: &serde_json::Map<String, serde_json::Value>,
    expected: &str,
) -> bool {
    release_sbom_package_has_annotation_matching(package, |comment| comment == expected)
}

fn release_sbom_package_has_annotation_prefix(
    package: &serde_json::Map<String, serde_json::Value>,
    expected_prefix: &str,
) -> bool {
    release_sbom_package_has_annotation_matching(package, |comment| {
        comment.starts_with(expected_prefix)
    })
}

fn release_sbom_package_has_annotation_matching(
    package: &serde_json::Map<String, serde_json::Value>,
    predicate: impl Fn(&str) -> bool,
) -> bool {
    package
        .get("annotations")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|annotations| {
            annotations.iter().any(|annotation| {
                annotation
                    .as_object()
                    .and_then(|annotation| annotation.get("comment"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(&predicate)
            })
        })
}

fn release_evidence_report_contains_forbidden_marker(report: &str) -> bool {
    [
        "manifest_path",
        "src_path",
        "license_file",
        "target/release",
        "local-data",
        "diagnostics.zip",
        "model-cache",
    ]
    .iter()
    .any(|marker| report.contains(marker))
}

fn validate_release_evidence_allowed_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed_keys: &[&str],
    context: &'static str,
) -> Result<()> {
    for key in object.keys() {
        if !allowed_keys.contains(&key.as_str()) {
            return Err(CliError::user(format!(
                "{context} blocked: {key} is not allowed"
            )));
        }
    }
    Ok(())
}

fn require_release_evidence_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    expected: &str,
    context: &'static str,
) -> Result<()> {
    match object.get(key).and_then(serde_json::Value::as_str) {
        Some(value) if value == expected => Ok(()),
        _ => Err(release_evidence_invalid(context, key)),
    }
}

fn require_release_evidence_string_value<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<&'a str> {
    object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| release_evidence_invalid(context, key))
}

fn require_release_evidence_non_empty_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<&'a str> {
    let value = require_release_evidence_string_value(object, key, context)?;
    if value.trim().is_empty() {
        Err(release_evidence_invalid(context, key))
    } else {
        Ok(value)
    }
}

fn require_release_evidence_object<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    object
        .get(key)
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| release_evidence_invalid(context, key))
}

fn require_release_evidence_array<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<&'a Vec<serde_json::Value>> {
    object
        .get(key)
        .and_then(serde_json::Value::as_array)
        .filter(|values| !values.is_empty())
        .ok_or_else(|| release_evidence_invalid(context, key))
}

fn require_release_evidence_array_contains_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    expected: &str,
    context: &'static str,
) -> Result<()> {
    let values = require_release_evidence_array(object, key, context)?;
    if values.iter().any(|value| value.as_str() == Some(expected)) {
        Ok(())
    } else {
        Err(release_evidence_invalid(context, key))
    }
}

fn require_release_evidence_exact_string_array(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    expected: &[&str],
    context: &'static str,
) -> Result<()> {
    let values = require_release_evidence_array(object, key, context)?;
    if values.len() != expected.len() {
        return Err(release_evidence_invalid(context, key));
    }
    let mut seen = BTreeSet::new();
    for (value, expected_value) in values.iter().zip(expected.iter()) {
        let actual = value
            .as_str()
            .ok_or_else(|| release_evidence_invalid(context, key))?;
        if actual != *expected_value || !seen.insert(actual.to_string()) {
            return Err(release_evidence_invalid(context, key));
        }
    }
    Ok(())
}

fn require_release_evidence_exact_steps(
    steps: &[serde_json::Value],
    expected_steps: &[(&str, &str)],
    context: &'static str,
) -> Result<()> {
    if steps.len() != expected_steps.len() {
        return Err(release_evidence_invalid(context, "steps"));
    }
    let mut seen_ids = BTreeSet::new();
    for (step, (expected_id, expected_status)) in steps.iter().zip(expected_steps.iter()) {
        let step = step
            .as_object()
            .ok_or_else(|| release_evidence_invalid(context, "steps"))?;
        validate_release_evidence_allowed_keys(step, &["id", "status", "exit_code"], context)?;
        let id = step
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| release_evidence_invalid(context, "steps"))?;
        let status = step
            .get("status")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| release_evidence_invalid(context, "steps"))?;
        if id != *expected_id || status != *expected_status || !seen_ids.insert(id.to_string()) {
            return Err(release_evidence_invalid(context, "steps"));
        }
    }
    Ok(())
}

fn require_release_evidence_step_exit_code(
    steps: &[serde_json::Value],
    expected_id: &str,
    expected_status: &str,
    expected_exit_code: u64,
    context: &'static str,
) -> Result<()> {
    let has_step = steps.iter().any(|step| {
        let Some(step) = step.as_object() else {
            return false;
        };
        step.get("id").and_then(serde_json::Value::as_str) == Some(expected_id)
            && step.get("status").and_then(serde_json::Value::as_str) == Some(expected_status)
            && step.get("exit_code").and_then(serde_json::Value::as_u64) == Some(expected_exit_code)
    });
    if has_step {
        Ok(())
    } else {
        Err(release_evidence_invalid(context, "steps"))
    }
}

fn require_release_evidence_sha256(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<()> {
    require_release_evidence_sha256_value(object, key, context).map(|_| ())
}

fn require_release_evidence_sha256_value<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<&'a str> {
    let Some(value) = object.get(key).and_then(serde_json::Value::as_str) else {
        return Err(release_evidence_invalid(context, key));
    };
    let is_sha256 = value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if is_sha256 {
        Ok(value)
    } else {
        Err(release_evidence_invalid(context, key))
    }
}

fn require_release_evidence_optional_sha256_value<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<Option<&'a str>> {
    let Some(value) = object.get(key) else {
        return Err(release_evidence_invalid(context, key));
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_str() else {
        return Err(release_evidence_invalid(context, key));
    };
    let is_sha256 = value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if is_sha256 {
        Ok(Some(value))
    } else {
        Err(release_evidence_invalid(context, key))
    }
}

fn require_release_evidence_output_digest(
    output_digests: &BTreeMap<String, String>,
    file: &str,
    expected_sha256: &str,
    context: &'static str,
) -> Result<()> {
    match output_digests.get(file) {
        Some(value) if value == expected_sha256 => Ok(()),
        _ => Err(release_evidence_invalid(context, "redacted_outputs")),
    }
}

fn require_release_evidence_bool(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    expected: bool,
    context: &'static str,
) -> Result<()> {
    match object.get(key).and_then(serde_json::Value::as_bool) {
        Some(value) if value == expected => Ok(()),
        _ => Err(release_evidence_invalid(context, key)),
    }
}

fn require_release_evidence_bool_value(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<bool> {
    object
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| release_evidence_invalid(context, key))
}

fn require_release_evidence_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    expected: u64,
    context: &'static str,
) -> Result<()> {
    match object.get(key).and_then(serde_json::Value::as_u64) {
        Some(value) if value == expected => Ok(()),
        _ => Err(release_evidence_invalid(context, key)),
    }
}

fn require_release_evidence_min_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    min: u64,
    context: &'static str,
) -> Result<()> {
    match object.get(key).and_then(serde_json::Value::as_u64) {
        Some(value) if value >= min => Ok(()),
        _ => Err(release_evidence_invalid(context, key)),
    }
}

fn require_release_evidence_positive_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<()> {
    match object.get(key).and_then(serde_json::Value::as_u64) {
        Some(value) if value > 0 => Ok(()),
        _ => Err(release_evidence_invalid(context, key)),
    }
}

fn require_release_evidence_positive_u64_value(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<u64> {
    match object.get(key).and_then(serde_json::Value::as_u64) {
        Some(value) if value > 0 => Ok(value),
        _ => Err(release_evidence_invalid(context, key)),
    }
}

fn require_release_evidence_number_value(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<f64> {
    match object.get(key).and_then(serde_json::Value::as_f64) {
        Some(value) if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err(release_evidence_invalid(context, key)),
    }
}

fn validate_current_stage_private_query_observability(
    value: &serde_json::Value,
    context: &'static str,
) -> Result<()> {
    const QUERY_BUCKETS: [&str; 7] = [
        "single_term",
        "and_2",
        "and_3_5",
        "and_6_16",
        "field_filter",
        "hybrid",
        "semantic",
    ];
    const STAGES: [&str; 7] = [
        "query_parse",
        "prefilter",
        "bm25",
        "ann",
        "fusion",
        "bulk_hydrate",
        "snippet",
    ];
    const HISTOGRAM_BOUNDS_MS: [f64; 13] = [
        1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0,
        60_000.0,
    ];

    let object = value
        .as_object()
        .ok_or_else(|| release_evidence_invalid(context, "private_query_observability"))?;
    validate_release_evidence_allowed_keys(
        object,
        &[
            "privacy_boundary",
            "dataset_kind",
            "document_count",
            "searchable_document_count",
            "vector_indexed_document_count",
            "query_count",
            "request_sample_count",
            "query_source",
            "private_scale_gate",
            "query_set_sha256",
            "tune_sha256",
            "holdout_sha256",
            "bucket_counts",
            "tune_bucket_counts",
            "holdout_bucket_counts",
            "samples_per_bucket",
            "query_latency_ms",
            "query_latency_by_bucket",
            "stage_latency_p95_ms",
            "stage_latency_by_bucket_p95_ms",
            "stage_histogram_ms",
            "stage_histogram_by_bucket_ms",
            "rss_delta_mb",
            "rss_delta_mb_by_bucket",
            "zero_result_queries",
            "query_runner",
            "query_mode",
            "retrieval_layers",
            "warm_or_cold_definition",
            "cache_state",
            "percentile_confidence",
            "spawn_per_query",
            "hot_index",
            "hot_path_ocr",
            "hot_path_parsing",
            "hot_path_heavy_model_inference",
            "contains_raw_resume_text",
            "contains_resume_paths",
            "contains_queries",
        ],
        context,
    )?;
    require_release_evidence_string(
        object,
        "privacy_boundary",
        "redacted_local_aggregate",
        context,
    )?;
    require_release_evidence_string(object, "dataset_kind", "private-real-corpus", context)?;
    require_release_evidence_string(object, "query_source", "trace_source_search_v1", context)?;
    match object.get("private_scale_gate") {
        Some(serde_json::Value::String(value)) if value == CURRENT_STAGE_D10K_SCALE_GATE => {}
        _ => return Err(release_evidence_invalid(context, "private_scale_gate")),
    }
    require_release_evidence_string(object, "query_runner", "resident-batch-command", context)?;
    require_release_evidence_string(object, "query_mode", "hybrid", context)?;
    require_release_evidence_string(
        object,
        "retrieval_layers",
        "fulltext+field+vector+rrf",
        context,
    )?;
    require_release_evidence_string(
        object,
        "warm_or_cold_definition",
        "current_stage_single_resident_batch_no_extra_warmup",
        context,
    )?;
    require_release_evidence_string(
        object,
        "cache_state",
        "hot_index_fully_covered_resident_batch_os_cache_uncontrolled",
        context,
    )?;
    require_release_evidence_string(object, "percentile_confidence", "sampled", context)?;
    require_release_evidence_bool(object, "spawn_per_query", false, context)?;
    require_release_evidence_bool(object, "hot_index", true, context)?;
    require_release_evidence_sha256(object, "query_set_sha256", context)?;
    require_release_evidence_sha256(object, "tune_sha256", context)?;
    require_release_evidence_sha256(object, "holdout_sha256", context)?;
    for key in [
        "hot_path_ocr",
        "hot_path_parsing",
        "hot_path_heavy_model_inference",
        "contains_raw_resume_text",
        "contains_resume_paths",
        "contains_queries",
    ] {
        require_release_evidence_bool(object, key, false, context)?;
    }

    let document_count =
        require_release_evidence_positive_u64_value(object, "document_count", context)?;
    let searchable_document_count =
        require_release_evidence_positive_u64_value(object, "searchable_document_count", context)?;
    let vector_indexed_document_count = require_release_evidence_positive_u64_value(
        object,
        "vector_indexed_document_count",
        context,
    )?;
    let query_count = require_release_evidence_positive_u64_value(object, "query_count", context)?;
    let request_sample_count =
        require_release_evidence_positive_u64_value(object, "request_sample_count", context)?;
    let zero_result_queries = require_release_evidence_positive_or_zero_u64_value(
        object,
        "zero_result_queries",
        context,
    )?;
    if document_count < CURRENT_STAGE_D10K_DOCUMENT_MIN
        || searchable_document_count < CURRENT_STAGE_D10K_SEARCHABLE_DOCUMENT_MIN
        || vector_indexed_document_count < CURRENT_STAGE_D10K_VECTOR_DOCUMENT_MIN
        || searchable_document_count > document_count
        || vector_indexed_document_count > searchable_document_count
        || query_count < CURRENT_STAGE_D10K_QUERY_MIN
        || request_sample_count < CURRENT_STAGE_D10K_REQUEST_SAMPLE_MIN
        || request_sample_count < query_count
        || zero_result_queries > request_sample_count
    {
        return Err(release_evidence_invalid(
            context,
            "private_query_observability",
        ));
    }

    let bucket_counts = require_release_evidence_object(object, "bucket_counts", context)?;
    validate_release_evidence_allowed_keys(bucket_counts, &QUERY_BUCKETS, context)?;
    let tune_bucket_counts =
        require_release_evidence_object(object, "tune_bucket_counts", context)?;
    validate_release_evidence_allowed_keys(tune_bucket_counts, &QUERY_BUCKETS, context)?;
    let holdout_bucket_counts =
        require_release_evidence_object(object, "holdout_bucket_counts", context)?;
    validate_release_evidence_allowed_keys(holdout_bucket_counts, &QUERY_BUCKETS, context)?;
    let samples_per_bucket =
        require_release_evidence_object(object, "samples_per_bucket", context)?;
    validate_release_evidence_allowed_keys(samples_per_bucket, &QUERY_BUCKETS, context)?;
    let mut total_queries = 0;
    let mut tune_queries = 0;
    let mut holdout_queries = 0;
    let mut total_samples = 0;
    for bucket in QUERY_BUCKETS {
        let bucket_count =
            require_release_evidence_positive_or_zero_u64_value(bucket_counts, bucket, context)?;
        let tune_count = require_release_evidence_positive_or_zero_u64_value(
            tune_bucket_counts,
            bucket,
            context,
        )?;
        let holdout_count = require_release_evidence_positive_or_zero_u64_value(
            holdout_bucket_counts,
            bucket,
            context,
        )?;
        if tune_count.checked_add(holdout_count) != Some(bucket_count) {
            return Err(release_evidence_invalid(context, "tune_bucket_counts"));
        }
        total_queries += bucket_count;
        tune_queries += tune_count;
        holdout_queries += holdout_count;
        let sample_count = require_release_evidence_positive_or_zero_u64_value(
            samples_per_bucket,
            bucket,
            context,
        )?;
        let min_query_count = D10K_TRACE_QUERY_BUCKET_MIN_COUNTS
            .iter()
            .find_map(|(expected_bucket, min_count)| {
                (*expected_bucket == bucket).then_some(*min_count as u64)
            })
            .unwrap_or(CURRENT_STAGE_D10K_QUERY_MIN);
        if bucket_count < min_query_count {
            return Err(release_evidence_invalid(context, "bucket_counts"));
        }
        if sample_count < CURRENT_STAGE_D10K_SAMPLES_PER_BUCKET_MIN {
            return Err(release_evidence_invalid(context, "samples_per_bucket"));
        }
        total_samples += sample_count;
    }
    if total_queries != query_count
        || tune_queries.checked_add(holdout_queries) != Some(query_count)
        || total_samples != request_sample_count
    {
        return Err(release_evidence_invalid(context, "samples_per_bucket"));
    }

    let query_latency = require_release_evidence_object(object, "query_latency_ms", context)?;
    validate_current_stage_latency_observability(
        query_latency,
        request_sample_count,
        "query_latency_ms",
        context,
    )?;

    let query_latency_by_bucket =
        require_release_evidence_object(object, "query_latency_by_bucket", context)?;
    validate_release_evidence_allowed_keys(query_latency_by_bucket, &QUERY_BUCKETS, context)?;
    for bucket in QUERY_BUCKETS {
        let sample_count = require_release_evidence_positive_or_zero_u64_value(
            samples_per_bucket,
            bucket,
            context,
        )?;
        let Some(latency) = query_latency_by_bucket.get(bucket) else {
            return Err(release_evidence_invalid(context, "query_latency_by_bucket"));
        };
        let latency = latency
            .as_object()
            .ok_or_else(|| release_evidence_invalid(context, "query_latency_by_bucket"))?;
        validate_current_stage_latency_observability(
            latency,
            sample_count,
            "query_latency_by_bucket",
            context,
        )?;
    }

    let stage_latency = require_release_evidence_object(object, "stage_latency_p95_ms", context)?;
    validate_release_evidence_allowed_keys(stage_latency, &STAGES, context)?;
    for stage in STAGES {
        require_release_evidence_number_value(stage_latency, stage, context)?;
    }

    let stage_latency_by_bucket =
        require_release_evidence_object(object, "stage_latency_by_bucket_p95_ms", context)?;
    validate_release_evidence_allowed_keys(stage_latency_by_bucket, &QUERY_BUCKETS, context)?;
    for bucket in QUERY_BUCKETS {
        let bucket_stages = stage_latency_by_bucket
            .get(bucket)
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| release_evidence_invalid(context, "stage_latency_by_bucket_p95_ms"))?;
        validate_release_evidence_allowed_keys(bucket_stages, &STAGES, context)?;
        for stage in STAGES {
            require_release_evidence_number_value(bucket_stages, stage, context)?;
        }
    }

    let stage_histogram = require_release_evidence_object(object, "stage_histogram_ms", context)?;
    validate_current_stage_stage_histogram_summary(
        stage_histogram,
        request_sample_count,
        &STAGES,
        &HISTOGRAM_BOUNDS_MS,
        "stage_histogram_ms",
        context,
    )?;

    let stage_histogram_by_bucket =
        require_release_evidence_object(object, "stage_histogram_by_bucket_ms", context)?;
    validate_release_evidence_allowed_keys(stage_histogram_by_bucket, &QUERY_BUCKETS, context)?;
    for bucket in QUERY_BUCKETS {
        let sample_count = require_release_evidence_positive_or_zero_u64_value(
            samples_per_bucket,
            bucket,
            context,
        )?;
        let bucket_stages = stage_histogram_by_bucket
            .get(bucket)
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| release_evidence_invalid(context, "stage_histogram_by_bucket_ms"))?;
        validate_current_stage_stage_histogram_summary(
            bucket_stages,
            sample_count,
            &STAGES,
            &HISTOGRAM_BOUNDS_MS,
            "stage_histogram_by_bucket_ms",
            context,
        )?;
    }

    let rss_delta = require_release_evidence_object(object, "rss_delta_mb", context)?;
    validate_current_stage_latency_observability(
        rss_delta,
        request_sample_count,
        "rss_delta_mb",
        context,
    )?;

    let rss_delta_by_bucket =
        require_release_evidence_object(object, "rss_delta_mb_by_bucket", context)?;
    validate_release_evidence_allowed_keys(rss_delta_by_bucket, &QUERY_BUCKETS, context)?;
    for bucket in QUERY_BUCKETS {
        let sample_count = require_release_evidence_positive_or_zero_u64_value(
            samples_per_bucket,
            bucket,
            context,
        )?;
        let Some(rss_delta) = rss_delta_by_bucket.get(bucket) else {
            return Err(release_evidence_invalid(context, "rss_delta_mb_by_bucket"));
        };
        let rss_delta = rss_delta
            .as_object()
            .ok_or_else(|| release_evidence_invalid(context, "rss_delta_mb_by_bucket"))?;
        validate_current_stage_latency_observability(
            rss_delta,
            sample_count,
            "rss_delta_mb_by_bucket",
            context,
        )?;
    }

    Ok(())
}

fn validate_current_stage_stage_histogram_summary(
    object: &serde_json::Map<String, serde_json::Value>,
    expected_samples: u64,
    stages: &[&str],
    histogram_bounds_ms: &[f64],
    field: &'static str,
    context: &'static str,
) -> Result<()> {
    validate_release_evidence_allowed_keys(object, stages, context)?;
    for stage in stages {
        let histogram = object
            .get(*stage)
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| release_evidence_invalid(context, field))?;
        validate_current_stage_histogram_observability(
            histogram,
            expected_samples,
            histogram_bounds_ms,
            field,
            context,
        )?;
    }
    Ok(())
}

fn validate_current_stage_histogram_observability(
    object: &serde_json::Map<String, serde_json::Value>,
    expected_samples: u64,
    histogram_bounds_ms: &[f64],
    field: &'static str,
    context: &'static str,
) -> Result<()> {
    validate_release_evidence_allowed_keys(
        object,
        &["samples", "bins", "overflow_count"],
        context,
    )?;
    let samples = require_release_evidence_positive_u64_value(object, "samples", context)?;
    if samples != expected_samples {
        return Err(release_evidence_invalid(context, field));
    }
    let bins = object
        .get("bins")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| release_evidence_invalid(context, field))?;
    if bins.len() != histogram_bounds_ms.len() {
        return Err(release_evidence_invalid(context, field));
    }

    let mut previous_count = 0;
    for (bin, expected_le_ms) in bins.iter().zip(histogram_bounds_ms) {
        let bin = bin
            .as_object()
            .ok_or_else(|| release_evidence_invalid(context, field))?;
        validate_release_evidence_allowed_keys(bin, &["le_ms", "count"], context)?;
        let le_ms = require_release_evidence_number_value(bin, "le_ms", context)?;
        let count = require_release_evidence_positive_or_zero_u64_value(bin, "count", context)?;
        if (le_ms - *expected_le_ms).abs() > f64::EPSILON
            || count < previous_count
            || count > samples
        {
            return Err(release_evidence_invalid(context, field));
        }
        previous_count = count;
    }

    let overflow_count =
        require_release_evidence_positive_or_zero_u64_value(object, "overflow_count", context)?;
    if previous_count.checked_add(overflow_count) == Some(samples) {
        Ok(())
    } else {
        Err(release_evidence_invalid(context, field))
    }
}

fn validate_current_stage_latency_observability(
    object: &serde_json::Map<String, serde_json::Value>,
    expected_samples: u64,
    field: &'static str,
    context: &'static str,
) -> Result<()> {
    validate_release_evidence_allowed_keys(object, &["samples", "p50", "p95", "p99"], context)?;
    let samples = require_release_evidence_positive_u64_value(object, "samples", context)?;
    let p50 = require_release_evidence_number_value(object, "p50", context)?;
    let p95 = require_release_evidence_number_value(object, "p95", context)?;
    let p99 = require_release_evidence_number_value(object, "p99", context)?;
    if samples == expected_samples && p50 <= p95 && p95 <= p99 {
        Ok(())
    } else {
        Err(release_evidence_invalid(context, field))
    }
}

fn require_release_evidence_positive_or_zero_u64_value(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    context: &'static str,
) -> Result<u64> {
    object
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| release_evidence_invalid(context, key))
}

fn validate_current_stage_aggregate_observability(
    value: &serde_json::Value,
    context: &'static str,
) -> Result<()> {
    match value {
        serde_json::Value::Object(object) => {
            for (key, nested) in object {
                if !is_current_stage_aggregate_key(key) {
                    return Err(release_evidence_invalid(
                        context,
                        "corpus_summary_observability",
                    ));
                }
                validate_current_stage_aggregate_observability(nested, context)?;
            }
            Ok(())
        }
        serde_json::Value::Number(number) if number.as_u64().is_some() => Ok(()),
        serde_json::Value::Bool(_) => Ok(()),
        serde_json::Value::String(value) if value == "redacted_local_aggregate" => Ok(()),
        serde_json::Value::Null => Ok(()),
        _ => Err(release_evidence_invalid(
            context,
            "corpus_summary_observability",
        )),
    }
}

fn is_current_stage_aggregate_key(key: &str) -> bool {
    !key.trim().is_empty()
        && key.len() <= 80
        && key.bytes().all(
            |byte| matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'),
        )
}

fn validate_release_evidence_version(version: &str, context: &'static str) -> Result<()> {
    let Some(raw_version) = version.strip_prefix('v') else {
        return Err(release_evidence_invalid(context, "version"));
    };
    let parts = raw_version.split('.').collect::<Vec<_>>();
    if parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        Ok(())
    } else {
        Err(release_evidence_invalid(context, "version"))
    }
}

fn is_release_evidence_basename(file: &str) -> bool {
    !file.trim().is_empty()
        && file != "."
        && file != ".."
        && !file.contains('/')
        && !file.contains('\\')
        && !file.contains(':')
}

fn release_evidence_invalid(context: &'static str, key: &str) -> CliError {
    CliError::user(format!("{context} blocked: {key} is invalid"))
}

fn read_release_readiness_evidence_report(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|_| CliError::user("unable to read release readiness evidence report"))
}

fn release_readiness_evidence_error(
    label: &'static str,
    error: benchmark_runner::BenchmarkGateError,
) -> CliError {
    CliError::user(format!(
        "release readiness evidence failed validation: {label}: {error}"
    ))
}

fn release_readiness_manifest_error(label: &'static str, error: CliError) -> CliError {
    CliError::user(format!(
        "release readiness evidence failed validation: {label}: {error}"
    ))
}

fn validate_release_readiness_diagnostics_report(report: &str) -> Result<()> {
    if release_readiness_diagnostics_report_contains_private_marker(report) {
        return Err(CliError::user(
            "diagnostics report blocked: private marker is present",
        ));
    }
    let value: serde_json::Value = serde_json::from_str(report)
        .map_err(|_| CliError::user("diagnostics report blocked: invalid JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| CliError::user("diagnostics report blocked: expected JSON object"))?;

    validate_release_readiness_diagnostics_allowed_keys(object)?;
    require_json_string(object, "schema_version", "diagnostics.v1")?;
    require_json_bool(object, "redacted", true)?;
    require_json_string(object, "raw_paths", "<redacted>")?;
    require_json_string(object, "raw_queries", "<redacted>")?;
    require_json_string(object, "raw_resume_text", "<redacted>")?;
    require_json_string(object, "evidence_level", "local_aggregate_only")?;
    require_optional_nested_redacted(object, "resource_telemetry", "paths")?;
    require_optional_nested_redacted(object, "ocr_runtime", "paths")?;
    require_optional_nested_redacted(object, "query_latency", "raw_queries")?;

    let diagnostic_scope = object
        .get("diagnostic_scope")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| CliError::user("diagnostics report blocked: diagnostic scope missing"))?;
    for (key, expected) in [
        ("metadata", "aggregate_counts"),
        ("search_index", "state_and_snapshot_health"),
        ("vector_index", "state_backend_and_counts"),
        ("query_latency", "aggregate_observations"),
        ("runtime_dependencies", "presence_only"),
        ("fault_simulations", "available_cases_only"),
    ] {
        require_json_string(diagnostic_scope, key, expected)?;
    }

    Ok(())
}

fn validate_release_readiness_diagnostics_allowed_keys(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    const ALLOWED_KEYS: &[&str] = &[
        "schema_version",
        "redacted",
        "raw_paths",
        "raw_queries",
        "raw_resume_text",
        "metadata",
        "search_index_state",
        "vector_index_state",
        "vector_index_backend",
        "vector_index_vectors",
        "vector_index_tombstones",
        "search_index_read_target",
        "index_health",
        "last_snapshot",
        "staging_orphans",
        "snapshot_fallback",
        "query_smoke",
        "query_latency",
        "contact_hash_key",
        "resource_telemetry",
        "ocr_runtime",
        "fault_simulations",
        "diagnostic_scope",
        "evidence_level",
        "scope",
    ];

    for key in object.keys() {
        if !ALLOWED_KEYS.contains(&key.as_str()) {
            return Err(CliError::user(format!(
                "diagnostics report blocked: {key} is not allowed"
            )));
        }
    }

    Ok(())
}

fn require_json_bool(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    expected: bool,
) -> Result<()> {
    match object.get(key).and_then(serde_json::Value::as_bool) {
        Some(value) if value == expected => Ok(()),
        _ => Err(CliError::user(format!(
            "diagnostics report blocked: {key} is invalid"
        ))),
    }
}

fn require_json_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    expected: &str,
) -> Result<()> {
    match object.get(key).and_then(serde_json::Value::as_str) {
        Some(value) if value == expected => Ok(()),
        _ => Err(CliError::user(format!(
            "diagnostics report blocked: {key} is invalid"
        ))),
    }
}

fn require_optional_nested_redacted(
    object: &serde_json::Map<String, serde_json::Value>,
    parent: &str,
    key: &str,
) -> Result<()> {
    let Some(parent_value) = object.get(parent) else {
        return Ok(());
    };
    let parent_object = parent_value.as_object().ok_or_else(|| {
        CliError::user(format!("diagnostics report blocked: {parent} is invalid"))
    })?;
    require_json_string(parent_object, key, "<redacted>")
}

fn require_release_json_string(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    expected: &str,
) -> Result<()> {
    match object.get(key).and_then(serde_json::Value::as_str) {
        Some(value) if value == expected => Ok(()),
        _ => Err(CliError::user(format!(
            "release automation evidence blocked: {key} is invalid"
        ))),
    }
}

fn require_release_json_sha256(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<()> {
    let is_sha256 = object
        .get(key)
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        });
    if is_sha256 {
        Ok(())
    } else {
        Err(CliError::user(format!(
            "release automation evidence blocked: {key} is invalid"
        )))
    }
}

fn require_release_json_non_empty_array(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<()> {
    match object.get(key).and_then(serde_json::Value::as_array) {
        Some(values) if !values.is_empty() => Ok(()),
        _ => Err(CliError::user(format!(
            "release automation evidence blocked: {key} is invalid"
        ))),
    }
}

fn require_release_blocked_planned_actions(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<()> {
    let actions = object
        .get("planned_actions")
        .and_then(serde_json::Value::as_array)
        .filter(|actions| !actions.is_empty())
        .ok_or_else(|| {
            CliError::user("release automation evidence blocked: planned_actions is invalid")
        })?;
    for action in actions {
        let Some(action_object) = action.as_object() else {
            return Err(CliError::user(
                "release automation evidence blocked: planned_actions is invalid",
            ));
        };
        require_release_json_string(action_object, "action_status", "blocked")?;
    }
    Ok(())
}

fn release_readiness_diagnostics_report_contains_private_marker(report: &str) -> bool {
    let markers = [
        ["/Users", "/"].concat(),
        ["/home", "/"].concat(),
        ["/private", "/"].concat(),
        ["\\Users", "\\"].concat(),
        [":", "\\"].concat(),
        ["BEGIN ", "PRIVATE", " KEY"].concat(),
        ["gh", "p_"].concat(),
        ["github", "_pat_"].concat(),
        ["s", "k-"].concat(),
        ["h", "f_"].concat(),
    ];
    markers.iter().any(|marker| report.contains(marker))
}

fn validate_release_readiness_ocr_manifest_coverage(
    validation: &OcrRuntimeManifestValidation,
) -> Result<()> {
    let has_engine = validation
        .components
        .iter()
        .any(|component| component.kind == "ocr-engine");
    let has_renderer = validation
        .components
        .iter()
        .any(|component| component.kind == "pdf-renderer");
    let has_language_pack = !validation.languages.is_empty()
        || validation
            .components
            .iter()
            .any(|component| component.kind == "ocr-language-pack");

    if !has_engine || !has_renderer || !has_language_pack {
        return Err(release_readiness_manifest_error(
            RELEASE_READINESS_OCR_LICENSE_LABEL,
            CliError::user(
                "ocr runtime manifest blocked: engine, renderer, and language-pack evidence required",
            ),
        ));
    }

    Ok(())
}

fn candidate_review_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(CliError::usage(candidate_review_usage()));
    };
    let store = ReadMetaStore::open_data_dir(data_dir).map_err(CliError::store)?;

    match action {
        "list" => candidate_review_list_command(&store, &args[1..]),
        "conflicts" => candidate_review_conflicts_command(&store, &args[1..]),
        _ => Err(CliError::usage(candidate_review_usage())),
    }
}

fn candidate_review_list_command(store: &ReadMetaStore, args: &[String]) -> Result<()> {
    let review_args = parse_candidate_review_list_args(args)?;
    let suggestions = candidate_review_suggestions(store, review_args.limit)?;

    println!("candidate review suggestions: {}", suggestions.len());
    for (index, suggestion) in suggestions.iter().enumerate() {
        println!("suggestion: {}", index + 1);
        println!("versions: 2");
        println!(
            "version_ids: {} {}",
            suggestion.left_version_id, suggestion.right_version_id
        );
        println!("confidence: {:.2}", suggestion.confidence);
        println!("folded: false");
        println!("paths: <redacted>");
    }

    Ok(())
}

fn candidate_review_conflicts_command(store: &ReadMetaStore, args: &[String]) -> Result<()> {
    let review_args = parse_candidate_review_list_args(args)?;
    let mut conflicts = store
        .candidate_contact_conflicts()
        .map_err(CliError::store)?;
    conflicts.truncate(review_args.limit);

    println!("candidate contact conflicts: {}", conflicts.len());
    for (index, conflict) in conflicts.iter().enumerate() {
        println!("conflict: {}", index + 1);
        println!("version_id: {}", conflict.resume_version_id);
        println!("email_candidate_id: {}", conflict.email_candidate_id);
        println!("phone_candidate_id: {}", conflict.phone_candidate_id);
        println!("contact_values: <redacted>");
        println!("contact_hashes: <redacted>");
        println!("paths: <redacted>");
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct CandidateReviewListArgs {
    limit: usize,
}

#[derive(Debug, PartialEq)]
struct CandidateReviewSuggestion {
    left_version_id: ResumeVersionId,
    right_version_id: ResumeVersionId,
    confidence: f32,
}

struct CandidateReviewProfile {
    document_id: DocumentId,
    version_id: ResumeVersionId,
    profile: DedupeProfile,
}

fn parse_candidate_review_list_args(args: &[String]) -> Result<CandidateReviewListArgs> {
    let mut limit = 20_usize;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--limit" => {
                limit = parse_candidate_review_positive_usize(take_candidate_review_value(
                    args, &mut index,
                )?)?;
            }
            _ => return Err(CliError::usage(candidate_review_usage())),
        }
    }

    Ok(CandidateReviewListArgs { limit })
}

fn parse_candidate_review_positive_usize(value: &str) -> Result<usize> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| CliError::usage(candidate_review_usage()))?;
    if parsed == 0 {
        return Err(CliError::usage(candidate_review_usage()));
    }
    Ok(parsed)
}

fn take_candidate_review_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str> {
    *index += 1;
    let Some(value) = args.get(*index) else {
        return Err(CliError::usage(candidate_review_usage()));
    };
    *index += 1;
    Ok(value)
}

fn candidate_review_suggestions(
    store: &ReadMetaStore,
    limit: usize,
) -> Result<Vec<CandidateReviewSuggestion>> {
    let mut profiles_by_name = BTreeMap::<String, Vec<CandidateReviewProfile>>::new();

    for document in store.visible_documents().map_err(CliError::store)? {
        if document.is_deleted
            || !matches!(
                document.status,
                DocumentStatus::Searchable | DocumentStatus::IndexedPartial
            )
        {
            continue;
        }
        let Some(projection) = store
            .active_search_projection_for_document(&document.id)
            .map_err(CliError::store)?
        else {
            continue;
        };
        let Some(version) = store
            .resume_version_by_id(&projection.resume_version_id)
            .map_err(CliError::store)?
        else {
            return Err(CliError::user("active search projection is invalid"));
        };
        if store
            .candidate_assignment_for_version(&version.id)
            .map_err(CliError::store)?
            .is_some()
        {
            continue;
        }
        let Some(profile) = dedupe_profile_for_review_version(store, &document.id, &version)?
        else {
            continue;
        };
        let Some(name) = profile.name().map(str::to_string) else {
            continue;
        };
        profiles_by_name
            .entry(name)
            .or_default()
            .push(CandidateReviewProfile {
                document_id: document.id.clone(),
                version_id: version.id,
                profile,
            });
    }

    let mut suggestions = Vec::new();
    for profiles in profiles_by_name.values() {
        for left_index in 0..profiles.len() {
            for right_index in (left_index + 1)..profiles.len() {
                let left = &profiles[left_index];
                let right = &profiles[right_index];
                if left.document_id == right.document_id {
                    continue;
                }
                let Some(score) = soft_dedupe_score(&left.profile, &right.profile) else {
                    continue;
                };
                let (left_version_id, right_version_id) =
                    ordered_version_pair(&left.version_id, &right.version_id);
                suggestions.push(CandidateReviewSuggestion {
                    left_version_id,
                    right_version_id,
                    confidence: score.confidence(),
                });
            }
        }
    }

    suggestions.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.left_version_id.cmp(&right.left_version_id))
            .then_with(|| left.right_version_id.cmp(&right.right_version_id))
    });
    suggestions.truncate(limit);
    Ok(suggestions)
}

fn dedupe_profile_for_review_version(
    store: &ReadMetaStore,
    document_id: &DocumentId,
    version: &ResumeVersion,
) -> Result<Option<DedupeProfile>> {
    if &version.document_id != document_id {
        return Ok(None);
    }
    let mentions = store
        .entity_mentions_for_version(&version.id)
        .map_err(CliError::store)?;
    let Some(name) = best_normalized_entity_value(&mentions, EntityType::Name) else {
        return Ok(None);
    };

    Ok(Some(
        DedupeProfile::new(document_id.to_string())
            .with_name(&name)
            .with_schools(normalized_entity_values(&mentions, EntityType::School))
            .with_companies(normalized_entity_values(&mentions, EntityType::Company))
            .with_skills(normalized_entity_values(&mentions, EntityType::Skill)),
    ))
}

fn best_normalized_entity_value(
    mentions: &[EntityMention],
    entity_type: EntityType,
) -> Option<String> {
    mentions
        .iter()
        .filter(|mention| {
            mention.entity_type == entity_type
                && mention.confidence >= FIELD_FILTER_CONFIDENCE_THRESHOLD
        })
        .filter_map(|mention| {
            Some((
                mention.normalized_value.as_deref()?.to_string(),
                mention.confidence,
                mention.span_start.unwrap_or(usize::MAX),
            ))
        })
        .max_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| right.0.cmp(&left.0))
        })
        .map(|candidate| candidate.0)
}

fn normalized_entity_values(mentions: &[EntityMention], entity_type: EntityType) -> Vec<String> {
    mentions
        .iter()
        .filter(|mention| {
            mention.entity_type == entity_type
                && mention.confidence >= FIELD_FILTER_CONFIDENCE_THRESHOLD
        })
        .filter_map(|mention| mention.normalized_value.as_deref())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn ordered_version_pair(
    left: &ResumeVersionId,
    right: &ResumeVersionId,
) -> (ResumeVersionId, ResumeVersionId) {
    if left <= right {
        (left.clone(), right.clone())
    } else {
        (right.clone(), left.clone())
    }
}

fn candidate_review_usage() -> &'static str {
    "usage: resume-cli candidate-review <list --limit <count>|conflicts --limit <count>>"
}

fn take_data_dir(args: &mut Vec<String>) -> Result<PathBuf> {
    if args.first().map(String::as_str) != Some("--data-dir") {
        return Ok(PathBuf::from("local-data"));
    }

    if args.len() < 2 {
        return Err(CliError::usage(
            "usage: resume-cli --data-dir <path> <command>",
        ));
    }

    let path = PathBuf::from(args.remove(1));
    args.remove(0);
    Ok(path)
}

fn model_command(args: &[String]) -> Result<()> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(CliError::usage(model_usage()));
    };

    match action {
        "draft-manifest" => model_draft_manifest_command(&args[1..]),
        "preflight" => model_preflight_command(&args[1..]),
        "validate-manifest" => model_validate_manifest_command(&args[1..]),
        _ => Err(CliError::usage(model_usage())),
    }
}

fn ocr_command(args: &[String]) -> Result<()> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(CliError::usage(ocr_usage()));
    };

    match action {
        "draft-manifest" => ocr_draft_manifest_command(&args[1..]),
        "preflight" => ocr_preflight_command(&args[1..]),
        "validate-manifest" => ocr_validate_manifest_command(&args[1..]),
        _ => Err(CliError::usage(ocr_usage())),
    }
}

fn privacy_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(CliError::usage(privacy_usage()));
    };

    match action {
        "dataset-manifest" => privacy_dataset_manifest_command(&args[1..]),
        "backup-contact-key" => {
            let key_args = parse_privacy_key_file_args(&args[1..], "--output")?;
            let passphrase = read_privacy_passphrase_file(&key_args.passphrase_path)?;
            backup_contact_hash_key(data_dir, &key_args.key_path, &passphrase)
                .map_err(CliError::privacy)?;
            println!("contact hash key backup: written");
            Ok(())
        }
        "restore-contact-key" => {
            let key_args = parse_privacy_key_file_args(&args[1..], "--input")?;
            let passphrase = read_privacy_passphrase_file(&key_args.passphrase_path)?;
            let _owner = import_processing::acquire_owner(data_dir)?;
            restore_contact_hash_key(data_dir, &key_args.key_path, &passphrase)
                .map_err(CliError::privacy)?;
            println!("contact hash key restore: restored");
            Ok(())
        }
        "backup-metadata-key" => {
            let key_args = parse_privacy_key_file_args(&args[1..], "--output")?;
            let passphrase = read_privacy_passphrase_file(&key_args.passphrase_path)?;
            backup_metadata_encryption_key(data_dir, &key_args.key_path, &passphrase)
                .map_err(CliError::store)?;
            println!("metadata encryption key backup: written");
            Ok(())
        }
        "restore-metadata-key" => {
            let key_args = parse_privacy_key_file_args(&args[1..], "--input")?;
            let passphrase = read_privacy_passphrase_file(&key_args.passphrase_path)?;
            let owner = import_processing::acquire_owner(data_dir)?;
            restore_metadata_encryption_key(&owner, &key_args.key_path, &passphrase)
                .map_err(CliError::store)?;
            println!("metadata encryption key restore: restored");
            Ok(())
        }
        "rotate-metadata-key" => {
            if args.len() != 1 {
                return Err(CliError::usage(privacy_usage()));
            }
            let owner = import_processing::acquire_owner(data_dir)?;
            owner
                .open_store()
                .map_err(CliError::store)?
                .rotate_metadata_encryption_key()
                .map_err(CliError::store)?;
            println!("metadata encryption key rotation: rotated");
            Ok(())
        }
        _ => Err(CliError::usage(privacy_usage())),
    }
}

struct PrivacyDatasetManifestArgs {
    root: PathBuf,
    out: PathBuf,
    profile: fs_crawler::ScanProfile,
    max_files: Option<usize>,
}

fn privacy_dataset_manifest_command(args: &[String]) -> Result<()> {
    let args = parse_privacy_dataset_manifest_args(args)?;
    let report = crawl_directory_with_options(
        &args.root,
        CrawlerScanOptions {
            profile: args.profile,
            max_files: args.max_files,
        },
    )
    .map_err(|_| CliError::user("dataset manifest blocked: root must exist and be readable"))?;
    let manifest = redacted_dataset_manifest(&report, args.profile, args.max_files);
    let manifest_text = serde_json::to_string_pretty(&manifest)
        .map_err(|_| CliError::user("dataset manifest blocked: manifest is unavailable"))?;

    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|_| CliError::user("dataset manifest blocked: output is unavailable"))?;
        }
    }
    fs::write(&args.out, format!("{manifest_text}\n"))
        .map_err(|_| CliError::user("dataset manifest blocked: output is unavailable"))?;
    let manifest_sha256 = file_sha256_hex(&args.out)
        .map_err(|_| CliError::user("dataset manifest blocked: checksum unavailable"))?;

    println!("dataset manifest: written");
    println!("schema: {DATASET_MANIFEST_SCHEMA_VERSION}");
    println!("privacy boundary: local_only_redacted_dataset_manifest");
    println!("files: {}", report.files.len());
    println!("manifest sha256: {manifest_sha256}");
    println!("paths: <redacted>");
    Ok(())
}

fn parse_privacy_dataset_manifest_args(args: &[String]) -> Result<PrivacyDatasetManifestArgs> {
    let mut root = None;
    let mut out = None;
    let mut profile = fs_crawler::ScanProfile::Explicit;
    let mut profile_seen = false;
    let mut max_files = None;
    let mut index = 0_usize;
    while index < args.len() {
        match args[index].as_str() {
            "--root" => root = Some(take_privacy_path_arg(args, &mut index, root.is_some())?),
            "--out" => out = Some(take_privacy_path_arg(args, &mut index, out.is_some())?),
            "--profile" => {
                if profile_seen {
                    return Err(CliError::usage(privacy_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(privacy_usage()));
                };
                profile = match value.as_str() {
                    "explicit" => fs_crawler::ScanProfile::Explicit,
                    "discovery" => fs_crawler::ScanProfile::Discovery,
                    _ => return Err(CliError::usage(privacy_usage())),
                };
                profile_seen = true;
                index += 2;
            }
            "--max-files" => {
                if max_files.is_some() {
                    return Err(CliError::usage(privacy_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(privacy_usage()));
                };
                max_files = Some(parse_privacy_positive_usize(value)?);
                index += 2;
            }
            _ => return Err(CliError::usage(privacy_usage())),
        }
    }

    Ok(PrivacyDatasetManifestArgs {
        root: root.ok_or_else(|| CliError::usage(privacy_usage()))?,
        out: out.ok_or_else(|| CliError::usage(privacy_usage()))?,
        profile,
        max_files,
    })
}

fn take_privacy_path_arg(args: &[String], index: &mut usize, duplicate: bool) -> Result<PathBuf> {
    if duplicate {
        return Err(CliError::usage(privacy_usage()));
    }
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(privacy_usage()));
    };
    if value.trim().is_empty() {
        return Err(CliError::usage(privacy_usage()));
    }
    *index += 2;
    Ok(PathBuf::from(value))
}

fn parse_privacy_positive_usize(value: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .ok()
        .filter(|parsed| *parsed > 0)
        .ok_or_else(|| CliError::usage(privacy_usage()))
}

fn redacted_dataset_manifest(
    report: &fs_crawler::ScanReport,
    profile: fs_crawler::ScanProfile,
    max_files: Option<usize>,
) -> serde_json::Value {
    let mut extension_counts = BTreeMap::<String, usize>::new();
    let mut total_bytes = 0_u64;
    let mut readonly_file_count = 0_usize;
    let mut fingerprint_sampled_bytes = 0_u64;
    let mut corpus_hasher = Sha256::new();
    update_sha256_string(&mut corpus_hasher, DATASET_MANIFEST_SCHEMA_VERSION);
    update_sha256_string(&mut corpus_hasher, profile.label());

    for file in &report.files {
        let extension = dataset_file_extension_label(&file.extension);
        *extension_counts.entry(extension.to_string()).or_insert(0) += 1;
        total_bytes = total_bytes.saturating_add(file.byte_size);
        fingerprint_sampled_bytes =
            fingerprint_sampled_bytes.saturating_add(file.fingerprint.sampled_bytes);
        if file.permissions.readonly {
            readonly_file_count += 1;
        }
        update_sha256_string(&mut corpus_hasher, extension);
        update_sha256_u64(&mut corpus_hasher, file.byte_size);
        update_sha256_i64(&mut corpus_hasher, file.mtime.as_unix_seconds());
        update_sha256_string(&mut corpus_hasher, file.fingerprint.as_str());
    }

    let scan_budget = report
        .budget_exhausted
        .map(|budget| {
            serde_json::json!({
                "exhausted": true,
                "kind": "files",
                "limit": budget.limit,
                "observed": budget.observed,
            })
        })
        .unwrap_or_else(|| {
            serde_json::json!({
                "exhausted": false,
                "kind": serde_json::Value::Null,
                "limit": max_files,
                "observed": report.files.len(),
            })
        });

    serde_json::json!({
        "schema_version": DATASET_MANIFEST_SCHEMA_VERSION,
        "privacy_boundary": "local_only_redacted_dataset_manifest",
        "dataset_kind": "private-local-corpus",
        "scan_profile": profile.label(),
        "max_files": max_files,
        "file_count": report.files.len(),
        "ignored_entries": report.ignored_count,
        "scan_error_count": report.errors.len(),
        "scanned_directory_count": report.scanned_directories.len(),
        "skipped_directory_count": report.skipped_directories.len(),
        "scan_budget": scan_budget,
        "total_bytes": total_bytes,
        "readonly_file_count": readonly_file_count,
        "fingerprint_sampled_bytes": fingerprint_sampled_bytes,
        "supported_extensions": ["doc", "docx", "pdf", "txt"],
        "extension_counts": extension_counts,
        "corpus_fingerprint_sha256": hex_encode_lower(&corpus_hasher.finalize()),
        "contains_paths": false,
        "contains_file_names": false,
        "contains_raw_resume_text": false,
        "contains_file_hashes": false,
        "must_not_upload": [
            "raw resumes",
            "local paths",
            "file names",
            "raw resume text",
            "per-file hashes",
            "diagnostic packages",
            "indexes",
            "SQLite databases"
        ]
    })
}

fn dataset_file_extension_label(extension: &FileExtension) -> &'static str {
    match extension {
        FileExtension::Doc => "doc",
        FileExtension::Docx => "docx",
        FileExtension::Pdf => "pdf",
        FileExtension::Txt => "txt",
        FileExtension::Image => "image",
        FileExtension::Other(_) => "other",
    }
}

struct PrivacyKeyFileArgs {
    key_path: PathBuf,
    passphrase_path: PathBuf,
}

fn parse_privacy_key_file_args(
    args: &[String],
    key_flag: &'static str,
) -> Result<PrivacyKeyFileArgs> {
    let mut key_path = None;
    let mut passphrase_path = None;
    let mut index = 0;
    while index < args.len() {
        if index + 1 >= args.len() || args[index + 1].is_empty() {
            return Err(CliError::usage(privacy_usage()));
        }

        match args[index].as_str() {
            flag if flag == key_flag => {
                if key_path.replace(PathBuf::from(&args[index + 1])).is_some() {
                    return Err(CliError::usage(privacy_usage()));
                }
            }
            "--passphrase-file" => {
                if passphrase_path
                    .replace(PathBuf::from(&args[index + 1]))
                    .is_some()
                {
                    return Err(CliError::usage(privacy_usage()));
                }
            }
            _ => return Err(CliError::usage(privacy_usage())),
        }

        index += 2;
    }

    let Some(key_path) = key_path else {
        return Err(CliError::usage(privacy_usage()));
    };
    let Some(passphrase_path) = passphrase_path else {
        return Err(CliError::usage(privacy_usage()));
    };

    Ok(PrivacyKeyFileArgs {
        key_path,
        passphrase_path,
    })
}

fn read_privacy_passphrase_file(path: &Path) -> Result<Vec<u8>> {
    let mut passphrase =
        fs::read(path).map_err(|_| CliError::user("privacy passphrase file could not be read"))?;
    while matches!(passphrase.last(), Some(b'\n' | b'\r')) {
        passphrase.pop();
    }

    Ok(passphrase)
}

fn privacy_usage() -> &'static str {
    "usage: resume-cli privacy dataset-manifest --root <path> --out <path> [--profile explicit|discovery] [--max-files <count>] | resume-cli privacy backup-contact-key --output <path> --passphrase-file <path> | resume-cli privacy restore-contact-key --input <path> --passphrase-file <path> | resume-cli privacy backup-metadata-key --output <path> --passphrase-file <path> | resume-cli privacy restore-metadata-key --input <path> --passphrase-file <path> | resume-cli privacy rotate-metadata-key"
}

fn model_validate_manifest_command(args: &[String]) -> Result<()> {
    let manifest_path = parse_model_validate_manifest_args(args)?;
    let validation = validate_model_manifest(&manifest_path)?;

    println!("model manifest: valid");
    println!("model pack: {}", validation.model_pack_id);
    println!("models: {}", validation.models.len());
    for model in &validation.models {
        println!("model id: {}", model.model_id);
        println!("type: {}", model.model_type);
        if let Some(dimension) = model.dimension {
            println!("dimension: {dimension}");
        }
        println!("license reviewed: yes");
        println!("checksum match: yes");
        println!("sha256 prefix: {}", checksum_prefix(&model.sha256));
    }
    println!("paths: <redacted>");
    Ok(())
}

#[derive(Debug, Clone)]
struct ModelDraftManifestArgs {
    out: PathBuf,
    model_pack_id: String,
    model_id: String,
    model_type: String,
    dimension: Option<usize>,
    format: String,
    artifact: PathBuf,
    license: String,
    reviewed: bool,
}

fn model_draft_manifest_command(args: &[String]) -> Result<()> {
    let draft_args = parse_model_draft_manifest_args(args)?;
    if !draft_args.artifact.is_file() {
        return Err(CliError::user(
            "model manifest draft blocked: artifact is unavailable",
        ));
    }
    let artifact_sha256 = file_sha256_hex(&draft_args.artifact)
        .map_err(|_| CliError::user("model manifest draft blocked: checksum unavailable"))?;
    let mut model = serde_json::json!({
        "id": draft_args.model_id,
        "type": draft_args.model_type,
        "format": draft_args.format,
        "artifact": {
            "path": path_string_lossless(&draft_args.artifact)?,
            "sha256": artifact_sha256
        },
        "license": {
            "id": draft_args.license,
            "reviewed": draft_args.reviewed
        }
    });
    if let Some(dimension) = draft_args.dimension {
        model
            .as_object_mut()
            .expect("model draft is an object")
            .insert("dim".to_string(), serde_json::json!(dimension));
    }
    let manifest = serde_json::json!({
        "schema_version": MODEL_MANIFEST_SCHEMA_VERSION,
        "model_pack_id": draft_args.model_pack_id,
        "models": [model]
    });
    let manifest_text = serde_json::to_string_pretty(&manifest)
        .map_err(|_| CliError::user("model manifest draft blocked: invalid manifest"))?;
    if let Some(parent) = draft_args.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|_| {
                CliError::user("model manifest draft blocked: output is unavailable")
            })?;
        }
    }
    fs::write(&draft_args.out, format!("{manifest_text}\n"))
        .map_err(|_| CliError::user("model manifest draft blocked: output is unavailable"))?;

    println!("model manifest draft: written");
    println!("schema: {MODEL_MANIFEST_SCHEMA_VERSION}");
    println!(
        "model pack: {}",
        manifest["model_pack_id"].as_str().unwrap_or("unknown")
    );
    println!(
        "model id: {}",
        manifest["models"][0]["id"].as_str().unwrap_or("unknown")
    );
    println!(
        "type: {}",
        manifest["models"][0]["type"].as_str().unwrap_or("unknown")
    );
    if let Some(dimension) = manifest["models"][0]["dim"].as_u64() {
        println!("dimension: {dimension}");
    }
    println!(
        "license reviewed: {}",
        if draft_args.reviewed { "yes" } else { "no" }
    );
    println!("paths: <redacted>");
    Ok(())
}

fn parse_model_draft_manifest_args(args: &[String]) -> Result<ModelDraftManifestArgs> {
    let mut out = None;
    let mut model_pack_id = None;
    let mut model_id = None;
    let mut model_type = None;
    let mut dimension = None;
    let mut format = None;
    let mut artifact = None;
    let mut license = None;
    let mut reviewed = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => out = Some(take_model_path_arg(args, &mut index, out.is_some())?),
            "--model-pack-id" => {
                model_pack_id = Some(take_model_identifier_arg(
                    args,
                    &mut index,
                    model_pack_id.is_some(),
                )?)
            }
            "--model-id" => {
                model_id = Some(take_model_identifier_arg(
                    args,
                    &mut index,
                    model_id.is_some(),
                )?)
            }
            "--model-type" => {
                model_type = Some(take_model_identifier_arg(
                    args,
                    &mut index,
                    model_type.is_some(),
                )?)
            }
            "--dimension" => {
                dimension = Some(take_model_dimension_arg(
                    args,
                    &mut index,
                    dimension.is_some(),
                )?)
            }
            "--format" => {
                format = Some(take_model_identifier_arg(
                    args,
                    &mut index,
                    format.is_some(),
                )?)
            }
            "--artifact" => {
                artifact = Some(take_model_path_arg(args, &mut index, artifact.is_some())?)
            }
            "--license" => {
                license = Some(take_model_license_arg(args, &mut index, license.is_some())?)
            }
            "--reviewed" => {
                if reviewed {
                    return Err(CliError::usage(model_usage()));
                }
                reviewed = true;
                index += 1;
            }
            _ => return Err(CliError::usage(model_usage())),
        }
    }

    let model_type = model_type.ok_or_else(|| CliError::usage(model_usage()))?;
    if !matches!(model_type.as_str(), "embedding" | "ner" | "ocr") {
        return Err(CliError::usage(model_usage()));
    }
    if model_type == "embedding" && dimension.is_none() {
        return Err(CliError::usage(model_usage()));
    }
    if model_type != "embedding" && dimension.is_some() {
        return Err(CliError::usage(model_usage()));
    }

    Ok(ModelDraftManifestArgs {
        out: out.ok_or_else(|| CliError::usage(model_usage()))?,
        model_pack_id: model_pack_id.ok_or_else(|| CliError::usage(model_usage()))?,
        model_id: model_id.ok_or_else(|| CliError::usage(model_usage()))?,
        model_type,
        dimension,
        format: format.ok_or_else(|| CliError::usage(model_usage()))?,
        artifact: artifact.ok_or_else(|| CliError::usage(model_usage()))?,
        license: license.ok_or_else(|| CliError::usage(model_usage()))?,
        reviewed,
    })
}

fn take_model_path_arg(args: &[String], index: &mut usize, duplicate: bool) -> Result<PathBuf> {
    if duplicate {
        return Err(CliError::usage(model_usage()));
    }
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(model_usage()));
    };
    if value.trim().is_empty() {
        return Err(CliError::usage(model_usage()));
    }
    *index += 2;
    Ok(PathBuf::from(value))
}

fn take_model_identifier_arg(
    args: &[String],
    index: &mut usize,
    duplicate: bool,
) -> Result<String> {
    if duplicate {
        return Err(CliError::usage(model_usage()));
    }
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(model_usage()));
    };
    if !valid_model_manifest_identifier(value) {
        return Err(CliError::usage(model_usage()));
    }
    *index += 2;
    Ok(value.clone())
}

fn take_model_license_arg(args: &[String], index: &mut usize, duplicate: bool) -> Result<String> {
    if duplicate {
        return Err(CliError::usage(model_usage()));
    }
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(model_usage()));
    };
    if !valid_license_expression(value) {
        return Err(CliError::usage(model_usage()));
    }
    *index += 2;
    Ok(value.clone())
}

fn take_model_dimension_arg(args: &[String], index: &mut usize, duplicate: bool) -> Result<usize> {
    if duplicate {
        return Err(CliError::usage(model_usage()));
    }
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(model_usage()));
    };
    let dimension = value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CliError::usage(model_usage()))?;
    *index += 2;
    Ok(dimension)
}

#[derive(Debug, Clone)]
struct ModelPreflightArgs {
    manifest_path: PathBuf,
    embedding_command: PathBuf,
    model_id: String,
    dimension: usize,
}

fn model_preflight_command(args: &[String]) -> Result<()> {
    let preflight_args = parse_model_preflight_args(args)?;
    let validation = validate_model_manifest(&preflight_args.manifest_path)?;
    let model = validation
        .models
        .iter()
        .find(|model| {
            model.model_id == preflight_args.model_id
                && model.model_type == "embedding"
                && model.dimension == Some(preflight_args.dimension)
        })
        .ok_or_else(|| {
            CliError::user(
                "embedding runtime preflight blocked: reviewed embedding model is not present",
            )
        })?;
    let command_available = is_executable_file(&preflight_args.embedding_command);
    let protocol_status = if command_available {
        model_preflight_protocol_status(&preflight_args)
    } else {
        EmbeddingPreflightProtocolStatus::NotRun
    };
    print_model_preflight_json(model, command_available, protocol_status);
    if command_available && protocol_status == EmbeddingPreflightProtocolStatus::Passed {
        Ok(())
    } else {
        Err(CliError::user(
            "embedding runtime preflight blocked: dependencies are not ready",
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmbeddingPreflightProtocolStatus {
    Passed,
    Failed,
    NotRun,
}

impl EmbeddingPreflightProtocolStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotRun => "not_run",
        }
    }
}

fn model_preflight_protocol_status(args: &ModelPreflightArgs) -> EmbeddingPreflightProtocolStatus {
    let spec = match LocalEmbeddingCommandSpec::new(
        args.embedding_command.clone(),
        Vec::<String>::new(),
        args.model_id.clone(),
        args.dimension,
    ) {
        Ok(spec) => spec,
        Err(_) => return EmbeddingPreflightProtocolStatus::Failed,
    };
    let embedder = LocalEmbeddingCommandEmbedder::new(spec);
    let input = EmbeddingInput::new(
        "preflight",
        "resume-ir synthetic embedding runtime preflight",
    );
    match embedder.embed_batch(&[input], EmbeddingBudget::new(1, 4096)) {
        Ok(vectors)
            if vectors.len() == 1
                && vectors[0].id() == "preflight"
                && vectors[0].model_id() == args.model_id
                && vectors[0].values().len() == args.dimension =>
        {
            EmbeddingPreflightProtocolStatus::Passed
        }
        _ => EmbeddingPreflightProtocolStatus::Failed,
    }
}

fn parse_model_preflight_args(args: &[String]) -> Result<ModelPreflightArgs> {
    let mut json = false;
    let mut manifest_path = None;
    let mut embedding_command = None;
    let mut model_id = None;
    let mut dimension = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                if json {
                    return Err(CliError::usage(model_usage()));
                }
                json = true;
                index += 1;
            }
            "--manifest" => {
                if manifest_path.is_some() {
                    return Err(CliError::usage(model_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(model_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(model_usage()));
                }
                manifest_path = Some(PathBuf::from(value));
                index += 2;
            }
            "--embedding-command" => {
                if embedding_command.is_some() {
                    return Err(CliError::usage(model_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(model_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(model_usage()));
                }
                embedding_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--model-id" => {
                if model_id.is_some() {
                    return Err(CliError::usage(model_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(model_usage()));
                };
                if !valid_model_manifest_identifier(value) {
                    return Err(CliError::usage(model_usage()));
                }
                model_id = Some(value.clone());
                index += 2;
            }
            "--dimension" => {
                if dimension.is_some() {
                    return Err(CliError::usage(model_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(model_usage()));
                };
                dimension = Some(
                    value
                        .parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| CliError::usage(model_usage()))?,
                );
                index += 2;
            }
            _ => return Err(CliError::usage(model_usage())),
        }
    }

    if !json {
        return Err(CliError::usage(model_usage()));
    }

    Ok(ModelPreflightArgs {
        manifest_path: manifest_path.ok_or_else(|| CliError::usage(model_usage()))?,
        embedding_command: embedding_command.ok_or_else(|| CliError::usage(model_usage()))?,
        model_id: model_id.ok_or_else(|| CliError::usage(model_usage()))?,
        dimension: dimension.ok_or_else(|| CliError::usage(model_usage()))?,
    })
}

fn print_model_preflight_json(
    model: &ModelManifestModelValidation,
    command_available: bool,
    protocol_status: EmbeddingPreflightProtocolStatus,
) {
    println!("{{");
    println!("  \"schema_version\": \"embedding-runtime-preflight.v1\",");
    println!(
        "  \"runtime_status\": \"{}\",",
        if command_available && protocol_status == EmbeddingPreflightProtocolStatus::Passed {
            "ready"
        } else {
            "blocked"
        }
    );
    println!("  \"runtime_boundary\": \"external_local_command\",");
    println!("  \"paths\": \"<redacted>\",");
    println!("  \"model_manifest\": \"valid\",");
    println!(
        "  \"embedding_command\": \"{}\",",
        if command_available {
            "available"
        } else {
            "missing"
        }
    );
    println!("  \"embedding_protocol\": \"{}\",", protocol_status.label());
    println!("  \"model_id\": \"{}\",", model.model_id);
    println!("  \"dimension\": {},", model.dimension.unwrap_or(0));
    println!("  \"license_reviewed\": true,");
    print!("  \"remediation\": [");
    let remediation = model_preflight_remediation(command_available, protocol_status);
    for (index, item) in remediation.iter().enumerate() {
        if index > 0 {
            print!(", ");
        }
        print!("\"{item}\"");
    }
    println!("]");
    println!("}}");
}

fn model_preflight_remediation(
    command_available: bool,
    protocol_status: EmbeddingPreflightProtocolStatus,
) -> Vec<&'static str> {
    let mut remediation = Vec::new();
    if !command_available {
        remediation.push("configure --embedding-command with a local executable");
    } else if protocol_status == EmbeddingPreflightProtocolStatus::Failed {
        remediation.push("verify the local embedding command speaks resume-ir-embedding-v1");
    }
    remediation
}

fn parse_model_validate_manifest_args(args: &[String]) -> Result<PathBuf> {
    let mut manifest = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--manifest" => {
                if manifest.is_some() {
                    return Err(CliError::usage(model_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(model_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(model_usage()));
                }
                manifest = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err(CliError::usage(model_usage())),
        }
    }

    manifest.ok_or_else(|| CliError::usage(model_usage()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModelManifestValidation {
    model_pack_id: String,
    models: Vec<ModelManifestModelValidation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ModelManifestModelValidation {
    model_id: String,
    model_type: String,
    dimension: Option<usize>,
    sha256: String,
}

fn validate_model_manifest(manifest_path: &Path) -> Result<ModelManifestValidation> {
    let manifest_text = fs::read_to_string(manifest_path)
        .map_err(|_| CliError::user("model manifest blocked: manifest is unavailable"))?;
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest_text)
        .map_err(|_| CliError::user("model manifest blocked: invalid manifest"))?;
    validate_model_manifest_allowed_keys(
        &manifest_json,
        &["schema_version", "model_pack_id", "models"],
    )?;

    let schema_version = model_manifest_string(&manifest_json, "schema_version")?;
    if schema_version != MODEL_MANIFEST_SCHEMA_VERSION {
        return Err(CliError::user(
            "model manifest blocked: unsupported schema version",
        ));
    }

    let model_pack_id = model_manifest_string(&manifest_json, "model_pack_id")?;
    if !valid_model_manifest_identifier(model_pack_id) {
        return Err(CliError::user(
            "model manifest blocked: invalid model pack id",
        ));
    }

    let models = model_manifest_array(&manifest_json, "models")?
        .iter()
        .map(|model| validate_model_manifest_model(manifest_path, model))
        .collect::<Result<Vec<_>>>()?;

    Ok(ModelManifestValidation {
        model_pack_id: model_pack_id.to_string(),
        models,
    })
}

fn validate_model_manifest_model(
    manifest_path: &Path,
    model: &serde_json::Value,
) -> Result<ModelManifestModelValidation> {
    validate_model_manifest_allowed_keys(
        model,
        &["id", "type", "dim", "format", "artifact", "license"],
    )?;

    let model_id = model_manifest_string(model, "id")?;
    if !valid_model_manifest_identifier(model_id) {
        return Err(CliError::user("model manifest blocked: invalid model id"));
    }

    let model_type = model_manifest_string(model, "type")?;
    let dimension = match model_type {
        "embedding" => Some(model_manifest_positive_usize(model, "dim")?),
        "ner" | "ocr" => None,
        _ => {
            return Err(CliError::user(
                "model manifest blocked: unsupported model type",
            ))
        }
    };

    let format = model_manifest_string(model, "format")?;
    if !valid_model_manifest_identifier(format) {
        return Err(CliError::user(
            "model manifest blocked: invalid model format",
        ));
    }

    let artifact = model_manifest_object(model, "artifact")?;
    validate_model_manifest_allowed_keys(artifact, &["path", "sha256"])?;
    let artifact_path = model_manifest_string(artifact, "path")?;
    if artifact_path.trim().is_empty()
        || artifact_path.contains('\n')
        || artifact_path.contains('\r')
    {
        return Err(CliError::user("model manifest blocked: invalid artifact"));
    }
    let expected_sha256 = model_manifest_sha256(model_manifest_string(artifact, "sha256")?)?;

    let license = model_manifest_object(model, "license")?;
    validate_model_manifest_allowed_keys(license, &["id", "reviewed"])?;
    let license_id = model_manifest_string(license, "id")?;
    if !valid_license_expression(license_id) {
        return Err(CliError::user("model manifest blocked: invalid license"));
    }
    let reviewed = license
        .get("reviewed")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| CliError::user("model manifest blocked: invalid license"))?;
    if !reviewed {
        return Err(CliError::user(
            "model manifest blocked: license has not been reviewed",
        ));
    }

    let artifact_path = model_manifest_artifact_path(manifest_path, artifact_path);
    let actual_sha256 = file_sha256_hex(&artifact_path)
        .map_err(|_| CliError::user("model manifest blocked: artifact is unavailable"))?;
    if actual_sha256 != expected_sha256 {
        return Err(CliError::user("model manifest blocked: checksum mismatch"));
    }

    Ok(ModelManifestModelValidation {
        model_id: model_id.to_string(),
        model_type: model_type.to_string(),
        dimension,
        sha256: actual_sha256,
    })
}

fn validate_model_manifest_allowed_keys(
    value: &serde_json::Value,
    allowed_keys: &[&str],
) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| CliError::user("model manifest blocked: invalid manifest"))?;
    validate_release_evidence_allowed_keys(object, allowed_keys, "model manifest")
}

fn model_manifest_object<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Result<&'a serde_json::Value> {
    value
        .get(key)
        .filter(|field| field.is_object())
        .ok_or_else(|| CliError::user("model manifest blocked: invalid manifest"))
}

fn model_manifest_array<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Result<&'a [serde_json::Value]> {
    let array = value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::user("model manifest blocked: invalid manifest"))?;
    if array.is_empty() {
        return Err(CliError::user("model manifest blocked: invalid manifest"));
    }
    Ok(array)
}

fn model_manifest_string<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|field| !field.trim().is_empty())
        .ok_or_else(|| CliError::user("model manifest blocked: invalid manifest"))
}

fn model_manifest_positive_usize(value: &serde_json::Value, key: &str) -> Result<usize> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|field| usize::try_from(field).ok())
        .filter(|field| *field > 0)
        .ok_or_else(|| CliError::user("model manifest blocked: invalid manifest"))
}

fn model_manifest_sha256(value: &str) -> Result<String> {
    let value = value.to_ascii_lowercase();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::user("model manifest blocked: invalid checksum"));
    }
    Ok(value)
}

fn model_manifest_artifact_path(manifest_path: &Path, artifact_path: &str) -> PathBuf {
    let artifact_path = PathBuf::from(artifact_path);
    if artifact_path.is_absolute() {
        artifact_path
    } else {
        manifest_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(artifact_path)
    }
}

fn valid_model_manifest_identifier(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | '.' | '/' | ':' | '+')
        })
}

fn valid_license_expression(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '-' | '_' | '.' | '/' | ':' | '+' | '(' | ')' | ' '
                )
        })
}

fn model_usage() -> &'static str {
    "usage: resume-cli model draft-manifest --out <path> --model-pack-id <id> --model-id <id> --model-type embedding --dimension <n> --format <id> --artifact <path> --license <id> [--reviewed] | resume-cli model preflight --json --manifest <path> --embedding-command <path> --model-id <id> --dimension <n> | resume-cli model validate-manifest --manifest <path>"
}

fn ocr_validate_manifest_command(args: &[String]) -> Result<()> {
    let manifest_path = parse_ocr_validate_manifest_args(args)?;
    let validation = validate_ocr_runtime_manifest(&manifest_path)?;

    println!("ocr runtime manifest: valid");
    println!("runtime pack: {}", validation.runtime_pack_id);
    println!("components: {}", validation.components.len());
    for component in &validation.components {
        println!("component id: {}", component.component_id);
        println!("kind: {}", component.kind);
        println!("engine: {}", component.engine);
        println!("version: {}", component.version);
        println!("license reviewed: yes");
        println!("checksum match: yes");
        println!("sha256 prefix: {}", checksum_prefix(&component.sha256));
    }
    println!("languages: {}", validation.languages.len());
    for language in &validation.languages {
        println!("language id: {}", language.language_id);
        println!("license reviewed: yes");
        println!("checksum match: yes");
        println!("sha256 prefix: {}", checksum_prefix(&language.sha256));
    }
    println!("paths: <redacted>");
    Ok(())
}

#[derive(Debug, Clone)]
struct OcrDraftManifestArgs {
    out: PathBuf,
    runtime_pack_id: String,
    tesseract_command: PathBuf,
    pdftoppm_command: PathBuf,
    language_packs: Vec<OcrLanguagePackDraft>,
    engine_license: String,
    renderer_license: String,
    language_license: String,
    reviewed: bool,
}

#[derive(Debug, Clone)]
struct OcrLanguagePackDraft {
    id: String,
    path: PathBuf,
}

fn ocr_draft_manifest_command(args: &[String]) -> Result<()> {
    let draft_args = parse_ocr_draft_manifest_args(args)?;
    if !is_executable_file(&draft_args.tesseract_command) {
        return Err(CliError::user(
            "ocr runtime manifest draft blocked: tesseract command is unavailable",
        ));
    }
    if !is_executable_file(&draft_args.pdftoppm_command) {
        return Err(CliError::user(
            "ocr runtime manifest draft blocked: pdftoppm command is unavailable",
        ));
    }
    for language_pack in &draft_args.language_packs {
        if !language_pack.path.is_file() {
            return Err(CliError::user(
                "ocr runtime manifest draft blocked: language pack is unavailable",
            ));
        }
    }

    let tesseract_sha256 = file_sha256_hex(&draft_args.tesseract_command).map_err(|_| {
        CliError::user("ocr runtime manifest draft blocked: tesseract checksum unavailable")
    })?;
    let pdftoppm_sha256 = file_sha256_hex(&draft_args.pdftoppm_command).map_err(|_| {
        CliError::user("ocr runtime manifest draft blocked: pdftoppm checksum unavailable")
    })?;
    let language_entries = draft_args
        .language_packs
        .iter()
        .map(|language_pack| {
            let language_sha256 = file_sha256_hex(&language_pack.path).map_err(|_| {
                CliError::user(
                    "ocr runtime manifest draft blocked: language pack checksum unavailable",
                )
            })?;
            Ok(serde_json::json!({
                "id": language_pack.id,
                "artifact": {
                    "path": path_string_lossless(&language_pack.path)?,
                    "sha256": language_sha256
                },
                "license": {
                    "id": draft_args.language_license,
                    "reviewed": draft_args.reviewed
                }
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let tesseract_version =
        local_command_version_token(&draft_args.tesseract_command, &["--version"]);
    let pdftoppm_version = local_command_version_token(&draft_args.pdftoppm_command, &["-v"]);

    let manifest = serde_json::json!({
        "schema_version": OCR_RUNTIME_MANIFEST_SCHEMA_VERSION,
        "runtime_pack_id": draft_args.runtime_pack_id,
        "components": [
            {
                "id": "tesseract",
                "kind": "ocr-engine",
                "engine": "tesseract",
                "version": tesseract_version,
                "artifact": {
                    "path": path_string_lossless(&draft_args.tesseract_command)?,
                    "sha256": tesseract_sha256
                },
                "license": {
                    "id": draft_args.engine_license,
                    "reviewed": draft_args.reviewed
                }
            },
            {
                "id": "poppler-pdftoppm",
                "kind": "pdf-renderer",
                "engine": "poppler-pdftoppm",
                "version": pdftoppm_version,
                "artifact": {
                    "path": path_string_lossless(&draft_args.pdftoppm_command)?,
                    "sha256": pdftoppm_sha256
                },
                "license": {
                    "id": draft_args.renderer_license,
                    "reviewed": draft_args.reviewed
                }
            }
        ],
        "languages": language_entries
    });
    let manifest_text = serde_json::to_string_pretty(&manifest)
        .map_err(|_| CliError::user("ocr runtime manifest draft blocked: invalid manifest"))?;
    if let Some(parent) = draft_args.out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|_| {
                CliError::user("ocr runtime manifest draft blocked: output is unavailable")
            })?;
        }
    }
    fs::write(&draft_args.out, format!("{manifest_text}\n"))
        .map_err(|_| CliError::user("ocr runtime manifest draft blocked: output is unavailable"))?;

    println!("ocr runtime manifest draft: written");
    println!("schema: {OCR_RUNTIME_MANIFEST_SCHEMA_VERSION}");
    println!(
        "runtime pack: {}",
        manifest["runtime_pack_id"].as_str().unwrap_or("unknown")
    );
    println!("components: 2");
    println!("languages: {}", draft_args.language_packs.len());
    println!(
        "license reviewed: {}",
        if draft_args.reviewed { "yes" } else { "no" }
    );
    println!("paths: <redacted>");
    Ok(())
}

fn parse_ocr_draft_manifest_args(args: &[String]) -> Result<OcrDraftManifestArgs> {
    let mut out = None;
    let mut runtime_pack_id = None;
    let mut tesseract_command = None;
    let mut pdftoppm_command = None;
    let mut language = None;
    let mut language_pack_args = Vec::new();
    let mut engine_license = None;
    let mut renderer_license = None;
    let mut language_license = None;
    let mut reviewed = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                out = Some(take_ocr_path_arg(args, &mut index, out.is_some())?);
            }
            "--runtime-pack-id" => {
                runtime_pack_id = Some(take_ocr_identifier_arg(
                    args,
                    &mut index,
                    runtime_pack_id.is_some(),
                )?);
            }
            "--tesseract-command" => {
                tesseract_command = Some(take_ocr_path_arg(
                    args,
                    &mut index,
                    tesseract_command.is_some(),
                )?);
            }
            "--pdftoppm-command" => {
                pdftoppm_command = Some(take_ocr_path_arg(
                    args,
                    &mut index,
                    pdftoppm_command.is_some(),
                )?);
            }
            "--language" => {
                language = Some(take_ocr_identifier_arg(
                    args,
                    &mut index,
                    language.is_some(),
                )?);
            }
            "--language-pack" => {
                language_pack_args.push(take_ocr_language_pack_arg(args, &mut index)?);
            }
            "--engine-license" => {
                engine_license = Some(take_ocr_license_arg(
                    args,
                    &mut index,
                    engine_license.is_some(),
                )?);
            }
            "--renderer-license" => {
                renderer_license = Some(take_ocr_license_arg(
                    args,
                    &mut index,
                    renderer_license.is_some(),
                )?);
            }
            "--language-license" => {
                language_license = Some(take_ocr_license_arg(
                    args,
                    &mut index,
                    language_license.is_some(),
                )?);
            }
            "--reviewed" => {
                if reviewed {
                    return Err(CliError::usage(ocr_usage()));
                }
                reviewed = true;
                index += 1;
            }
            _ => return Err(CliError::usage(ocr_usage())),
        }
    }

    let language = language.ok_or_else(|| CliError::usage(ocr_usage()))?;
    let language_packs = parse_ocr_language_pack_args(&language, &language_pack_args)?;

    Ok(OcrDraftManifestArgs {
        out: out.ok_or_else(|| CliError::usage(ocr_usage()))?,
        runtime_pack_id: runtime_pack_id.ok_or_else(|| CliError::usage(ocr_usage()))?,
        tesseract_command: tesseract_command.ok_or_else(|| CliError::usage(ocr_usage()))?,
        pdftoppm_command: pdftoppm_command.ok_or_else(|| CliError::usage(ocr_usage()))?,
        language_packs,
        engine_license: engine_license.ok_or_else(|| CliError::usage(ocr_usage()))?,
        renderer_license: renderer_license.ok_or_else(|| CliError::usage(ocr_usage()))?,
        language_license: language_license.ok_or_else(|| CliError::usage(ocr_usage()))?,
        reviewed,
    })
}

fn parse_ocr_language_pack_args(
    requested_language: &str,
    raw_args: &[String],
) -> Result<Vec<OcrLanguagePackDraft>> {
    if raw_args.is_empty() {
        return Err(CliError::usage(ocr_usage()));
    }

    let requested_languages = split_ocr_language_set(requested_language)?;
    if raw_args.len() == 1 && !raw_args[0].contains('=') {
        return Ok(vec![OcrLanguagePackDraft {
            id: requested_language.to_string(),
            path: PathBuf::from(&raw_args[0]),
        }]);
    }

    let mut language_packs = Vec::new();
    for raw_arg in raw_args {
        let Some((language_id, path)) = raw_arg.split_once('=') else {
            return Err(CliError::usage(ocr_usage()));
        };
        if !valid_model_manifest_identifier(language_id) || path.trim().is_empty() {
            return Err(CliError::usage(ocr_usage()));
        }
        if !requested_languages
            .iter()
            .any(|language| language == language_id)
        {
            return Err(CliError::usage(ocr_usage()));
        }
        if language_packs
            .iter()
            .any(|language_pack: &OcrLanguagePackDraft| language_pack.id == language_id)
        {
            return Err(CliError::usage(ocr_usage()));
        }
        language_packs.push(OcrLanguagePackDraft {
            id: language_id.to_string(),
            path: PathBuf::from(path),
        });
    }

    if language_packs.len() != requested_languages.len() {
        return Err(CliError::usage(ocr_usage()));
    }

    Ok(language_packs)
}

fn split_ocr_language_set(requested_language: &str) -> Result<Vec<String>> {
    let languages = requested_language
        .split('+')
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(|language| {
            if valid_model_manifest_identifier(language) {
                Ok(language.to_string())
            } else {
                Err(CliError::usage(ocr_usage()))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    if languages.is_empty() {
        return Err(CliError::usage(ocr_usage()));
    }
    Ok(languages)
}

fn take_ocr_language_pack_arg(args: &[String], index: &mut usize) -> Result<String> {
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(ocr_usage()));
    };
    if value.trim().is_empty() {
        return Err(CliError::usage(ocr_usage()));
    }
    *index += 2;
    Ok(value.clone())
}

fn take_ocr_path_arg(args: &[String], index: &mut usize, duplicate: bool) -> Result<PathBuf> {
    if duplicate {
        return Err(CliError::usage(ocr_usage()));
    }
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(ocr_usage()));
    };
    if value.trim().is_empty() {
        return Err(CliError::usage(ocr_usage()));
    }
    *index += 2;
    Ok(PathBuf::from(value))
}

fn take_ocr_identifier_arg(args: &[String], index: &mut usize, duplicate: bool) -> Result<String> {
    if duplicate {
        return Err(CliError::usage(ocr_usage()));
    }
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(ocr_usage()));
    };
    if !valid_model_manifest_identifier(value) {
        return Err(CliError::usage(ocr_usage()));
    }
    *index += 2;
    Ok(value.clone())
}

fn take_ocr_license_arg(args: &[String], index: &mut usize, duplicate: bool) -> Result<String> {
    if duplicate {
        return Err(CliError::usage(ocr_usage()));
    }
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(ocr_usage()));
    };
    if !valid_license_expression(value) {
        return Err(CliError::usage(ocr_usage()));
    }
    *index += 2;
    Ok(value.clone())
}

fn path_string_lossless(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| CliError::user("ocr runtime manifest draft blocked: invalid path"))
}

fn local_command_version_token(command: &Path, args: &[&str]) -> String {
    let output = Command::new(command).args(args).output();
    let Ok(output) = output else {
        return "unknown".to_string();
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    stdout
        .split_whitespace()
        .chain(stderr.split_whitespace())
        .find_map(safe_version_token)
        .unwrap_or_else(|| "unknown".to_string())
}

fn safe_version_token(token: &str) -> Option<String> {
    let token = token
        .trim_matches(|character: char| {
            matches!(
                character,
                '"' | '\'' | '(' | ')' | '[' | ']' | ',' | ';' | ':'
            )
        })
        .trim_start_matches('v');
    if token.is_empty() || !token.chars().any(|character| character.is_ascii_digit()) {
        return None;
    }
    let normalized = token
        .chars()
        .take_while(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+')
        })
        .collect::<String>();
    if valid_model_manifest_identifier(&normalized) {
        Some(normalized)
    } else {
        None
    }
}

#[derive(Debug, Clone)]
struct OcrPreflightArgs {
    ocr_lang: String,
    tesseract_command: Option<PathBuf>,
    pdftoppm_command: Option<PathBuf>,
}

fn ocr_preflight_command(args: &[String]) -> Result<()> {
    let preflight_args = parse_ocr_preflight_args(args)?;
    let pdftoppm_command =
        resolve_ocr_preflight_command(preflight_args.pdftoppm_command.as_ref(), "pdftoppm");
    let tesseract_command =
        resolve_ocr_preflight_command(preflight_args.tesseract_command.as_ref(), "tesseract");
    let runtime = inspect_ocr_runtime_with_commands(
        &preflight_args.ocr_lang,
        pdftoppm_command.as_ref(),
        tesseract_command.as_ref(),
    );
    let dependencies_ready = runtime.pdftoppm == OcrRuntimeState::Available
        && runtime.tesseract == OcrRuntimeState::Available
        && runtime.requested_language_status == OcrRuntimeState::Available;
    let probe_status = if dependencies_ready {
        ocr_preflight_probe_status(
            pdftoppm_command.as_ref(),
            tesseract_command.as_ref(),
            &preflight_args.ocr_lang,
        )
    } else {
        OcrPreflightProbeStatus::NotRun
    };
    let ready = dependencies_ready && probe_status == OcrPreflightProbeStatus::Passed;
    print_ocr_preflight_json(&runtime, ready, probe_status);
    if ready {
        Ok(())
    } else {
        Err(CliError::user(
            "ocr runtime preflight blocked: dependencies are not ready",
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OcrPreflightProbeStatus {
    Passed,
    Failed,
    NotRun,
}

impl OcrPreflightProbeStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotRun => "not_run",
        }
    }
}

fn resolve_ocr_preflight_command(configured: Option<&PathBuf>, name: &str) -> Option<PathBuf> {
    configured.cloned().or_else(|| find_command_in_path(name))
}

fn ocr_preflight_probe_status(
    pdftoppm_command: Option<&PathBuf>,
    tesseract_command: Option<&PathBuf>,
    ocr_lang: &str,
) -> OcrPreflightProbeStatus {
    let Some(pdftoppm_command) = pdftoppm_command else {
        return OcrPreflightProbeStatus::NotRun;
    };
    let Some(tesseract_command) = tesseract_command else {
        return OcrPreflightProbeStatus::NotRun;
    };

    let budget = match OcrWorkerBudget::new(5_000) {
        Ok(budget) => budget,
        Err(_) => return OcrPreflightProbeStatus::Failed,
    };
    let cancellation = CancellationToken::new();
    let render_spec = match PdftoppmRenderSpec::new(pdftoppm_command.clone()) {
        Ok(spec) => spec,
        Err(_) => return OcrPreflightProbeStatus::Failed,
    };
    let renderer = PdftoppmPdfRenderer::new(render_spec);
    let rendered = match renderer.render_page(
        &ocr_preflight_blank_pdf_bytes(),
        1,
        72,
        budget,
        &cancellation,
    ) {
        Ok(rendered) => rendered,
        Err(_) => return OcrPreflightProbeStatus::Failed,
    };
    let tesseract = match TesseractOcrSpec::new(tesseract_command.clone(), "preflight-tesseract") {
        Ok(spec) => TesseractOcrClient::new(spec),
        Err(_) => return OcrPreflightProbeStatus::Failed,
    };
    let options = match OcrOptions::new(ocr_lang, "preflight") {
        Ok(options) => options,
        Err(_) => return OcrPreflightProbeStatus::Failed,
    };
    let request = match OcrPageRequest::new(rendered, options) {
        Ok(request) => request,
        Err(_) => return OcrPreflightProbeStatus::Failed,
    };

    match tesseract.recognize_page(request, budget, &cancellation) {
        Ok(page) if page.page_no() == 1 => OcrPreflightProbeStatus::Passed,
        _ => OcrPreflightProbeStatus::Failed,
    }
}

fn ocr_preflight_blank_pdf_bytes() -> Vec<u8> {
    let mut output = Vec::new();
    output.extend_from_slice(b"%PDF-1.4\n");
    let object_1 = output.len();
    output.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
    let object_2 = output.len();
    output.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
    let object_3 = output.len();
    output.extend_from_slice(
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] /Resources << >> >>\nendobj\n",
    );
    let xref = output.len();
    output.extend_from_slice(b"xref\n0 4\n");
    output.extend_from_slice(b"0000000000 65535 f \n");
    for offset in [object_1, object_2, object_3] {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    output
}

fn parse_ocr_preflight_args(args: &[String]) -> Result<OcrPreflightArgs> {
    let mut json = false;
    let mut ocr_lang = "eng".to_string();
    let mut seen_ocr_lang = false;
    let mut tesseract_command = None;
    let mut pdftoppm_command = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                if json {
                    return Err(CliError::usage(ocr_usage()));
                }
                json = true;
                index += 1;
            }
            "--ocr-lang" => {
                if seen_ocr_lang {
                    return Err(CliError::usage(ocr_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(ocr_usage()));
                };
                ocr_lang = parse_ocr_diagnostic_language(value, ocr_usage())?;
                seen_ocr_lang = true;
                index += 2;
            }
            "--tesseract-command" => {
                if tesseract_command.is_some() {
                    return Err(CliError::usage(ocr_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(ocr_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(ocr_usage()));
                }
                tesseract_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--pdftoppm-command" => {
                if pdftoppm_command.is_some() {
                    return Err(CliError::usage(ocr_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(ocr_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(ocr_usage()));
                }
                pdftoppm_command = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err(CliError::usage(ocr_usage())),
        }
    }

    if !json {
        return Err(CliError::usage(ocr_usage()));
    }

    Ok(OcrPreflightArgs {
        ocr_lang,
        tesseract_command,
        pdftoppm_command,
    })
}

fn print_ocr_preflight_json(
    runtime: &OcrRuntimeDiagnostic,
    ready: bool,
    probe_status: OcrPreflightProbeStatus,
) {
    println!("{{");
    println!("  \"schema_version\": \"ocr-runtime-preflight.v1\",");
    println!(
        "  \"runtime_status\": \"{}\",",
        if ready { "ready" } else { "blocked" }
    );
    println!("  \"runtime_boundary\": \"external_local_commands\",");
    println!("  \"paths\": \"<redacted>\",");
    println!("  \"dependencies\": {{");
    println!("    \"pdftoppm\": \"{}\",", runtime.pdftoppm.label());
    println!("    \"tesseract\": \"{}\",", runtime.tesseract.label());
    println!(
        "    \"requested_language\": \"{}\",",
        runtime.requested_language
    );
    println!(
        "    \"requested_language_status\": \"{}\"",
        runtime.requested_language_status.label()
    );
    println!("  }},");
    println!("  \"runtime_probe\": \"{}\",", probe_status.label());
    print!("  \"remediation\": [");
    let remediation = ocr_preflight_remediation(runtime, probe_status);
    for (index, item) in remediation.iter().enumerate() {
        if index > 0 {
            print!(", ");
        }
        print!("\"{item}\"");
    }
    println!("]");
    println!("}}");
}

fn ocr_preflight_remediation(
    runtime: &OcrRuntimeDiagnostic,
    probe_status: OcrPreflightProbeStatus,
) -> Vec<&'static str> {
    let mut remediation = Vec::new();
    if runtime.pdftoppm != OcrRuntimeState::Available {
        remediation.push("install Poppler/pdftoppm or configure --pdftoppm-command");
    }
    if runtime.tesseract != OcrRuntimeState::Available {
        remediation.push("install Tesseract/tessdata or configure --tesseract-command");
    } else if runtime.requested_language_status != OcrRuntimeState::Available {
        remediation
            .push("install requested Tesseract language pack or choose an installed --ocr-lang");
    }
    if probe_status == OcrPreflightProbeStatus::Failed {
        remediation.push("verify pdftoppm can render and Tesseract can OCR a local probe");
    }
    remediation
}

fn parse_ocr_validate_manifest_args(args: &[String]) -> Result<PathBuf> {
    let mut manifest = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--manifest" => {
                if manifest.is_some() {
                    return Err(CliError::usage(ocr_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(ocr_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(ocr_usage()));
                }
                manifest = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err(CliError::usage(ocr_usage())),
        }
    }

    manifest.ok_or_else(|| CliError::usage(ocr_usage()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OcrRuntimeManifestValidation {
    runtime_pack_id: String,
    components: Vec<OcrRuntimeComponentValidation>,
    languages: Vec<OcrRuntimeLanguageValidation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OcrRuntimeComponentValidation {
    component_id: String,
    kind: String,
    engine: String,
    version: String,
    sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OcrRuntimeLanguageValidation {
    language_id: String,
    sha256: String,
}

fn validate_ocr_runtime_manifest(manifest_path: &Path) -> Result<OcrRuntimeManifestValidation> {
    let manifest_text = fs::read_to_string(manifest_path)
        .map_err(|_| CliError::user("ocr runtime manifest blocked: manifest is unavailable"))?;
    let manifest_json: serde_json::Value = serde_json::from_str(&manifest_text)
        .map_err(|_| CliError::user("ocr runtime manifest blocked: invalid manifest"))?;
    validate_ocr_runtime_manifest_allowed_keys(
        &manifest_json,
        &[
            "schema_version",
            "runtime_pack_id",
            "components",
            "languages",
        ],
    )?;

    let schema_version = ocr_manifest_string(&manifest_json, "schema_version")?;
    if schema_version != OCR_RUNTIME_MANIFEST_SCHEMA_VERSION {
        return Err(CliError::user(
            "ocr runtime manifest blocked: unsupported schema version",
        ));
    }

    let runtime_pack_id = ocr_manifest_string(&manifest_json, "runtime_pack_id")?;
    if !valid_model_manifest_identifier(runtime_pack_id) {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid runtime pack id",
        ));
    }

    let components = ocr_manifest_array(&manifest_json, "components")?
        .iter()
        .map(|component| validate_ocr_runtime_component(manifest_path, component))
        .collect::<Result<Vec<_>>>()?;
    let languages = match manifest_json.get("languages") {
        Some(value) => ocr_manifest_array_value(value)?
            .iter()
            .map(|language| validate_ocr_runtime_language(manifest_path, language))
            .collect::<Result<Vec<_>>>()?,
        None => Vec::new(),
    };

    Ok(OcrRuntimeManifestValidation {
        runtime_pack_id: runtime_pack_id.to_string(),
        components,
        languages,
    })
}

fn validate_ocr_runtime_component(
    manifest_path: &Path,
    component: &serde_json::Value,
) -> Result<OcrRuntimeComponentValidation> {
    validate_ocr_runtime_manifest_allowed_keys(
        component,
        &["id", "kind", "engine", "version", "artifact", "license"],
    )?;

    let component_id = ocr_manifest_string(component, "id")?;
    if !valid_model_manifest_identifier(component_id) {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid component id",
        ));
    }

    let kind = ocr_manifest_string(component, "kind")?;
    if !matches!(kind, "ocr-engine" | "pdf-renderer" | "ocr-language-pack") {
        return Err(CliError::user(
            "ocr runtime manifest blocked: unsupported component kind",
        ));
    }
    let engine = ocr_manifest_string(component, "engine")?;
    if !valid_model_manifest_identifier(engine) {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid engine id",
        ));
    }
    let version = ocr_manifest_string(component, "version")?;
    if !valid_model_manifest_identifier(version) {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid version",
        ));
    }
    let sha256 = validate_ocr_runtime_artifact(manifest_path, component)?;
    validate_ocr_manifest_license(component)?;

    Ok(OcrRuntimeComponentValidation {
        component_id: component_id.to_string(),
        kind: kind.to_string(),
        engine: engine.to_string(),
        version: version.to_string(),
        sha256,
    })
}

fn validate_ocr_runtime_language(
    manifest_path: &Path,
    language: &serde_json::Value,
) -> Result<OcrRuntimeLanguageValidation> {
    validate_ocr_runtime_manifest_allowed_keys(language, &["id", "artifact", "license"])?;
    let language_id = ocr_manifest_string(language, "id")?;
    if !valid_model_manifest_identifier(language_id) {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid language id",
        ));
    }
    let sha256 = validate_ocr_runtime_artifact(manifest_path, language)?;
    validate_ocr_manifest_license(language)?;

    Ok(OcrRuntimeLanguageValidation {
        language_id: language_id.to_string(),
        sha256,
    })
}

fn validate_ocr_runtime_artifact(
    manifest_path: &Path,
    value: &serde_json::Value,
) -> Result<String> {
    let artifact = ocr_manifest_object(value, "artifact")?;
    validate_ocr_runtime_manifest_allowed_keys(artifact, &["path", "sha256"])?;
    let artifact_path = ocr_manifest_string(artifact, "path")?;
    if artifact_path.trim().is_empty()
        || artifact_path.contains('\n')
        || artifact_path.contains('\r')
    {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid artifact",
        ));
    }
    let expected_sha256 = ocr_manifest_sha256(ocr_manifest_string(artifact, "sha256")?)?;
    let artifact_path = model_manifest_artifact_path(manifest_path, artifact_path);
    let actual_sha256 = file_sha256_hex(&artifact_path)
        .map_err(|_| CliError::user("ocr runtime manifest blocked: artifact is unavailable"))?;
    if actual_sha256 != expected_sha256 {
        return Err(CliError::user(
            "ocr runtime manifest blocked: checksum mismatch",
        ));
    }

    Ok(actual_sha256)
}

fn validate_ocr_manifest_license(value: &serde_json::Value) -> Result<()> {
    let license = ocr_manifest_object(value, "license")?;
    validate_ocr_runtime_manifest_allowed_keys(license, &["id", "reviewed"])?;
    let license_id = ocr_manifest_string(license, "id")?;
    if !valid_license_expression(license_id) {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid license",
        ));
    }
    let reviewed = license
        .get("reviewed")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| CliError::user("ocr runtime manifest blocked: invalid license"))?;
    if !reviewed {
        return Err(CliError::user(
            "ocr runtime manifest blocked: license has not been reviewed",
        ));
    }

    Ok(())
}

fn validate_ocr_runtime_manifest_allowed_keys(
    value: &serde_json::Value,
    allowed_keys: &[&str],
) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| CliError::user("ocr runtime manifest blocked: invalid manifest"))?;
    validate_release_evidence_allowed_keys(object, allowed_keys, "ocr runtime manifest")
}

fn ocr_manifest_object<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Result<&'a serde_json::Value> {
    value
        .get(key)
        .filter(|field| field.is_object())
        .ok_or_else(|| CliError::user("ocr runtime manifest blocked: invalid manifest"))
}

fn ocr_manifest_array<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Result<&'a [serde_json::Value]> {
    let Some(array) = value.get(key) else {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid manifest",
        ));
    };
    ocr_manifest_array_value(array)
}

fn ocr_manifest_array_value(value: &serde_json::Value) -> Result<&[serde_json::Value]> {
    let array = value
        .as_array()
        .ok_or_else(|| CliError::user("ocr runtime manifest blocked: invalid manifest"))?;
    if array.is_empty() {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid manifest",
        ));
    }
    Ok(array)
}

fn ocr_manifest_string<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|field| !field.trim().is_empty())
        .ok_or_else(|| CliError::user("ocr runtime manifest blocked: invalid manifest"))
}

fn ocr_manifest_sha256(value: &str) -> Result<String> {
    let value = value.to_ascii_lowercase();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::user(
            "ocr runtime manifest blocked: invalid checksum",
        ));
    }
    Ok(value)
}

fn ocr_usage() -> &'static str {
    "usage: resume-cli ocr draft-manifest --out <path> --runtime-pack-id <id> --tesseract-command <path> --pdftoppm-command <path> --language <lang> --language-pack <path|lang=path> [--language-pack <lang=path> ...] --engine-license <id> --renderer-license <id> --language-license <id> [--reviewed] | resume-cli ocr preflight --json [--ocr-lang <lang>] [--tesseract-command <path>] [--pdftoppm-command <path>] | resume-cli ocr validate-manifest --manifest <path>"
}

fn service_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(CliError::usage(service_usage()));
    };

    match action {
        "install" => service_install_command(data_dir, &args[1..]),
        "uninstall" => service_uninstall_command(&args[1..]),
        "status" => service_status_command(&args[1..]),
        "start" => service_start_command(&args[1..]),
        "stop" => service_stop_command(&args[1..]),
        _ => Err(CliError::usage(service_usage())),
    }
}

fn service_install_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let install_args = parse_service_install_args(args)?;
    let daemon_binary = install_args
        .daemon_binary
        .as_ref()
        .cloned()
        .unwrap_or_else(default_daemon_binary_path);

    if !daemon_binary.is_file() {
        return Err(CliError::user(
            "service install blocked: daemon binary is unavailable",
        ));
    }

    let program_arguments = service_program_arguments(data_dir, &daemon_binary, &install_args)?;
    if install_args.common.platform == ServicePlatform::WindowsService {
        if !install_args.common.dry_run {
            return Err(windows_service_control_blocked("install"));
        }
        print_windows_service_dry_run("install", &install_args.common, "sc.exe create");
        println!("service command: <redacted>");
        return Ok(());
    }

    let plist_path = service_plist_path(&install_args.common);
    let stdout_path = data_dir.join("logs").join("resume-daemon.stdout.log");
    let stderr_path = data_dir.join("logs").join("resume-daemon.stderr.log");
    let plist = render_launch_agent_plist(
        &install_args.common.label,
        &program_arguments,
        &stdout_path,
        &stderr_path,
    )?;

    if install_args.common.dry_run {
        println!("service: install dry-run");
        println!("label: {}", install_args.common.label);
        println!("platform: macos-launch-agent");
        println!("launch agent: would write");
        println!("paths: <redacted>");
        return Ok(());
    }

    fs::create_dir_all(data_dir)
        .map_err(|_| CliError::user("unable to prepare service data directory"))?;
    fs::create_dir_all(data_dir.join("logs"))
        .map_err(|_| CliError::user("unable to prepare service log directory"))?;
    fs::create_dir_all(&install_args.common.launch_agent_dir)
        .map_err(|_| CliError::user("unable to prepare service launch agent directory"))?;
    write_service_file_atomically(&plist_path, plist.as_bytes())?;

    println!("service: installed");
    println!("label: {}", install_args.common.label);
    println!("platform: macos-launch-agent");
    println!("launch agent: configured");
    println!("paths: <redacted>");
    Ok(())
}

fn service_uninstall_command(args: &[String]) -> Result<()> {
    let common = parse_service_common_args(args, true)?;
    if common.platform == ServicePlatform::WindowsService {
        if !common.dry_run {
            return Err(windows_service_control_blocked("uninstall"));
        }
        print_windows_service_dry_run("uninstall", &common, "sc.exe delete");
        return Ok(());
    }

    let plist_path = service_plist_path(&common);
    if common.dry_run {
        println!("service: uninstall dry-run");
        println!("label: {}", common.label);
        println!("platform: macos-launch-agent");
        println!("launch agent: would remove");
        println!("paths: <redacted>");
        return Ok(());
    }

    match fs::remove_file(&plist_path) {
        Ok(()) => {
            println!("service: uninstalled");
            println!("label: {}", common.label);
            println!("user data: preserved");
            println!("paths: <redacted>");
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("service: not installed");
            println!("label: {}", common.label);
            println!("user data: preserved");
            println!("paths: <redacted>");
            Ok(())
        }
        Err(_) => Err(CliError::user("unable to remove service launch agent")),
    }
}

fn service_status_command(args: &[String]) -> Result<()> {
    let common = parse_service_common_args(args, true)?;
    if common.platform == ServicePlatform::WindowsService {
        if !common.dry_run {
            return Err(windows_service_control_blocked("status"));
        }
        print_windows_service_dry_run("status", &common, "sc.exe query");
        return Ok(());
    }

    let plist_path = service_plist_path(&common);
    let installed = plist_path.exists();
    if installed {
        println!("service: installed");
    } else {
        println!("service: not installed");
    }
    if common.dry_run {
        println!("label: {}", common.label);
        println!("platform: macos-launch-agent");
        println!("launchctl print: <redacted>");
        println!("paths: <redacted>");
        return Ok(());
    }
    let runtime_state = if installed {
        query_service_runtime_state(&common.label)?
    } else {
        ServiceRuntimeState::NotLoaded
    };
    println!("label: {}", common.label);
    println!("platform: macos-launch-agent");
    println!("runtime: {}", runtime_state.label());
    println!("paths: <redacted>");
    Ok(())
}

fn service_start_command(args: &[String]) -> Result<()> {
    let common = parse_service_common_args(args, true)?;
    if common.platform == ServicePlatform::WindowsService {
        if !common.dry_run {
            return Err(windows_service_control_blocked("start"));
        }
        print_windows_service_dry_run("start", &common, "sc.exe start");
        return Ok(());
    }

    let plist_path = service_plist_path(&common);
    if !plist_path.exists() {
        return Err(CliError::user(
            "service start blocked: service is not installed",
        ));
    }

    if common.dry_run {
        println!("service: start dry-run");
        println!("label: {}", common.label);
        println!("launchctl bootstrap: <redacted>");
        println!("launchctl kickstart: <redacted>");
        return Ok(());
    }

    let domain = current_user_launchctl_domain()?;
    run_launchctl(&["bootstrap", domain.as_str(), path_as_str(&plist_path)?])?;
    let target = format!("{domain}/{}", common.label);
    run_launchctl(&["kickstart", "-k", target.as_str()])?;

    println!("service: started");
    println!("label: {}", common.label);
    println!("paths: <redacted>");
    Ok(())
}

fn service_stop_command(args: &[String]) -> Result<()> {
    let common = parse_service_common_args(args, true)?;
    if common.platform == ServicePlatform::WindowsService {
        if !common.dry_run {
            return Err(windows_service_control_blocked("stop"));
        }
        print_windows_service_dry_run("stop", &common, "sc.exe stop");
        return Ok(());
    }

    let plist_path = service_plist_path(&common);
    if !plist_path.exists() {
        return Err(CliError::user(
            "service stop blocked: service is not installed",
        ));
    }

    if common.dry_run {
        println!("service: stop dry-run");
        println!("label: {}", common.label);
        println!("launchctl bootout: <redacted>");
        return Ok(());
    }

    let domain = current_user_launchctl_domain()?;
    run_launchctl(&["bootout", domain.as_str(), path_as_str(&plist_path)?])?;

    println!("service: stopped");
    println!("label: {}", common.label);
    println!("paths: <redacted>");
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ServiceCommonArgs {
    label: String,
    platform: ServicePlatform,
    launch_agent_dir: PathBuf,
    dry_run: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServicePlatform {
    MacosLaunchAgent,
    WindowsService,
}

impl ServicePlatform {
    fn label(self) -> &'static str {
        match self {
            ServicePlatform::MacosLaunchAgent => "macos-launch-agent",
            ServicePlatform::WindowsService => "windows-service",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ServiceInstallArgs {
    common: ServiceCommonArgs,
    daemon_binary: Option<PathBuf>,
    ocr_command: Option<PathBuf>,
    ocr_engine_profile: Option<String>,
    ocr_lang: Option<String>,
    ocr_profile: Option<String>,
    ocr_render_dpi: Option<String>,
    ocr_page_timeout_ms: Option<String>,
    ocr_max_pages_per_document: Option<String>,
    embedding_command: Option<PathBuf>,
    embedding_model_id: Option<String>,
    embedding_dimension: Option<String>,
    embedding_timeout_ms: Option<String>,
}

fn parse_service_install_args(args: &[String]) -> Result<ServiceInstallArgs> {
    let mut label = DEFAULT_SERVICE_LABEL.to_string();
    let mut platform = ServicePlatform::MacosLaunchAgent;
    let mut platform_seen = false;
    let mut launch_agent_dir = None;
    let mut dry_run = false;
    let mut daemon_binary = None;
    let mut ocr_command = None;
    let mut ocr_engine_profile = None;
    let mut ocr_lang = None;
    let mut ocr_profile = None;
    let mut ocr_render_dpi = None;
    let mut ocr_page_timeout_ms = None;
    let mut ocr_max_pages_per_document = None;
    let mut embedding_command = None;
    let mut embedding_model_id = None;
    let mut embedding_dimension = None;
    let mut embedding_timeout_ms = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--label" => {
                label = parse_service_label(take_service_value(args, &mut index)?)?;
            }
            "--platform" => {
                if platform_seen {
                    return Err(CliError::usage(service_usage()));
                }
                platform = parse_service_platform(take_service_value(args, &mut index)?)?;
                platform_seen = true;
            }
            "--launch-agent-dir" => {
                if launch_agent_dir.is_some() {
                    return Err(CliError::usage(service_usage()));
                }
                launch_agent_dir = Some(PathBuf::from(take_service_value(args, &mut index)?));
            }
            "--daemon-binary" => {
                set_once_path(
                    &mut daemon_binary,
                    PathBuf::from(take_service_value(args, &mut index)?),
                )?;
            }
            "--ocr-command" => {
                set_once_path(
                    &mut ocr_command,
                    PathBuf::from(take_service_value(args, &mut index)?),
                )?;
            }
            "--ocr-engine-profile" => {
                set_once_string(
                    &mut ocr_engine_profile,
                    take_service_identifier(args, &mut index)?,
                )?;
            }
            "--ocr-lang" => {
                set_once_string(&mut ocr_lang, take_service_identifier(args, &mut index)?)?;
            }
            "--ocr-profile" => {
                set_once_string(&mut ocr_profile, take_service_identifier(args, &mut index)?)?;
            }
            "--ocr-render-dpi" => {
                set_once_string(
                    &mut ocr_render_dpi,
                    take_service_positive_number(args, &mut index)?,
                )?;
            }
            "--ocr-page-timeout-ms" => {
                set_once_string(
                    &mut ocr_page_timeout_ms,
                    take_service_positive_number(args, &mut index)?,
                )?;
            }
            "--ocr-max-pages-per-document" => {
                set_once_string(
                    &mut ocr_max_pages_per_document,
                    take_service_positive_number(args, &mut index)?,
                )?;
            }
            "--embedding-command" => {
                set_once_path(
                    &mut embedding_command,
                    PathBuf::from(take_service_value(args, &mut index)?),
                )?;
            }
            "--embedding-model-id" => {
                set_once_string(
                    &mut embedding_model_id,
                    take_service_identifier(args, &mut index)?,
                )?;
            }
            "--embedding-dimension" => {
                set_once_string(
                    &mut embedding_dimension,
                    take_service_positive_number(args, &mut index)?,
                )?;
            }
            "--embedding-timeout-ms" => {
                set_once_string(
                    &mut embedding_timeout_ms,
                    take_service_positive_number(args, &mut index)?,
                )?;
            }
            "--dry-run" => {
                if dry_run {
                    return Err(CliError::usage(service_usage()));
                }
                dry_run = true;
                index += 1;
            }
            _ => return Err(CliError::usage(service_usage())),
        }
    }

    if embedding_command.is_some()
        && (embedding_model_id.is_none() || embedding_dimension.is_none())
    {
        return Err(CliError::usage(service_usage()));
    }
    if embedding_command.is_none()
        && (embedding_model_id.is_some()
            || embedding_dimension.is_some()
            || embedding_timeout_ms.is_some())
    {
        return Err(CliError::usage(service_usage()));
    }
    if ocr_command.is_none()
        && (ocr_engine_profile.is_some()
            || ocr_lang.is_some()
            || ocr_profile.is_some()
            || ocr_render_dpi.is_some()
            || ocr_page_timeout_ms.is_some()
            || ocr_max_pages_per_document.is_some())
    {
        return Err(CliError::usage(service_usage()));
    }

    Ok(ServiceInstallArgs {
        common: ServiceCommonArgs {
            label,
            platform,
            launch_agent_dir: service_launch_agent_dir_or_default(platform, launch_agent_dir)?,
            dry_run,
        },
        daemon_binary,
        ocr_command,
        ocr_engine_profile,
        ocr_lang,
        ocr_profile,
        ocr_render_dpi,
        ocr_page_timeout_ms,
        ocr_max_pages_per_document,
        embedding_command,
        embedding_model_id,
        embedding_dimension,
        embedding_timeout_ms,
    })
}

fn parse_service_common_args(args: &[String], allow_dry_run: bool) -> Result<ServiceCommonArgs> {
    let mut label = DEFAULT_SERVICE_LABEL.to_string();
    let mut platform = ServicePlatform::MacosLaunchAgent;
    let mut platform_seen = false;
    let mut launch_agent_dir = None;
    let mut dry_run = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--label" => {
                label = parse_service_label(take_service_value(args, &mut index)?)?;
            }
            "--platform" => {
                if platform_seen {
                    return Err(CliError::usage(service_usage()));
                }
                platform = parse_service_platform(take_service_value(args, &mut index)?)?;
                platform_seen = true;
            }
            "--launch-agent-dir" => {
                if launch_agent_dir.is_some() {
                    return Err(CliError::usage(service_usage()));
                }
                launch_agent_dir = Some(PathBuf::from(take_service_value(args, &mut index)?));
            }
            "--dry-run" if allow_dry_run => {
                if dry_run {
                    return Err(CliError::usage(service_usage()));
                }
                dry_run = true;
                index += 1;
            }
            _ => return Err(CliError::usage(service_usage())),
        }
    }

    Ok(ServiceCommonArgs {
        label,
        platform,
        launch_agent_dir: service_launch_agent_dir_or_default(platform, launch_agent_dir)?,
        dry_run,
    })
}

fn service_program_arguments(
    data_dir: &Path,
    daemon_binary: &Path,
    install_args: &ServiceInstallArgs,
) -> Result<Vec<String>> {
    let mut arguments = vec![
        path_as_str(daemon_binary)?.to_string(),
        "--data-dir".to_string(),
        path_as_str(data_dir)?.to_string(),
        "run".to_string(),
        "--foreground".to_string(),
        "--work-imports".to_string(),
        "--work-index".to_string(),
        "--ipc-listen".to_string(),
        DEFAULT_SERVICE_IPC_LISTEN.to_string(),
    ];

    if let Some(command) = install_args.ocr_command.as_ref() {
        arguments.push("--work-ocr".to_string());
        arguments.push("--ocr-command".to_string());
        arguments.push(path_as_str(command)?.to_string());
        push_optional_pair(
            &mut arguments,
            "--ocr-engine-profile",
            install_args.ocr_engine_profile.as_deref(),
        );
        push_optional_pair(
            &mut arguments,
            "--ocr-lang",
            install_args.ocr_lang.as_deref(),
        );
        push_optional_pair(
            &mut arguments,
            "--ocr-profile",
            install_args.ocr_profile.as_deref(),
        );
        push_optional_pair(
            &mut arguments,
            "--ocr-render-dpi",
            install_args.ocr_render_dpi.as_deref(),
        );
        push_optional_pair(
            &mut arguments,
            "--ocr-page-timeout-ms",
            install_args.ocr_page_timeout_ms.as_deref(),
        );
        push_optional_pair(
            &mut arguments,
            "--ocr-max-pages-per-document",
            install_args.ocr_max_pages_per_document.as_deref(),
        );
    }

    if let Some(command) = install_args.embedding_command.as_ref() {
        arguments.push("--embedding-command".to_string());
        arguments.push(path_as_str(command)?.to_string());
        push_optional_pair(
            &mut arguments,
            "--embedding-model-id",
            install_args.embedding_model_id.as_deref(),
        );
        push_optional_pair(
            &mut arguments,
            "--embedding-dimension",
            install_args.embedding_dimension.as_deref(),
        );
        push_optional_pair(
            &mut arguments,
            "--embedding-timeout-ms",
            install_args.embedding_timeout_ms.as_deref(),
        );
    }

    Ok(arguments)
}

fn render_launch_agent_plist(
    label: &str,
    program_arguments: &[String],
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<String> {
    let mut plist = String::new();
    plist.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    plist.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    plist.push_str("<plist version=\"1.0\">\n");
    plist.push_str("<dict>\n");
    plist.push_str("  <key>Label</key>\n");
    plist.push_str("  <string>");
    plist.push_str(&xml_escape(label));
    plist.push_str("</string>\n");
    plist.push_str("  <key>ProgramArguments</key>\n");
    plist.push_str("  <array>\n");
    for argument in program_arguments {
        plist.push_str("    <string>");
        plist.push_str(&xml_escape(argument));
        plist.push_str("</string>\n");
    }
    plist.push_str("  </array>\n");
    plist.push_str("  <key>RunAtLoad</key>\n");
    plist.push_str("  <true/>\n");
    plist.push_str("  <key>KeepAlive</key>\n");
    plist.push_str("  <true/>\n");
    plist.push_str("  <key>StandardOutPath</key>\n");
    plist.push_str("  <string>");
    plist.push_str(&xml_escape(path_as_str(stdout_path)?));
    plist.push_str("</string>\n");
    plist.push_str("  <key>StandardErrorPath</key>\n");
    plist.push_str("  <string>");
    plist.push_str(&xml_escape(path_as_str(stderr_path)?));
    plist.push_str("</string>\n");
    plist.push_str("</dict>\n");
    plist.push_str("</plist>\n");
    Ok(plist)
}

fn write_service_file_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(CliError::user("service launch agent path is invalid"));
    };
    let tmp_path = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|file_name| file_name.to_str())
            .unwrap_or("resume-ir-service"),
        std::process::id()
    ));
    fs::write(&tmp_path, bytes)
        .map_err(|_| CliError::user("unable to write service launch agent"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o644))
            .map_err(|_| CliError::user("unable to secure service launch agent"))?;
    }
    fs::rename(&tmp_path, path)
        .map_err(|_| CliError::user("unable to publish service launch agent"))?;
    Ok(())
}

fn print_windows_service_dry_run(action: &str, common: &ServiceCommonArgs, command: &str) {
    println!("service: {action} dry-run");
    println!("label: {}", common.label);
    println!("platform: {}", common.platform.label());
    println!("{command}: <redacted>");
    println!("paths: <redacted>");
}

fn windows_service_control_blocked(action: &str) -> CliError {
    CliError::user(format!(
        "service {action} blocked: Windows service control requires a Windows service validation run"
    ))
}

fn current_user_launchctl_domain() -> Result<String> {
    let output = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .map_err(|_| CliError::user("unable to determine user launch domain"))?;
    if !output.status.success() {
        return Err(CliError::user("unable to determine user launch domain"));
    }
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uid.is_empty() || !uid.chars().all(|character| character.is_ascii_digit()) {
        return Err(CliError::user("unable to determine user launch domain"));
    }
    Ok(format!("gui/{uid}"))
}

fn run_launchctl(args: &[&str]) -> Result<()> {
    let output = Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|_| CliError::user("unable to run launchctl"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CliError::user("launchctl reported a service error"))
    }
}

fn query_service_runtime_state(label: &str) -> Result<ServiceRuntimeState> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = label;
        Ok(ServiceRuntimeState::Unknown)
    }

    #[cfg(target_os = "macos")]
    {
        let domain = current_user_launchctl_domain()?;
        let target = format!("{domain}/{label}");
        let output = Command::new("/bin/launchctl")
            .args(["print", target.as_str()])
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                Ok(service_runtime_state_from_launchctl_result(
                    output.status.success(),
                    &stdout,
                    &stderr,
                ))
            }
            Err(_) => Ok(ServiceRuntimeState::Unknown),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServiceRuntimeState {
    #[cfg(any(target_os = "macos", test))]
    Running,
    #[cfg(any(target_os = "macos", test))]
    Loaded,
    NotLoaded,
    Unknown,
}

impl ServiceRuntimeState {
    fn label(self) -> &'static str {
        match self {
            #[cfg(any(target_os = "macos", test))]
            Self::Running => "running",
            #[cfg(any(target_os = "macos", test))]
            Self::Loaded => "loaded",
            Self::NotLoaded => "not_loaded",
            Self::Unknown => "unknown",
        }
    }
}

#[cfg(any(target_os = "macos", test))]
fn service_runtime_state_from_launchctl_result(
    success: bool,
    stdout: &str,
    stderr: &str,
) -> ServiceRuntimeState {
    if success {
        if stdout
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case("state = running"))
        {
            ServiceRuntimeState::Running
        } else {
            ServiceRuntimeState::Loaded
        }
    } else if stderr.contains("Could not find service") || stderr.contains("No such process") {
        ServiceRuntimeState::NotLoaded
    } else {
        ServiceRuntimeState::Unknown
    }
}

fn service_plist_path(common: &ServiceCommonArgs) -> PathBuf {
    common
        .launch_agent_dir
        .join(format!("{}.plist", common.label))
}

fn default_daemon_binary_path() -> PathBuf {
    let current_exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("resume-cli"));
    let binary_name = if cfg!(windows) {
        "resume-daemon.exe"
    } else {
        "resume-daemon"
    };
    current_exe
        .parent()
        .map(|parent| parent.join(binary_name))
        .unwrap_or_else(|| PathBuf::from(binary_name))
}

fn default_launch_agent_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::user("service launch agent directory is not configured"))?;
    Ok(home.join("Library").join("LaunchAgents"))
}

fn service_launch_agent_dir_or_default(
    platform: ServicePlatform,
    launch_agent_dir: Option<PathBuf>,
) -> Result<PathBuf> {
    match (platform, launch_agent_dir) {
        (_, Some(path)) => Ok(path),
        (ServicePlatform::MacosLaunchAgent, None) => default_launch_agent_dir(),
        (ServicePlatform::WindowsService, None) => Ok(PathBuf::new()),
    }
}

fn parse_service_label(value: &str) -> Result<String> {
    if value.is_empty()
        || value.starts_with('.')
        || value.ends_with('.')
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '.' || character == '-'
        })
    {
        return Err(CliError::usage(service_usage()));
    }
    Ok(value.to_string())
}

fn parse_service_platform(value: &str) -> Result<ServicePlatform> {
    match value {
        "macos-launch-agent" => Ok(ServicePlatform::MacosLaunchAgent),
        "windows-service" => Ok(ServicePlatform::WindowsService),
        _ => Err(CliError::usage(service_usage())),
    }
}

fn take_service_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str> {
    let Some(value) = args.get(*index + 1).map(String::as_str) else {
        return Err(CliError::usage(service_usage()));
    };
    if value.trim().is_empty() {
        return Err(CliError::usage(service_usage()));
    }
    *index += 2;
    Ok(value)
}

fn take_service_identifier(args: &[String], index: &mut usize) -> Result<String> {
    let value = take_service_value(args, index)?;
    if !valid_cli_identifier(value) {
        return Err(CliError::usage(service_usage()));
    }
    Ok(value.to_string())
}

fn valid_cli_identifier(value: &str) -> bool {
    !value.trim().is_empty()
        && !value.contains('\n')
        && !value.contains('\r')
        && !value.contains('\t')
}

fn take_service_positive_number(args: &[String], index: &mut usize) -> Result<String> {
    let value = take_service_value(args, index)?;
    if value
        .parse::<usize>()
        .ok()
        .filter(|parsed| *parsed > 0)
        .is_none()
    {
        return Err(CliError::usage(service_usage()));
    }
    Ok(value.to_string())
}

fn set_once_path(slot: &mut Option<PathBuf>, value: PathBuf) -> Result<()> {
    if slot.is_some() {
        return Err(CliError::usage(service_usage()));
    }
    *slot = Some(value);
    Ok(())
}

fn set_once_string(slot: &mut Option<String>, value: String) -> Result<()> {
    if slot.is_some() {
        return Err(CliError::usage(service_usage()));
    }
    *slot = Some(value);
    Ok(())
}

fn push_optional_pair(arguments: &mut Vec<String>, flag: &str, value: Option<&str>) {
    if let Some(value) = value {
        arguments.push(flag.to_string());
        arguments.push(value.to_string());
    }
}

fn path_as_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| CliError::user("service path contains unsupported characters"))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn service_usage() -> &'static str {
    "usage: resume-cli service <install|uninstall|status|start|stop> [--platform <macos-launch-agent|windows-service>] [--launch-agent-dir <path>] [--label <id>] [--dry-run] [--daemon-binary <path>] [--ocr-command <path>] [--ocr-max-pages-per-document <n>] [--embedding-command <path> --embedding-model-id <id> --embedding-dimension <n>]"
}

fn fault_simulate_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let fault_args = parse_fault_simulate_args(args)?;
    let scratch_dir = fault_args
        .scratch_dir
        .clone()
        .unwrap_or_else(|| data_dir.join("fault-probes"));

    if let Some(suite) = fault_args.suite {
        return print_fault_simulation_suite_report(
            suite,
            data_dir,
            &scratch_dir,
            fault_args.json,
            fault_args.daemon_binary.as_deref(),
            fault_args.ocr_command.as_deref(),
        );
    }

    let report = fault_simulation_report_for_args(&scratch_dir, &fault_args)?;
    print_fault_simulation_report(fault_args.json, report)
}

fn fault_simulation_report_for_args(
    scratch_dir: &Path,
    fault_args: &FaultSimulationArgs,
) -> Result<FaultSimulationReport> {
    let case = fault_args
        .case
        .ok_or_else(|| CliError::usage(fault_simulate_usage()))?;
    let report = match case {
        FaultSimulationCase::DiskSpaceLow => {
            let required = fault_args
                .required_bytes
                .ok_or_else(|| CliError::usage(fault_simulate_usage()))?;
            let available = fault_args
                .available_bytes
                .ok_or_else(|| CliError::usage(fault_simulate_usage()))?;

            if required > available {
                fault_report(
                    "disk_space_low",
                    "reproduced",
                    serde_json::json!({
                        "required_bytes": required,
                        "available_bytes": available,
                        "probe_writes": "skipped"
                    }),
                    vec![
                        "fault: disk_space_low".to_string(),
                        format!("required bytes: {required}"),
                        format!("available bytes: {available}"),
                        "status: reproduced".to_string(),
                        "probe writes: skipped".to_string(),
                        "paths: <redacted>".to_string(),
                    ],
                )
            } else {
                let probe_bytes = required.min(FAULT_PROBE_MAX_BYTES);
                write_fault_probe(scratch_dir, probe_bytes)
                    .map_err(|_| CliError::user("fault simulation probe failed"))?;
                fault_report(
                    "disk_space_low",
                    "not reproduced",
                    serde_json::json!({
                        "required_bytes": required,
                        "available_bytes": available,
                        "probe_writes": "completed",
                        "probe_bytes": probe_bytes
                    }),
                    vec![
                        "fault: disk_space_low".to_string(),
                        format!("required bytes: {required}"),
                        format!("available bytes: {available}"),
                        "status: not reproduced".to_string(),
                        "probe writes: completed".to_string(),
                        format!("probe bytes: {probe_bytes}"),
                        "paths: <redacted>".to_string(),
                    ],
                )
            }
        }
        FaultSimulationCase::PermissionDenied => match write_fault_probe(scratch_dir, 1) {
            Ok(()) => fault_report(
                "permission_denied",
                "not reproduced",
                serde_json::json!({ "probe_writes": "completed" }),
                vec![
                    "fault: permission_denied".to_string(),
                    "status: not reproduced".to_string(),
                    "probe writes: completed".to_string(),
                    "paths: <redacted>".to_string(),
                ],
            ),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => fault_report(
                "permission_denied",
                "reproduced",
                serde_json::json!({ "probe_writes": "denied" }),
                vec![
                    "fault: permission_denied".to_string(),
                    "status: reproduced".to_string(),
                    "probe writes: denied".to_string(),
                    "paths: <redacted>".to_string(),
                ],
            ),
            Err(_) => return Err(CliError::user("fault simulation probe failed")),
        },
        FaultSimulationCase::FileLock => match contend_file_lock_probe(scratch_dir) {
            Ok(FileLockProbeResult::Contended) => fault_report(
                "file_lock",
                "reproduced",
                serde_json::json!({
                    "lock_holder": "active",
                    "contended_lock": "denied"
                }),
                vec![
                    "fault: file_lock".to_string(),
                    "status: reproduced".to_string(),
                    "lock holder: active".to_string(),
                    "contended lock: denied".to_string(),
                    "paths: <redacted>".to_string(),
                ],
            ),
            Ok(FileLockProbeResult::NotContended) => fault_report(
                "file_lock",
                "not reproduced",
                serde_json::json!({
                    "lock_holder": "active",
                    "contended_lock": "acquired"
                }),
                vec![
                    "fault: file_lock".to_string(),
                    "status: not reproduced".to_string(),
                    "lock holder: active".to_string(),
                    "contended lock: acquired".to_string(),
                    "paths: <redacted>".to_string(),
                ],
            ),
            Err(_) => return Err(CliError::user("fault simulation probe failed")),
        },
        FaultSimulationCase::IndexSnapshotCorrupt => {
            let result = simulate_index_snapshot_corrupt_probe(scratch_dir)?;
            let status = if result.reproduced {
                "reproduced"
            } else {
                "not reproduced"
            };
            let ready_generation = if result.ready_generation_corrupt {
                "corrupt"
            } else {
                "not_corrupt"
            };
            let recovery_rebuilt = if result.recovery_rebuilt { "yes" } else { "no" };
            let previous_generation_retained = if result.previous_generation_retained {
                "yes"
            } else {
                "no"
            };
            let query_after_recovery = if result.query_after_recovery_passed {
                "passed"
            } else {
                "failed"
            };
            fault_report(
                "index_snapshot_corrupt",
                status,
                serde_json::json!({
                    "ready_generation": ready_generation,
                    "recovery_rebuilt": recovery_rebuilt,
                    "previous_generation_retained": previous_generation_retained,
                    "query_after_recovery": query_after_recovery
                }),
                vec![
                    "fault: index_snapshot_corrupt".to_string(),
                    format!("status: {status}"),
                    format!("ready generation: {ready_generation}"),
                    format!("recovery rebuilt: {recovery_rebuilt}"),
                    format!("previous generation retained: {previous_generation_retained}"),
                    format!("query after recovery: {query_after_recovery}"),
                    "paths: <redacted>".to_string(),
                ],
            )
        }
        FaultSimulationCase::MetadataMigration => {
            let result = simulate_metadata_migration_failure_probe(scratch_dir)
                .map_err(|_| CliError::user("fault simulation probe failed"))?;
            let (status, migration_check) = if result.reproduced {
                ("reproduced", "failed")
            } else {
                ("not reproduced", "passed")
            };
            let recovery = "restore metadata backup before retrying migration";
            fault_report(
                "metadata_migration",
                status,
                serde_json::json!({
                    "migration_check": migration_check,
                    "recovery": recovery
                }),
                vec![
                    "fault: metadata_migration".to_string(),
                    format!("status: {status}"),
                    format!("migration check: {migration_check}"),
                    format!("recovery: {recovery}"),
                    "paths: <redacted>".to_string(),
                ],
            )
        }
        FaultSimulationCase::ModelChecksum => {
            let model_file = fault_args
                .model_file
                .as_deref()
                .ok_or_else(|| CliError::usage(fault_simulate_usage()))?;
            let expected_sha256 = fault_args
                .expected_sha256
                .as_deref()
                .ok_or_else(|| CliError::usage(fault_simulate_usage()))?;

            let actual_sha256 = file_sha256_hex(model_file)
                .map_err(|_| CliError::user("fault simulation probe failed"))?;
            let reproduced = actual_sha256 != expected_sha256;
            let (status, checksum_match) = if reproduced {
                ("reproduced", "no")
            } else {
                ("not reproduced", "yes")
            };
            let expected_prefix = checksum_prefix(expected_sha256).to_string();
            let actual_prefix = checksum_prefix(&actual_sha256).to_string();
            fault_report(
                "model_checksum",
                status,
                serde_json::json!({
                    "checksum_match": checksum_match,
                    "expected_sha256_prefix": expected_prefix,
                    "actual_sha256_prefix": actual_prefix
                }),
                vec![
                    "fault: model_checksum".to_string(),
                    format!("status: {status}"),
                    format!("checksum match: {checksum_match}"),
                    format!("expected sha256 prefix: {expected_prefix}"),
                    format!("actual sha256 prefix: {actual_prefix}"),
                    "paths: <redacted>".to_string(),
                ],
            )
        }
        FaultSimulationCase::DaemonKill => {
            let daemon_binary = fault_args
                .daemon_binary
                .as_deref()
                .ok_or_else(|| CliError::usage(fault_simulate_usage()))?;

            let result = simulate_daemon_kill_probe(scratch_dir, daemon_binary)
                .map_err(|_| CliError::user("fault simulation probe failed"))?;
            let status = if result.terminated && result.restart_succeeded {
                "reproduced"
            } else {
                "not reproduced"
            };
            let terminated_daemon = if result.terminated { "yes" } else { "no" };
            let restart_check = if result.restart_succeeded {
                "passed"
            } else {
                "failed"
            };
            fault_report(
                "daemon_kill",
                status,
                serde_json::json!({
                    "daemon_ready": "yes",
                    "terminated_daemon": terminated_daemon,
                    "restart_check": restart_check
                }),
                vec![
                    "fault: daemon_kill".to_string(),
                    format!("status: {status}"),
                    "daemon ready: yes".to_string(),
                    format!("terminated daemon: {terminated_daemon}"),
                    format!("restart check: {restart_check}"),
                    "paths: <redacted>".to_string(),
                ],
            )
        }
        FaultSimulationCase::OcrCrash => {
            let ocr_command = fault_args
                .ocr_command
                .as_deref()
                .ok_or_else(|| CliError::usage(fault_simulate_usage()))?;

            let result = simulate_ocr_crash_probe(scratch_dir, ocr_command)?;
            let (status, ocr_command_status) = if result.reproduced {
                ("reproduced", "failed")
            } else {
                ("not reproduced", "completed")
            };
            fault_report(
                "ocr_crash",
                status,
                serde_json::json!({
                    "ocr_command": ocr_command_status,
                    "probe_bytes": result.probe_bytes
                }),
                vec![
                    "fault: ocr_crash".to_string(),
                    format!("status: {status}"),
                    format!("ocr command: {ocr_command_status}"),
                    format!("probe bytes: {}", result.probe_bytes),
                    "paths: <redacted>".to_string(),
                ],
            )
        }
        FaultSimulationCase::BatteryMode => {
            let battery_state = fault_args
                .battery_state
                .ok_or_else(|| CliError::usage(fault_simulate_usage()))?;

            let (status, power_source, degradation) = match battery_state {
                FaultBatteryState::Battery => (
                    "reproduced",
                    "battery",
                    "pause or lower OCR/vector worker budgets",
                ),
                FaultBatteryState::Ac => ("not reproduced", "ac", "not required"),
            };
            fault_report(
                "battery_mode",
                status,
                serde_json::json!({
                    "power_source": power_source,
                    "degradation": degradation,
                    "real_hardware_drill": "blocked"
                }),
                vec![
                    "fault: battery_mode".to_string(),
                    format!("status: {status}"),
                    format!("power source: {power_source}"),
                    format!("degradation: {degradation}"),
                    "real hardware drill: blocked".to_string(),
                    "paths: <redacted>".to_string(),
                ],
            )
        }
        FaultSimulationCase::ExternalDriveDisconnect => {
            let drive_state = fault_args
                .drive_state
                .ok_or_else(|| CliError::usage(fault_simulate_usage()))?;

            let (status, mount_state, import_roots, recovery) = match drive_state {
                FaultDriveState::Disconnected => (
                    "reproduced",
                    "disconnected",
                    "unavailable",
                    "reconnect drive or reselect root before retry",
                ),
                FaultDriveState::Mounted => {
                    ("not reproduced", "mounted", "available", "not required")
                }
            };
            fault_report(
                "external_drive_disconnect",
                status,
                serde_json::json!({
                    "mount_state": mount_state,
                    "import_roots": import_roots,
                    "recovery": recovery,
                    "real_hardware_drill": "blocked"
                }),
                vec![
                    "fault: external_drive_disconnect".to_string(),
                    format!("status: {status}"),
                    format!("mount state: {mount_state}"),
                    format!("import roots: {import_roots}"),
                    format!("recovery: {recovery}"),
                    "real hardware drill: blocked".to_string(),
                    "paths: <redacted>".to_string(),
                ],
            )
        }
    };

    Ok(report)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FaultSimulationReport {
    fault: &'static str,
    status: &'static str,
    details: serde_json::Value,
    text_lines: Vec<String>,
}

fn fault_report(
    fault: &'static str,
    status: &'static str,
    details: serde_json::Value,
    text_lines: Vec<String>,
) -> FaultSimulationReport {
    FaultSimulationReport {
        fault,
        status,
        details,
        text_lines,
    }
}

fn print_fault_simulation_report(json: bool, report: FaultSimulationReport) -> Result<()> {
    if json {
        let body = serde_json::json!({
            "schema_version": "fault-simulation.v1",
            "redacted": true,
            "fault": report.fault,
            "status": report.status,
            "paths": "<redacted>",
            "details": report.details,
            "evidence_level": "local_synthetic_fault_probe"
        });
        let output = serde_json::to_string_pretty(&body)
            .map_err(|_| CliError::user("fault simulation report serialization failed"))?;
        println!("{output}");
    } else {
        for line in report.text_lines {
            println!("{line}");
        }
    }
    Ok(())
}

fn print_fault_simulation_suite_report(
    suite: FaultSimulationSuite,
    data_dir: &Path,
    scratch_dir: &Path,
    json: bool,
    daemon_binary: Option<&Path>,
    ocr_command: Option<&Path>,
) -> Result<()> {
    if !json {
        return Err(CliError::usage(fault_simulate_usage()));
    }

    match suite {
        FaultSimulationSuite::LocalSafe => {
            let cases =
                run_local_safe_fault_suite(data_dir, scratch_dir, daemon_binary, ocr_command)?;
            let total_cases = cases.len();
            let failed_cases = cases.iter().filter(|case| case.status == "failed").count();
            let reproduced_cases = cases
                .iter()
                .filter(|case| case.status == "reproduced")
                .count();
            let blocked_by_host_cases = cases
                .iter()
                .filter(|case| case.status == "blocked_by_host")
                .count();
            let body = serde_json::json!({
                "schema_version": "fault-simulation-suite.v1",
                "suite": "local_safe",
                "redacted": true,
                "paths": "<redacted>",
                "evidence_level": "local_synthetic_fault_suite",
                "release_hardware_drills": "blocked",
                "summary": {
                    "total_cases": total_cases,
                    "reproduced_cases": reproduced_cases,
                    "blocked_by_host_cases": blocked_by_host_cases,
                    "failed_cases": failed_cases,
                    "release_blockers_cleared": false
                },
                "cases": cases.into_iter().map(FaultSuiteCaseReport::into_json).collect::<Vec<_>>()
            });
            let output = serde_json::to_string_pretty(&body)
                .map_err(|_| CliError::user("fault simulation report serialization failed"))?;
            println!("{output}");
            Ok(())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FaultSuiteCaseReport {
    fault: &'static str,
    status: String,
    details: serde_json::Value,
}

impl FaultSuiteCaseReport {
    fn from_report(report: FaultSimulationReport) -> Self {
        Self {
            fault: report.fault,
            status: report.status.to_string(),
            details: report.details,
        }
    }

    fn blocked_by_host(fault: &'static str, reason: &'static str) -> Self {
        Self {
            fault,
            status: "blocked_by_host".to_string(),
            details: serde_json::json!({ "reason": reason }),
        }
    }

    fn failed(fault: &'static str) -> Self {
        Self {
            fault,
            status: "failed".to_string(),
            details: serde_json::json!({ "reason": "probe failed" }),
        }
    }

    fn into_json(self) -> serde_json::Value {
        serde_json::json!({
            "fault": self.fault,
            "status": self.status,
            "redacted": true,
            "paths": "<redacted>",
            "details": self.details
        })
    }
}

fn run_local_safe_fault_suite(
    data_dir: &Path,
    scratch_dir: &Path,
    daemon_binary: Option<&Path>,
    ocr_command: Option<&Path>,
) -> Result<Vec<FaultSuiteCaseReport>> {
    let mut cases = vec![run_fault_suite_case(
        scratch_dir,
        "disk_space_low",
        FaultSimulationArgs::suite_case(FaultSimulationCase::DiskSpaceLow)
            .with_disk_space(4096, 1024),
    )];

    #[cfg(unix)]
    cases.push(run_permission_denied_suite_case(scratch_dir));
    #[cfg(not(unix))]
    cases.push(FaultSuiteCaseReport::blocked_by_host(
        "permission_denied",
        "permission bit probe requires unix permissions",
    ));

    cases.extend([
        run_fault_suite_case(
            scratch_dir,
            "file_lock",
            FaultSimulationArgs::suite_case(FaultSimulationCase::FileLock),
        ),
        run_fault_suite_case(
            scratch_dir,
            "index_snapshot_corrupt",
            FaultSimulationArgs::suite_case(FaultSimulationCase::IndexSnapshotCorrupt),
        ),
        run_fault_suite_case(
            scratch_dir,
            "metadata_migration",
            FaultSimulationArgs::suite_case(FaultSimulationCase::MetadataMigration),
        ),
        run_model_checksum_suite_case(scratch_dir),
    ]);
    cases.push(match daemon_binary {
        Some(path) => run_fault_suite_case(
            scratch_dir,
            "daemon_kill",
            FaultSimulationArgs::suite_case(FaultSimulationCase::DaemonKill)
                .with_daemon_binary(path.to_path_buf()),
        ),
        None => FaultSuiteCaseReport::blocked_by_host(
            "daemon_kill",
            "suite requires explicit daemon binary to avoid guessing host paths",
        ),
    });
    cases.push(match ocr_command {
        Some(path) => run_fault_suite_case(
            scratch_dir,
            "ocr_crash",
            FaultSimulationArgs::suite_case(FaultSimulationCase::OcrCrash)
                .with_ocr_command(path.to_path_buf()),
        ),
        None => FaultSuiteCaseReport::blocked_by_host(
            "ocr_crash",
            "suite requires explicit local OCR crash fixture to avoid shell-specific probes",
        ),
    });
    cases.extend([
        run_fault_suite_case(
            scratch_dir,
            "battery_mode",
            FaultSimulationArgs::suite_case(FaultSimulationCase::BatteryMode)
                .with_battery_state(FaultBatteryState::Battery),
        ),
        run_fault_suite_case(
            scratch_dir,
            "external_drive_disconnect",
            FaultSimulationArgs::suite_case(FaultSimulationCase::ExternalDriveDisconnect)
                .with_drive_state(FaultDriveState::Disconnected),
        ),
    ]);

    if !data_dir.as_os_str().is_empty() {
        let _ = fs::create_dir_all(scratch_dir);
    }

    Ok(cases)
}

fn run_fault_suite_case(
    scratch_dir: &Path,
    fault: &'static str,
    args: FaultSimulationArgs,
) -> FaultSuiteCaseReport {
    match fault_simulation_report_for_args(&scratch_dir.join(fault), &args) {
        Ok(report) => FaultSuiteCaseReport::from_report(report),
        Err(_) => FaultSuiteCaseReport::failed(fault),
    }
}

#[cfg(unix)]
fn run_permission_denied_suite_case(scratch_dir: &Path) -> FaultSuiteCaseReport {
    let case_dir = scratch_dir.join("permission_denied");
    if fs::create_dir_all(&case_dir).is_err() {
        return FaultSuiteCaseReport::failed("permission_denied");
    }
    let original_permissions = match fs::metadata(&case_dir).map(|metadata| metadata.permissions())
    {
        Ok(permissions) => permissions,
        Err(_) => return FaultSuiteCaseReport::failed("permission_denied"),
    };
    let mut denied = original_permissions.clone();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        denied.set_mode(0o500);
    }
    if fs::set_permissions(&case_dir, denied).is_err() {
        return FaultSuiteCaseReport::failed("permission_denied");
    }
    let result = run_fault_suite_case(
        scratch_dir,
        "permission_denied",
        FaultSimulationArgs::suite_case(FaultSimulationCase::PermissionDenied)
            .with_scratch_dir(case_dir.clone()),
    );
    let _ = fs::set_permissions(&case_dir, original_permissions);
    result
}

fn run_model_checksum_suite_case(scratch_dir: &Path) -> FaultSuiteCaseReport {
    let case_dir = scratch_dir.join("model_checksum");
    if fs::create_dir_all(&case_dir).is_err() {
        return FaultSuiteCaseReport::failed("model_checksum");
    }
    let model_path = case_dir.join("model.bin");
    if fs::write(&model_path, MODEL_CHECKSUM_PROBE_BYTES).is_err() {
        return FaultSuiteCaseReport::failed("model_checksum");
    }
    let result = run_fault_suite_case(
        scratch_dir,
        "model_checksum",
        FaultSimulationArgs::suite_case(FaultSimulationCase::ModelChecksum).with_model_file(
            model_path.clone(),
            "0000000000000000000000000000000000000000000000000000000000000000",
        ),
    );
    let _ = fs::remove_file(model_path);
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaultSimulationCase {
    DiskSpaceLow,
    PermissionDenied,
    FileLock,
    IndexSnapshotCorrupt,
    MetadataMigration,
    ModelChecksum,
    DaemonKill,
    OcrCrash,
    BatteryMode,
    ExternalDriveDisconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaultSimulationSuite {
    LocalSafe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaultBatteryState {
    Battery,
    Ac,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FaultDriveState {
    Disconnected,
    Mounted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FaultSimulationArgs {
    case: Option<FaultSimulationCase>,
    suite: Option<FaultSimulationSuite>,
    json: bool,
    scratch_dir: Option<PathBuf>,
    required_bytes: Option<u64>,
    available_bytes: Option<u64>,
    daemon_binary: Option<PathBuf>,
    ocr_command: Option<PathBuf>,
    model_file: Option<PathBuf>,
    expected_sha256: Option<String>,
    battery_state: Option<FaultBatteryState>,
    drive_state: Option<FaultDriveState>,
}

impl FaultSimulationArgs {
    fn suite_case(case: FaultSimulationCase) -> Self {
        Self {
            case: Some(case),
            suite: None,
            json: true,
            scratch_dir: None,
            required_bytes: None,
            available_bytes: None,
            daemon_binary: None,
            ocr_command: None,
            model_file: None,
            expected_sha256: None,
            battery_state: None,
            drive_state: None,
        }
    }

    fn with_scratch_dir(mut self, scratch_dir: PathBuf) -> Self {
        self.scratch_dir = Some(scratch_dir);
        self
    }

    fn with_disk_space(mut self, required_bytes: u64, available_bytes: u64) -> Self {
        self.required_bytes = Some(required_bytes);
        self.available_bytes = Some(available_bytes);
        self
    }

    fn with_model_file(mut self, model_file: PathBuf, expected_sha256: &str) -> Self {
        self.model_file = Some(model_file);
        self.expected_sha256 = Some(expected_sha256.to_string());
        self
    }

    fn with_daemon_binary(mut self, daemon_binary: PathBuf) -> Self {
        self.daemon_binary = Some(daemon_binary);
        self
    }

    fn with_ocr_command(mut self, ocr_command: PathBuf) -> Self {
        self.ocr_command = Some(ocr_command);
        self
    }

    fn with_battery_state(mut self, battery_state: FaultBatteryState) -> Self {
        self.battery_state = Some(battery_state);
        self
    }

    fn with_drive_state(mut self, drive_state: FaultDriveState) -> Self {
        self.drive_state = Some(drive_state);
        self
    }
}

fn parse_fault_simulate_args(args: &[String]) -> Result<FaultSimulationArgs> {
    let mut case = None;
    let mut suite = None;
    let mut json = false;
    let mut scratch_dir = None;
    let mut required_bytes = None;
    let mut available_bytes = None;
    let mut daemon_binary = None;
    let mut ocr_command = None;
    let mut model_file = None;
    let mut expected_sha256 = None;
    let mut battery_state = None;
    let mut drive_state = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--case" => {
                if case.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                let value = take_fault_value(args, &mut index)?;
                case = Some(parse_fault_case(value)?);
            }
            "--suite" => {
                if suite.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                let value = take_fault_value(args, &mut index)?;
                suite = Some(parse_fault_suite(value)?);
            }
            "--json" => {
                if json {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                json = true;
                index += 1;
            }
            "--scratch-dir" => {
                if scratch_dir.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                scratch_dir = Some(PathBuf::from(take_fault_value(args, &mut index)?));
            }
            "--required-bytes" => {
                if required_bytes.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                required_bytes = Some(take_fault_positive_u64(args, &mut index)?);
            }
            "--available-bytes" => {
                if available_bytes.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                available_bytes = Some(take_fault_positive_u64(args, &mut index)?);
            }
            "--daemon-binary" => {
                if daemon_binary.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                daemon_binary = Some(PathBuf::from(take_fault_value(args, &mut index)?));
            }
            "--ocr-command" => {
                if ocr_command.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                ocr_command = Some(PathBuf::from(take_fault_value(args, &mut index)?));
            }
            "--model-file" => {
                if model_file.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                model_file = Some(PathBuf::from(take_fault_value(args, &mut index)?));
            }
            "--expected-sha256" => {
                if expected_sha256.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                expected_sha256 = Some(take_fault_sha256(args, &mut index)?);
            }
            "--battery-state" => {
                if battery_state.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                battery_state = Some(parse_fault_battery_state(take_fault_value(
                    args, &mut index,
                )?)?);
            }
            "--drive-state" => {
                if drive_state.is_some() {
                    return Err(CliError::usage(fault_simulate_usage()));
                }
                drive_state = Some(parse_fault_drive_state(take_fault_value(
                    args, &mut index,
                )?)?);
            }
            _ => return Err(CliError::usage(fault_simulate_usage())),
        }
    }

    if suite.is_some() {
        if case.is_some()
            || required_bytes.is_some()
            || available_bytes.is_some()
            || model_file.is_some()
            || expected_sha256.is_some()
            || battery_state.is_some()
            || drive_state.is_some()
            || !json
        {
            return Err(CliError::usage(fault_simulate_usage()));
        }
        return Ok(FaultSimulationArgs {
            case: None,
            suite,
            json,
            scratch_dir,
            required_bytes,
            available_bytes,
            daemon_binary,
            ocr_command,
            model_file,
            expected_sha256,
            battery_state,
            drive_state,
        });
    }

    let case = case.ok_or_else(|| CliError::usage(fault_simulate_usage()))?;
    match case {
        FaultSimulationCase::DiskSpaceLow => {
            if required_bytes.is_none() || available_bytes.is_none() {
                return Err(CliError::usage(fault_simulate_usage()));
            }
            if daemon_binary.is_some()
                || ocr_command.is_some()
                || model_file.is_some()
                || expected_sha256.is_some()
                || battery_state.is_some()
                || drive_state.is_some()
            {
                return Err(CliError::usage(fault_simulate_usage()));
            }
        }
        FaultSimulationCase::PermissionDenied
        | FaultSimulationCase::FileLock
        | FaultSimulationCase::IndexSnapshotCorrupt
        | FaultSimulationCase::MetadataMigration => {
            if required_bytes.is_some()
                || available_bytes.is_some()
                || daemon_binary.is_some()
                || ocr_command.is_some()
                || model_file.is_some()
                || expected_sha256.is_some()
                || battery_state.is_some()
                || drive_state.is_some()
            {
                return Err(CliError::usage(fault_simulate_usage()));
            }
        }
        FaultSimulationCase::ModelChecksum => {
            if required_bytes.is_some()
                || available_bytes.is_some()
                || daemon_binary.is_some()
                || ocr_command.is_some()
                || model_file.is_none()
                || expected_sha256.is_none()
                || battery_state.is_some()
                || drive_state.is_some()
            {
                return Err(CliError::usage(fault_simulate_usage()));
            }
        }
        FaultSimulationCase::DaemonKill => {
            if required_bytes.is_some()
                || available_bytes.is_some()
                || daemon_binary.is_none()
                || ocr_command.is_some()
                || model_file.is_some()
                || expected_sha256.is_some()
                || battery_state.is_some()
                || drive_state.is_some()
            {
                return Err(CliError::usage(fault_simulate_usage()));
            }
        }
        FaultSimulationCase::OcrCrash => {
            if required_bytes.is_some()
                || available_bytes.is_some()
                || daemon_binary.is_some()
                || ocr_command.is_none()
                || model_file.is_some()
                || expected_sha256.is_some()
                || battery_state.is_some()
                || drive_state.is_some()
            {
                return Err(CliError::usage(fault_simulate_usage()));
            }
        }
        FaultSimulationCase::BatteryMode => {
            if required_bytes.is_some()
                || available_bytes.is_some()
                || daemon_binary.is_some()
                || ocr_command.is_some()
                || model_file.is_some()
                || expected_sha256.is_some()
                || battery_state.is_none()
                || drive_state.is_some()
            {
                return Err(CliError::usage(fault_simulate_usage()));
            }
        }
        FaultSimulationCase::ExternalDriveDisconnect => {
            if required_bytes.is_some()
                || available_bytes.is_some()
                || daemon_binary.is_some()
                || ocr_command.is_some()
                || model_file.is_some()
                || expected_sha256.is_some()
                || battery_state.is_some()
                || drive_state.is_none()
            {
                return Err(CliError::usage(fault_simulate_usage()));
            }
        }
    }

    Ok(FaultSimulationArgs {
        case: Some(case),
        suite: None,
        json,
        scratch_dir,
        required_bytes,
        available_bytes,
        daemon_binary,
        ocr_command,
        model_file,
        expected_sha256,
        battery_state,
        drive_state,
    })
}

fn parse_fault_suite(value: &str) -> Result<FaultSimulationSuite> {
    match value {
        "local-safe" | "local_safe" => Ok(FaultSimulationSuite::LocalSafe),
        _ => Err(CliError::usage(fault_simulate_usage())),
    }
}

fn parse_fault_case(value: &str) -> Result<FaultSimulationCase> {
    match value {
        "disk-space-low" => Ok(FaultSimulationCase::DiskSpaceLow),
        "permission-denied" => Ok(FaultSimulationCase::PermissionDenied),
        "file-lock" => Ok(FaultSimulationCase::FileLock),
        "index-snapshot-corrupt" | "index_snapshot_corrupt" => {
            Ok(FaultSimulationCase::IndexSnapshotCorrupt)
        }
        "migration-failure" | "metadata-migration" => Ok(FaultSimulationCase::MetadataMigration),
        "model-checksum" => Ok(FaultSimulationCase::ModelChecksum),
        "daemon-kill" => Ok(FaultSimulationCase::DaemonKill),
        "ocr-crash" => Ok(FaultSimulationCase::OcrCrash),
        "battery-mode" | "battery_mode" => Ok(FaultSimulationCase::BatteryMode),
        "external-drive-disconnect" | "external_drive_disconnect" => {
            Ok(FaultSimulationCase::ExternalDriveDisconnect)
        }
        _ => Err(CliError::usage(fault_simulate_usage())),
    }
}

fn parse_fault_battery_state(value: &str) -> Result<FaultBatteryState> {
    match value {
        "battery" => Ok(FaultBatteryState::Battery),
        "ac" => Ok(FaultBatteryState::Ac),
        _ => Err(CliError::usage(fault_simulate_usage())),
    }
}

fn parse_fault_drive_state(value: &str) -> Result<FaultDriveState> {
    match value {
        "disconnected" => Ok(FaultDriveState::Disconnected),
        "mounted" => Ok(FaultDriveState::Mounted),
        _ => Err(CliError::usage(fault_simulate_usage())),
    }
}

fn take_fault_value<'a>(args: &'a [String], index: &mut usize) -> Result<&'a str> {
    let Some(value) = args.get(*index + 1).map(String::as_str) else {
        return Err(CliError::usage(fault_simulate_usage()));
    };
    if value.trim().is_empty() {
        return Err(CliError::usage(fault_simulate_usage()));
    }
    *index += 2;
    Ok(value)
}

fn take_fault_positive_u64(args: &[String], index: &mut usize) -> Result<u64> {
    take_fault_value(args, index)?
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CliError::usage(fault_simulate_usage()))
}

fn take_fault_sha256(args: &[String], index: &mut usize) -> Result<String> {
    let value = take_fault_value(args, index)?.to_ascii_lowercase();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CliError::usage(fault_simulate_usage()));
    }
    Ok(value)
}

fn file_sha256_hex(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    Ok(hex_encode_lower(&hasher.finalize()))
}

fn hex_encode_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn update_sha256_string(hasher: &mut Sha256, value: &str) {
    update_sha256_u64(hasher, value.len() as u64);
    hasher.update(value.as_bytes());
}

fn update_sha256_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_le_bytes());
}

fn update_sha256_i64(hasher: &mut Sha256, value: i64) {
    hasher.update(value.to_le_bytes());
}

fn checksum_prefix(checksum: &str) -> &str {
    checksum.get(..8).unwrap_or("<invalid>")
}

fn write_fault_probe(scratch_dir: &Path, bytes: u64) -> std::io::Result<()> {
    fs::create_dir_all(scratch_dir)?;
    let probe_path = scratch_dir.join(format!(
        ".resume-ir-fault-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    let result = write_fault_probe_file(&probe_path, bytes);
    let _ = fs::remove_file(&probe_path);
    result
}

fn write_fault_probe_file(path: &Path, bytes: u64) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    let mut remaining = bytes;
    let buffer = [0_u8; 8192];
    while remaining > 0 {
        let chunk_len = remaining.min(buffer.len() as u64) as usize;
        file.write_all(&buffer[..chunk_len])?;
        remaining -= chunk_len as u64;
    }
    file.sync_all()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileLockProbeResult {
    Contended,
    NotContended,
}

fn contend_file_lock_probe(scratch_dir: &Path) -> std::io::Result<FileLockProbeResult> {
    fs::create_dir_all(scratch_dir)?;
    let lock_path = scratch_dir.join(format!(
        ".resume-ir-lock-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));

    let result = contend_file_lock_probe_file(&lock_path);
    let _ = fs::remove_file(&lock_path);
    result
}

fn contend_file_lock_probe_file(path: &Path) -> std::io::Result<FileLockProbeResult> {
    let holder = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(path)?;
    holder.lock_exclusive()?;

    let contender = fs::OpenOptions::new().read(true).write(true).open(path)?;
    let result = match contender.try_lock_exclusive() {
        Ok(true) => {
            contender.unlock()?;
            Ok(FileLockProbeResult::NotContended)
        }
        Ok(false) => Ok(FileLockProbeResult::Contended),
        Err(error) => Err(error),
    };

    holder.unlock()?;
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IndexSnapshotCorruptProbeResult {
    reproduced: bool,
    ready_generation_corrupt: bool,
    recovery_rebuilt: bool,
    previous_generation_retained: bool,
    query_after_recovery_passed: bool,
}

fn simulate_index_snapshot_corrupt_probe(
    scratch_dir: &Path,
) -> Result<IndexSnapshotCorruptProbeResult> {
    fs::create_dir_all(scratch_dir).map_err(|_| CliError::user("fault simulation probe failed"))?;
    let probe_dir = scratch_dir.join(format!(
        ".resume-ir-index-corrupt-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&probe_dir).map_err(|_| CliError::user("fault simulation probe failed"))?;

    let result = simulate_index_snapshot_corrupt_probe_dir(&probe_dir);
    let _ = fs::remove_dir_all(&probe_dir);
    result
}

fn simulate_index_snapshot_corrupt_probe_dir(
    probe_dir: &Path,
) -> Result<IndexSnapshotCorruptProbeResult> {
    const QUERY_TOKEN: &str = "SYNTHETIC_INDEX_CORRUPT_PRIVATE_TOKEN";
    let first_now = UnixTimestamp::from_unix_seconds(1_800_003_000);
    let input_root = probe_dir.join("input");
    fs::create_dir_all(&input_root).map_err(|_| CliError::user("fault simulation probe failed"))?;
    fs::write(
        input_root.join("synthetic-resume.txt"),
        format!(
            "SUMMARY\nSynthetic recovery candidate\nEXPERIENCE\nBuilt {QUERY_TOKEN}\nSKILLS\nRust"
        ),
    )
    .map_err(|_| CliError::user("fault simulation probe failed"))?;
    let data_directory_owner = import_processing::acquire_owner_for_mutation(
        probe_dir,
        import_processing::OfflineImportProcessingMutation::SyntheticFaultProbe,
    )?;

    let store = data_directory_owner
        .open_store()
        .map_err(|_| CliError::user("fault simulation probe failed"))?;
    let task = ImportTask {
        id: ImportTaskId::from_non_secret_parts(&["fault-probe", "snapshot-corrupt"]),
        root_path: path_string(&input_root),
        status: ImportTaskStatus::Queued,
        queued_at: first_now,
        started_at: None,
        finished_at: None,
        updated_at: first_now,
    };
    let import_options = ImportOptions {
        scan_profile: ScanProfile::Explicit,
        max_files: None,
        parse_workers: ImportParseWorkers::sequential(),
        index_writer_heap_bytes: ImportResourcePolicy::detect().index_writer_heap_bytes,
        linear_promotion: LinearPromotionPolicy::default(),
        search_vectorization: SearchPublicationVectorization::default(),
    };
    let processing_contract = import_processing::current_contract(&import_options)?;
    import_processing::normalize_orphaned_running_tasks(&store, first_now)?;
    import_processing::activate_contract(&store, &processing_contract, first_now)?;
    prepare_migration_rebuild_artifacts(&store, first_now)
        .map_err(|_| CliError::user("fault simulation probe failed"))?;
    import_processing::ensure_local_import_ready(
        &store,
        &processing_contract,
        first_now,
        &import_options.search_vectorization,
    )
    .map_err(|_| CliError::user("fault simulation probe failed"))?;
    let scope = new_import_scan_scope(
        &task,
        path_string(&input_root),
        StoreImportRootKind::Explicit,
        None,
        ScanProfile::Explicit,
        None,
        first_now,
    )?;
    import_processing::insert_new_configured_task_head(&store, &task, &scope, &processing_contract)
        .map_err(|_| CliError::user("fault simulation probe failed"))?;
    let _task_owner_lock = ImportTaskOwnerLock::acquire(probe_dir, &task.id)
        .map_err(|_| CliError::user("fault simulation probe failed"))?;
    let claimed_task =
        import_processing::claim_task_for_local_execution(&store, &task, current_timestamp()?)?;
    let imported = import_root_with_options(
        probe_dir,
        &store,
        &claimed_task,
        &input_root,
        first_now,
        import_options,
    )
    .map_err(|_| CliError::user("fault simulation probe failed"))?;
    if imported.searchable_documents != 1 {
        return Err(CliError::user("fault simulation probe failed"));
    }

    let corrupt_generation = store
        .search_projection_state()
        .map_err(|_| CliError::user("fault simulation probe failed"))?
        .generation
        .ok_or_else(|| CliError::user("fault simulation probe failed"))?;
    let corrupt_snapshot = probe_dir
        .join("search-index")
        .join("snapshots")
        .join(&corrupt_generation)
        .join("fulltext.snapshot.enc");
    fs::write(
        &corrupt_snapshot,
        b"not a valid encrypted full-text snapshot",
    )
    .map_err(|_| CliError::user("fault simulation probe failed"))?;

    let ready_generation_corrupt = QueryCoordinator::open(probe_dir)
        .and_then(|mut coordinator| coordinator.with_query(|_| Ok(())))
        .is_err();
    let recovery = reconcile_search_artifacts(
        &store,
        UnixTimestamp::from_unix_seconds(1_800_003_002),
        &SearchPublicationVectorization::default(),
    )
    .map_err(|_| CliError::user("fault simulation probe failed"))?;
    let recovered_generation = store
        .search_projection_state()
        .map_err(|_| CliError::user("fault simulation probe failed"))?
        .generation
        .ok_or_else(|| CliError::user("fault simulation probe failed"))?;

    let mut coordinator = QueryCoordinator::open(probe_dir)
        .map_err(|_| CliError::user("fault simulation probe failed"))?;
    let query_after_recovery_passed = coordinator
        .with_query(|scope| {
            let hits = scope.fulltext_candidates(QUERY_TOKEN, HitLimit::new(5)?, None)?;
            Ok(hits.len() == 1)
        })
        .map_err(|_| CliError::user("fault simulation probe failed"))?;
    let previous_generation_retained = corrupt_snapshot.exists();

    Ok(IndexSnapshotCorruptProbeResult {
        reproduced: ready_generation_corrupt
            && recovery.active_generation_rebuilt
            && corrupt_generation != recovered_generation
            && previous_generation_retained
            && query_after_recovery_passed,
        ready_generation_corrupt,
        recovery_rebuilt: recovery.active_generation_rebuilt,
        previous_generation_retained,
        query_after_recovery_passed,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MetadataMigrationProbeResult {
    reproduced: bool,
}

fn simulate_metadata_migration_failure_probe(
    scratch_dir: &Path,
) -> std::io::Result<MetadataMigrationProbeResult> {
    fs::create_dir_all(scratch_dir)?;
    let probe_dir = scratch_dir.join(format!(
        ".resume-ir-migration-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&probe_dir)?;

    let result = simulate_metadata_migration_failure_probe_dir(&probe_dir);
    let _ = fs::remove_dir_all(&probe_dir);
    result
}

fn simulate_metadata_migration_failure_probe_dir(
    probe_dir: &Path,
) -> std::io::Result<MetadataMigrationProbeResult> {
    let db_path = probe_dir.join("metadata.sqlite3");
    let connection = Connection::open(&db_path).map_err(sqlite_probe_error)?;
    connection
        .execute_batch(
            "\
            CREATE TABLE schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at_seconds INTEGER NOT NULL
            );
            INSERT INTO schema_migrations (version, applied_at_seconds)
            VALUES (1, 0);",
        )
        .map_err(sqlite_probe_error)?;
    drop(connection);

    let owner = match meta_store::DataDirectoryOwnerLease::try_acquire(probe_dir)
        .map_err(|_| metadata_migration_probe_error())?
    {
        meta_store::DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
        meta_store::DataDirectoryOwnerAcquisition::Contended => {
            return Err(metadata_migration_probe_error());
        }
    };
    Ok(MetadataMigrationProbeResult {
        reproduced: owner.open_store().is_err(),
    })
}

fn sqlite_probe_error(_: rusqlite::Error) -> std::io::Error {
    metadata_migration_probe_error()
}

fn metadata_migration_probe_error() -> std::io::Error {
    std::io::Error::other("metadata migration probe failed")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DaemonKillProbeResult {
    terminated: bool,
    restart_succeeded: bool,
}

fn simulate_daemon_kill_probe(
    scratch_dir: &Path,
    daemon_binary: &Path,
) -> std::io::Result<DaemonKillProbeResult> {
    fs::create_dir_all(scratch_dir)?;
    let probe_data_dir = scratch_dir.join(format!(
        ".resume-ir-daemon-kill-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&probe_data_dir)?;

    let result = simulate_daemon_kill_probe_dir(daemon_binary, &probe_data_dir);
    let _ = fs::remove_dir_all(&probe_data_dir);
    result
}

fn simulate_daemon_kill_probe_dir(
    daemon_binary: &Path,
    data_dir: &Path,
) -> std::io::Result<DaemonKillProbeResult> {
    let mut child = Command::new(daemon_binary)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("run")
        .arg("--foreground")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "daemon stdout was not captured",
        )
    })?;

    if !wait_for_daemon_ready(&mut child, stdout, Duration::from_secs(5))? {
        let _ = child.kill();
        let _ = child.wait();
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "daemon did not report ready",
        ));
    }

    child.kill()?;
    let status = child.wait()?;
    let restart_succeeded = daemon_restart_once_succeeds(daemon_binary, data_dir)?;

    Ok(DaemonKillProbeResult {
        terminated: !status.success(),
        restart_succeeded,
    })
}

fn wait_for_daemon_ready<R: Read + Send + 'static>(
    child: &mut Child,
    stdout: R,
    timeout: Duration,
) -> std::io::Result<bool> {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = sender.send(false);
                    return;
                }
                Ok(_) => {
                    if line.contains("resume-daemon foreground ready") {
                        let _ = sender.send(true);
                        return;
                    }
                }
                Err(_) => {
                    let _ = sender.send(false);
                    return;
                }
            }
        }
    });

    let deadline = Instant::now() + timeout;
    loop {
        match receiver.try_recv() {
            Ok(ready) => return Ok(ready),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(false),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if child.try_wait()?.is_some() {
            return Ok(false);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn daemon_restart_once_succeeds(daemon_binary: &Path, data_dir: &Path) -> std::io::Result<bool> {
    let output = Command::new(daemon_binary)
        .arg("--data-dir")
        .arg(data_dir)
        .arg("run")
        .arg("--foreground")
        .arg("--once")
        .output()?;

    Ok(output.status.success()
        && output.stderr.is_empty()
        && String::from_utf8_lossy(&output.stdout).contains("resume-daemon foreground ready"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OcrCrashProbeResult {
    reproduced: bool,
    probe_bytes: usize,
}

fn simulate_ocr_crash_probe(scratch_dir: &Path, ocr_command: &Path) -> Result<OcrCrashProbeResult> {
    fs::create_dir_all(scratch_dir).map_err(|_| CliError::user("fault simulation probe failed"))?;
    let client = LocalOcrCommandClient::new(
        LocalOcrCommandSpec::new(ocr_command, Vec::<String>::new(), "fault-simulate")
            .map_err(CliError::ocr)?,
    );
    let request = OcrPageRequest::new(
        RenderedPage::new(1, 300, OCR_CRASH_PROBE_BYTES.to_vec()).map_err(CliError::ocr)?,
        OcrOptions::new("eng", "balanced").map_err(CliError::ocr)?,
    )
    .map_err(CliError::ocr)?;
    let reproduced = match client.recognize_page(
        request,
        OcrWorkerBudget::new(/* page_timeout_ms */ 10_000).map_err(CliError::ocr)?,
        &CancellationToken::new(),
    ) {
        Ok(_) => false,
        Err(error) if error.kind() == OcrErrorKind::EngineFailed => true,
        Err(_) => return Err(CliError::user("fault simulation probe failed")),
    };

    Ok(OcrCrashProbeResult {
        reproduced,
        probe_bytes: OCR_CRASH_PROBE_BYTES.len(),
    })
}

fn fault_simulate_usage() -> &'static str {
    "usage: resume-cli fault-simulate --suite local-safe --json [--scratch-dir <path>] [--daemon-binary <path>] [--ocr-command <path>] OR resume-cli fault-simulate --case disk-space-low --required-bytes <n> --available-bytes <n> [--scratch-dir <path>] [--json] OR resume-cli fault-simulate --case permission-denied [--scratch-dir <path>] [--json] OR resume-cli fault-simulate --case file-lock [--scratch-dir <path>] [--json] OR resume-cli fault-simulate --case index-snapshot-corrupt [--scratch-dir <path>] [--json] OR resume-cli fault-simulate --case migration-failure [--scratch-dir <path>] [--json] OR resume-cli fault-simulate --case model-checksum --model-file <path> --expected-sha256 <hex> [--scratch-dir <path>] [--json] OR resume-cli fault-simulate --case daemon-kill --daemon-binary <path> [--scratch-dir <path>] [--json] OR resume-cli fault-simulate --case ocr-crash --ocr-command <path> [--scratch-dir <path>] [--json] OR resume-cli fault-simulate --case battery-mode --battery-state <battery|ac> [--scratch-dir <path>] [--json] OR resume-cli fault-simulate --case external-drive-disconnect --drive-state <disconnected|mounted> [--scratch-dir <path>] [--json]"
}

fn status_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let status_args = parse_status_args(data_dir, args)?;
    if let Some(watch) = status_args.watch_import {
        return status_watch_import_command(&watch.endpoint, &watch.token_file);
    }
    if let Some(endpoint) = status_args.ipc_endpoint {
        return status_ipc_command(&endpoint);
    }

    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(CliError::store)?;
    let latest_import_scan = store.latest_import_scan_scope().map_err(CliError::store)?;
    let scan_error_breakdown = store
        .import_scan_error_breakdown()
        .map_err(CliError::store)?;
    let ocr_task = store
        .worker_task_control(WorkerTaskKind::Ocr)
        .map_err(CliError::store)?;
    let index_diagnostic = inspect_search_index(data_dir, &store);
    let vector_diagnostic = inspect_vector_index(data_dir);

    println!("resume-ir status");
    println!("indexed documents: {}", summary.indexed_documents);
    println!("searchable documents: {}", summary.searchable_documents);
    println!("partial documents: {}", summary.partial_documents);
    println!("failed retryable: {}", summary.failed_retryable);
    println!("failed permanent: {}", summary.failed_permanent);
    println!("recovery queue: {}", summary.recovery_queue_depth);
    println!("ocr queue: {}", summary.ocr_queue_depth);
    println!("ocr jobs queued: {}", summary.ocr_jobs_queued);
    println!(
        "ocr page budget blocked: {}",
        summary.ocr_page_budget_blocked
    );
    if summary.ocr_page_budget_blocked > 0 {
        println!("ocr remediation: {}", OCR_PAGE_BUDGET_REMEDIATION);
    }
    println!(
        "ocr language unavailable: {}",
        summary.ocr_language_unavailable
    );
    if summary.ocr_language_unavailable > 0 {
        println!("ocr language remediation: {}", OCR_LANGUAGE_REMEDIATION);
    }
    println!("ocr task: {}", worker_task_status_label(ocr_task.paused));
    println!("embedding queue: {}", summary.embedding_queue_depth);
    println!("entity mentions: {}", summary.entity_mentions);
    println!("import tasks queued: {}", summary.import_tasks_queued);
    println!(
        "import tasks recoverable: {}",
        summary.import_tasks_recoverable
    );
    println!("import tasks cancelled: {}", summary.import_tasks_cancelled);
    println!("import scan scopes: {}", summary.import_scan_scopes);
    println!("import scan errors: {}", summary.import_scan_errors);
    print_import_scan_error_breakdown(&scan_error_breakdown);
    print_query_latency_summary(&summary.query_latency);
    if let Some(scope) = latest_import_scan.as_ref() {
        print_import_scan_progress(scope);
    }
    println!("active profile: balanced");
    println!("index health: {}", index_health_label(summary.index_health));
    println!(
        "last snapshot: {}",
        summary.last_snapshot_id.as_deref().unwrap_or("none")
    );
    println!("search index: {}", index_diagnostic.index_label());
    println!("vector index: {}", vector_diagnostic.index_label());
    println!("vector index vectors: {}", vector_diagnostic.vector_count());
    println!(
        "vector index tombstones: {}",
        vector_diagnostic.deleted_count()
    );

    Ok(())
}

struct StatusArgs {
    ipc_endpoint: Option<IpcStatusEndpoint>,
    watch_import: Option<ImportProgressWatchArgs>,
}

struct ImportProgressWatchArgs {
    endpoint: IpcImportProgressEndpoint,
    token_file: PathBuf,
}

fn print_import_scan_progress(scope: &ImportScanScope) {
    println!(
        "latest import scan profile: {}",
        store_import_scan_profile_label(scope.scan_profile)
    );
    println!("latest import files discovered: {}", scope.files_discovered);
    println!("latest import ignored entries: {}", scope.ignored_entries);
    println!("latest import scan errors: {}", scope.scan_errors);
    println!(
        "latest import searchable documents: {}",
        scope.searchable_documents
    );
    println!(
        "latest import ocr required documents: {}",
        scope.ocr_required_documents
    );
    println!("latest import ocr jobs queued: {}", scope.ocr_jobs_queued);
    println!("latest import failed documents: {}", scope.failed_documents);
    println!(
        "latest import deleted documents: {}",
        scope.deleted_documents
    );
    println!(
        "latest import scan budget: {}",
        import_scan_budget_progress_label(scope)
    );
}

fn import_scan_error_breakdown_label(breakdown: &[ImportScanErrorSummary]) -> String {
    if breakdown.is_empty() {
        return "none".to_string();
    }

    breakdown
        .iter()
        .map(|summary| {
            format!(
                "{}/{}={}",
                summary.kind.label(),
                summary.operation.label(),
                summary.count
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn print_import_scan_error_breakdown(breakdown: &[ImportScanErrorSummary]) {
    println!(
        "import scan error breakdown: {}",
        import_scan_error_breakdown_label(breakdown)
    );
}

fn print_import_scan_error_breakdown_json(breakdown: &[ImportScanErrorSummary], indent: &str) {
    for (index, summary) in breakdown.iter().enumerate() {
        let comma = if index + 1 == breakdown.len() {
            ""
        } else {
            ","
        };
        println!(
            "{indent}{{\"kind\": \"{}\", \"operation\": \"{}\", \"count\": {}}}{comma}",
            summary.kind.label(),
            summary.operation.label(),
            summary.count
        );
    }
}

fn print_query_latency_summary(summary: &QueryLatencySummary) {
    println!("query telemetry samples: {}", summary.sample_count);
    if summary.sample_count == 0 {
        return;
    }
    println!(
        "query latency p50 ms: {}",
        format_optional_u64(summary.p50_ms)
    );
    println!(
        "query latency p95 ms: {}",
        format_optional_u64(summary.p95_ms)
    );
    println!(
        "query latency p99 ms: {}",
        format_optional_u64(summary.p99_ms)
    );
    println!(
        "query latest result count: {}",
        format_optional_u64(summary.last_result_count)
    );
}

fn print_query_latency_json_summary(query_latency: &serde_json::Value) {
    let sample_count = query_latency
        .get("sample_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    println!("query telemetry samples: {sample_count}");
    if sample_count == 0 {
        return;
    }
    println!(
        "query latency p50 ms: {}",
        format_optional_u64(
            query_latency
                .get("p50_ms")
                .and_then(serde_json::Value::as_u64)
        )
    );
    println!(
        "query latency p95 ms: {}",
        format_optional_u64(
            query_latency
                .get("p95_ms")
                .and_then(serde_json::Value::as_u64)
        )
    );
    println!(
        "query latency p99 ms: {}",
        format_optional_u64(
            query_latency
                .get("p99_ms")
                .and_then(serde_json::Value::as_u64)
        )
    );
    println!(
        "query latest result count: {}",
        format_optional_u64(
            query_latency
                .get("last_result_count")
                .and_then(serde_json::Value::as_u64)
        )
    );
}

fn store_import_scan_profile_label(profile: StoreImportScanProfile) -> &'static str {
    match profile {
        StoreImportScanProfile::Explicit => "explicit",
        StoreImportScanProfile::Discovery => "discovery",
    }
}

fn import_scan_budget_progress_label(scope: &ImportScanScope) -> String {
    match (scope.scan_budget_observed, scope.scan_budget_limit) {
        (Some(observed), Some(limit)) => format!(
            "{observed}/{limit} exhausted={}",
            if scope.scan_budget_exhausted {
                "yes"
            } else {
                "no"
            }
        ),
        _ => "none".to_string(),
    }
}

fn parse_status_args(data_dir: &Path, args: &[String]) -> Result<StatusArgs> {
    let mut watch_import = false;
    let mut ipc_value = None;
    let mut ipc_token_file = None;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--watch-import" => {
                if watch_import {
                    return Err(CliError::usage(status_usage()));
                }
                watch_import = true;
                index += 1;
            }
            "--ipc" => {
                if ipc_value.is_some() {
                    return Err(CliError::usage(status_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(status_usage()));
                };
                ipc_value = Some(value.as_str());
                index += 2;
            }
            "--ipc-token-file" => {
                if ipc_token_file.is_some() {
                    return Err(CliError::usage(status_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(status_usage()));
                };
                ipc_token_file = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err(CliError::usage(status_usage())),
        }
    }

    if !watch_import && ipc_token_file.is_some() {
        return Err(CliError::usage(status_usage()));
    }

    let Some(ipc_value) = ipc_value else {
        if ipc_token_file.is_some() {
            return Err(CliError::usage(status_usage()));
        }
        return Ok(StatusArgs {
            ipc_endpoint: None,
            watch_import: None,
        });
    };

    if watch_import {
        if ipc_value == "auto" {
            if ipc_token_file.is_some() {
                return Err(CliError::usage(status_usage()));
            }
            let status_endpoint = discover_status_ipc_endpoint(data_dir)?;
            let endpoint = discover_import_progress_ipc_endpoint(data_dir)?;
            ensure_auto_ipc_same_daemon(status_endpoint.addr, endpoint.addr)?;
            verify_auto_ipc_status(&status_endpoint)?;
            return Ok(StatusArgs {
                ipc_endpoint: None,
                watch_import: Some(ImportProgressWatchArgs {
                    endpoint,
                    token_file: auto_ipc_token_file(data_dir),
                }),
            });
        }
        let token_file = ipc_token_file.ok_or_else(|| CliError::usage(status_usage()))?;
        return Ok(StatusArgs {
            ipc_endpoint: None,
            watch_import: Some(ImportProgressWatchArgs {
                endpoint: parse_import_progress_ipc_endpoint(ipc_value)?,
                token_file,
            }),
        });
    }

    if ipc_value == "auto" {
        return Ok(StatusArgs {
            ipc_endpoint: Some(discover_status_ipc_endpoint(data_dir)?),
            watch_import: None,
        });
    }

    Ok(StatusArgs {
        ipc_endpoint: Some(parse_status_ipc_endpoint(ipc_value)?),
        watch_import: None,
    })
}

fn status_usage() -> &'static str {
    "usage: resume-cli status [--watch-import] [--ipc <auto|http://127.0.0.1:port/status|/imports/progress>] [--ipc-token-file <path>]"
}

fn parse_status_ipc_endpoint(value: &str) -> Result<IpcStatusEndpoint> {
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| CliError::usage(status_usage()))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| CliError::usage(status_usage()))?;
    if path != "status" {
        return Err(CliError::usage(status_usage()));
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| CliError::usage(status_usage()))?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage("status ipc endpoint must be loopback"));
    }

    Ok(IpcStatusEndpoint { addr })
}

fn status_ipc_command(endpoint: &IpcStatusEndpoint) -> Result<()> {
    let body = request_status_ipc_body(endpoint)?;
    if !valid_status_ipc_body(&body) {
        return Err(CliError::user(
            "daemon status ipc returned invalid protocol",
        ));
    }
    render_ipc_status(&body);
    Ok(())
}

fn status_watch_import_command(
    endpoint: &IpcImportProgressEndpoint,
    token_file: &Path,
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .map_err(|_| CliError::user("unable to read daemon import progress ipc token"))?;
    let token = validate_daemon_ipc_token(&token, "daemon import progress ipc token is invalid")?;
    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon import progress ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|_| CliError::user("unable to configure daemon import progress ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon import progress ipc"))?;
    let request = format!(
        "GET /imports/progress HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
        endpoint.addr, token
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon import progress ipc"))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon import progress ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon import progress ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200 ") && !status_line.starts_with("HTTP/1.0 200 ") {
        return Err(CliError::user(
            "daemon import progress ipc returned an error",
        ));
    }
    render_import_progress_stream(body)?;
    Ok(())
}

fn request_status_ipc_body(endpoint: &IpcStatusEndpoint) -> Result<serde_json::Value> {
    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon status ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon status ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon status ipc"))?;
    let request = format!(
        "GET /status HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        endpoint.addr
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon status ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon status ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon status ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200 ") && !status_line.starts_with("HTTP/1.0 200 ") {
        return Err(CliError::user("daemon status ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon status ipc returned invalid json"))?;
    Ok(body)
}

fn verify_auto_ipc_status(endpoint: &IpcStatusEndpoint) -> Result<()> {
    let body = request_status_ipc_body(endpoint)
        .map_err(|_| CliError::user("daemon ipc auto-discovery is stale"))?;
    if !valid_status_ipc_body(&body) {
        return Err(CliError::user("daemon ipc auto-discovery is stale"));
    }
    Ok(())
}

fn valid_status_ipc_body(body: &serde_json::Value) -> bool {
    json_str(body, "schema_version") == Some("daemon.status.v2")
        && json_str(body, "process_state") == Some("ready")
        && matches!(
            json_str(body, "status"),
            Some("ok" | "repairing" | "degraded")
        )
}

fn render_ipc_status(body: &serde_json::Value) {
    println!("resume-ir status");
    println!("indexed documents: {}", json_u64(body, "indexed_documents"));
    println!(
        "searchable documents: {}",
        json_u64(body, "searchable_documents")
    );
    println!("partial documents: {}", json_u64(body, "partial_documents"));
    println!("failed retryable: {}", json_u64(body, "failed_retryable"));
    println!("failed permanent: {}", json_u64(body, "failed_permanent"));
    println!("recovery queue: {}", json_u64(body, "recovery_queue_depth"));
    println!("ocr queue: {}", json_u64(body, "ocr_queue_depth"));
    println!("ocr jobs queued: {}", json_u64(body, "ocr_jobs_queued"));
    let ocr_page_budget_blocked = json_u64(body, "ocr_page_budget_blocked");
    println!("ocr page budget blocked: {ocr_page_budget_blocked}");
    if ocr_page_budget_blocked > 0 {
        println!("ocr remediation: {}", OCR_PAGE_BUDGET_REMEDIATION);
    }
    let ocr_language_unavailable = json_u64(body, "ocr_language_unavailable");
    println!("ocr language unavailable: {ocr_language_unavailable}");
    if ocr_language_unavailable > 0 {
        println!("ocr language remediation: {}", OCR_LANGUAGE_REMEDIATION);
    }
    println!(
        "embedding queue: {}",
        json_u64(body, "embedding_queue_depth")
    );
    println!("entity mentions: {}", json_u64(body, "entity_mentions"));
    println!(
        "import tasks queued: {}",
        json_u64(body, "import_tasks_queued")
    );
    println!(
        "import tasks recoverable: {}",
        json_u64(body, "import_tasks_recoverable")
    );
    println!(
        "import tasks cancelled: {}",
        json_u64(body, "import_tasks_cancelled")
    );
    println!(
        "import scan scopes: {}",
        json_u64(body, "import_scan_scopes")
    );
    println!(
        "import scan errors: {}",
        json_u64(body, "import_scan_errors")
    );
    if let Some(query_latency) = body.get("query_latency") {
        print_query_latency_json_summary(query_latency);
    } else {
        println!("query telemetry samples: 0");
    }
    if let Some(latest_import) = body.get("latest_import_scan") {
        render_ipc_import_scan_progress(latest_import);
    }
    println!(
        "active profile: {}",
        json_str(body, "active_profile").unwrap_or("unknown")
    );
    println!(
        "index health: {}",
        json_str(body, "index_health").unwrap_or("unknown")
    );
    let snapshot_label = if json_bool(body, "snapshot_present") {
        "present"
    } else {
        "none"
    };
    println!("last snapshot: {snapshot_label}");
    println!("search index: daemon ipc (full-text state reported by daemon)");
}

fn render_import_progress_stream(body: &str) -> Result<()> {
    println!("resume-ir import progress stream");
    for line in body.lines().filter(|line| !line.trim().is_empty()) {
        let event: serde_json::Value = serde_json::from_str(line)
            .map_err(|_| CliError::user("daemon import progress ipc returned invalid json"))?;
        if json_str(&event, "schema_version") != Some("daemon.import_progress.v1") {
            return Err(CliError::user(
                "daemon import progress ipc returned invalid protocol",
            ));
        }
        println!(
            "import progress event: {}",
            json_str(&event, "event").unwrap_or("unknown")
        );
        if let Some(latest_import) = event.get("latest_import_scan") {
            render_ipc_import_scan_progress(latest_import);
        }
    }
    Ok(())
}

fn render_ipc_import_scan_progress(body: &serde_json::Value) {
    if !body.is_object() {
        return;
    }
    println!(
        "latest import scan profile: {}",
        json_str(body, "scan_profile").unwrap_or("unknown")
    );
    println!(
        "latest import files discovered: {}",
        json_u64(body, "files_discovered")
    );
    println!(
        "latest import ignored entries: {}",
        json_u64(body, "ignored_entries")
    );
    println!(
        "latest import scan errors: {}",
        json_u64(body, "scan_errors")
    );
    println!(
        "latest import searchable documents: {}",
        json_u64(body, "searchable_documents")
    );
    println!(
        "latest import ocr required documents: {}",
        json_u64(body, "ocr_required_documents")
    );
    println!(
        "latest import ocr jobs queued: {}",
        json_u64(body, "ocr_jobs_queued")
    );
    println!(
        "latest import failed documents: {}",
        json_u64(body, "failed_documents")
    );
    println!(
        "latest import deleted documents: {}",
        json_u64(body, "deleted_documents")
    );
    println!(
        "latest import scan budget: {}",
        ipc_import_scan_budget_progress_label(body)
    );
}

fn ipc_import_scan_budget_progress_label(body: &serde_json::Value) -> String {
    let observed = body
        .get("scan_budget_observed")
        .and_then(serde_json::Value::as_u64);
    let limit = body
        .get("scan_budget_limit")
        .and_then(serde_json::Value::as_u64);
    match (observed, limit) {
        (Some(observed), Some(limit)) => format!(
            "{observed}/{limit} exhausted={}",
            if json_bool(body, "scan_budget_exhausted") {
                "yes"
            } else {
                "no"
            }
        ),
        _ => "none".to_string(),
    }
}

fn json_u64(body: &serde_json::Value, key: &str) -> u64 {
    body.get(key)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

fn json_str<'a>(body: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    body.get(key).and_then(serde_json::Value::as_str)
}

fn json_bool(body: &serde_json::Value, key: &str) -> bool {
    body.get(key)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

struct IpcStatusEndpoint {
    addr: SocketAddr,
}

struct IpcImportEndpoint {
    addr: SocketAddr,
}

struct IpcImportCancelEndpoint {
    addr: SocketAddr,
}

struct IpcImportProgressEndpoint {
    addr: SocketAddr,
}

#[derive(Clone)]
struct IpcSearchEndpoint {
    addr: SocketAddr,
}

#[derive(Clone)]
struct IpcDetailEndpoint {
    addr: SocketAddr,
}

#[derive(Clone)]
struct IpcDeleteEndpoint {
    addr: SocketAddr,
}

fn auto_ipc_token_file(data_dir: &Path) -> PathBuf {
    data_dir.join(IPC_AUTH_TOKEN_FILE)
}

fn discover_status_ipc_endpoint(data_dir: &Path) -> Result<IpcStatusEndpoint> {
    parse_status_ipc_endpoint(&discover_ipc_url(data_dir, "status")?)
}

fn discover_import_ipc_endpoint(data_dir: &Path) -> Result<IpcImportEndpoint> {
    parse_import_ipc_endpoint(&discover_ipc_url(data_dir, "imports")?)
}

fn discover_import_cancel_ipc_endpoint(data_dir: &Path) -> Result<IpcImportCancelEndpoint> {
    parse_import_cancel_ipc_endpoint(&discover_ipc_url(data_dir, "import_cancel")?)
}

fn discover_import_progress_ipc_endpoint(data_dir: &Path) -> Result<IpcImportProgressEndpoint> {
    parse_import_progress_ipc_endpoint(&discover_ipc_url(data_dir, "import_progress")?)
}

fn discover_search_ipc_endpoint(data_dir: &Path) -> Result<IpcSearchEndpoint> {
    parse_search_ipc_endpoint(&discover_ipc_url(data_dir, "search")?)
}

fn discover_detail_ipc_endpoint(data_dir: &Path) -> Result<IpcDetailEndpoint> {
    parse_detail_ipc_endpoint(&discover_ipc_url(data_dir, "details")?)
}

fn discover_delete_ipc_endpoint(data_dir: &Path) -> Result<IpcDeleteEndpoint> {
    parse_delete_ipc_endpoint(&discover_ipc_url(data_dir, "delete")?)
}

fn ensure_auto_ipc_same_daemon(status_addr: SocketAddr, command_addr: SocketAddr) -> Result<()> {
    if status_addr != command_addr {
        return Err(CliError::user("daemon ipc auto-discovery is invalid"));
    }
    Ok(())
}

fn discover_ipc_url(data_dir: &Path, key: &str) -> Result<String> {
    let manifest_path = data_dir.join(IPC_ENDPOINT_FILE);
    let manifest_text = fs::read_to_string(&manifest_path)
        .map_err(|_| CliError::user("daemon ipc auto-discovery is unavailable"))?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text)
        .map_err(|_| CliError::user("daemon ipc auto-discovery is invalid"))?;
    let allowed_fields = [
        "schema_version",
        "instance_id",
        "owner_mode",
        "status",
        "diagnostics",
        "imports",
        "import_cancel",
        "import_control",
        "import_progress",
        "search",
        "search_batch",
        "details",
        "delete",
    ];
    let valid_shape = manifest.as_object().is_some_and(|object| {
        object.len() == allowed_fields.len()
            && object
                .keys()
                .all(|field| allowed_fields.contains(&field.as_str()))
    });
    let instance_id = json_str(&manifest, "instance_id");
    let owner_mode = json_str(&manifest, "owner_mode");
    if !valid_shape
        || json_str(&manifest, "schema_version") != Some(IPC_ENDPOINT_SCHEMA_VERSION)
        || !instance_id.is_some_and(valid_daemon_generation_value)
        || !matches!(owner_mode, Some("standalone" | "desktop_supervised"))
        || allowed_fields[3..]
            .iter()
            .any(|field| json_str(&manifest, field).is_none())
    {
        return Err(CliError::user("daemon ipc auto-discovery is invalid"));
    }
    let auth_text = fs::read_to_string(data_dir.join(IPC_AUTH_TOKEN_FILE))
        .map_err(|_| CliError::user("daemon ipc auto-discovery is unavailable"))?;
    let (auth_instance_id, _) =
        parse_daemon_ipc_auth(&auth_text, "daemon ipc auto-discovery is invalid")?;
    let stable_manifest = fs::read_to_string(manifest_path)
        .map_err(|_| CliError::user("daemon ipc auto-discovery is unavailable"))?;
    if auth_instance_id != instance_id.unwrap_or_default() || stable_manifest != manifest_text {
        return Err(CliError::user("daemon ipc auto-discovery is stale"));
    }
    json_str(&manifest, key)
        .map(str::to_string)
        .ok_or_else(|| CliError::user("daemon ipc auto-discovery is invalid"))
}

fn import_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let import_args = parse_import_args(args)?;
    if import_args.ipc_auto {
        let status_endpoint = discover_status_ipc_endpoint(data_dir)?;
        let endpoint = discover_import_ipc_endpoint(data_dir)?;
        ensure_auto_ipc_same_daemon(status_endpoint.addr, endpoint.addr)?;
        verify_auto_ipc_status(&status_endpoint)?;
        let token_file = auto_ipc_token_file(data_dir);
        return import_ipc_command_with_token_file(&endpoint, &token_file, &import_args);
    }
    if let Some(endpoint) = &import_args.ipc_endpoint {
        return import_ipc_command(endpoint, &import_args);
    }
    let data_directory_owner = import_processing::acquire_owner_for_mutation(
        data_dir,
        import_processing::OfflineImportProcessingMutation::DirectImport,
    )?;

    let requested_roots = expand_import_root_selection(&import_args.root_selection)?;
    let roots = canonical_import_roots(&requested_roots)?;
    let linear_promotion = import_args
        .linear_promotion_artifact
        .as_deref()
        .map(LinearPromotionPolicy::load_local)
        .unwrap_or_default();
    let import_options = ImportOptions {
        scan_profile: import_args.profile,
        max_files: import_args.max_files,
        parse_workers: import_args.parse_workers,
        index_writer_heap_bytes: import_args.index_writer_heap_bytes,
        linear_promotion: linear_promotion.clone(),
        search_vectorization: SearchPublicationVectorization::default(),
    };
    let processing_contract = import_processing::current_contract(&import_options)?;

    let store = open_owned_store(&data_directory_owner)?;
    let now = current_timestamp()?;
    import_processing::normalize_orphaned_running_tasks(&store, now)?;
    import_processing::activate_contract(&store, &processing_contract, now)?;
    prepare_migration_rebuild_artifacts(&store, now).map_err(CliError::import)?;
    import_processing::ensure_local_import_ready(
        &store,
        &processing_contract,
        now,
        &import_options.search_vectorization,
    )?;
    if !import_args.enqueue {
        reconcile_search_artifacts(&store, now, &SearchPublicationVectorization::default())
            .map_err(CliError::import)?;
    }
    let requested_heads = roots
        .iter()
        .map(|root| {
            let canonical_root_path = path_string(&root.canonical);
            let task = ImportTask {
                id: new_import_task_id()?,
                root_path: canonical_root_path,
                status: ImportTaskStatus::Queued,
                queued_at: now,
                started_at: None,
                finished_at: None,
                updated_at: now,
            };
            let scope = initial_import_scan_scope(&task, root, &import_args, now)?;
            Ok((task, scope))
        })
        .collect::<Result<Vec<_>>>()?;
    let requests = requested_heads
        .iter()
        .map(|(task, scope)| ImportRootTaskHeadRequest::Configured {
            task,
            scope,
            processing_contract: &processing_contract,
        })
        .collect::<Vec<_>>();
    let outcomes = match store
        .coordinate_import_root_task_heads(&requests)
        .map_err(CliError::store)?
    {
        ImportRootTaskHeadBatchOutcome::Committed { outcomes } => outcomes,
        ImportRootTaskHeadBatchOutcome::Rejected(
            ImportRootTaskHeadBatchRejection::RunningTaskConflict,
        ) => return Err(CliError::user("import task is already running")),
        ImportRootTaskHeadBatchOutcome::Rejected(ImportRootTaskHeadBatchRejection::RootPaused) => {
            return Err(CliError::user("managed root is paused"));
        }
        ImportRootTaskHeadBatchOutcome::Rejected(
            ImportRootTaskHeadBatchRejection::MigrationRebuildSuperseded,
        ) => {
            return Err(CliError::user(
                "offline import is blocked until migration rebuild completes",
            ));
        }
    };
    let tasks = outcomes
        .into_iter()
        .map(|outcome| match outcome {
            ImportRootTaskHeadOutcome::HeadInserted { task, .. }
            | ImportRootTaskHeadOutcome::HeadPromoted { task, .. }
            | ImportRootTaskHeadOutcome::HeadRetained { task, .. } => Ok(task),
            ImportRootTaskHeadOutcome::RunningTaskConflict
            | ImportRootTaskHeadOutcome::RootPaused
            | ImportRootTaskHeadOutcome::MigrationRebuildSuperseded => {
                Err(CliError::user("import root coordination failed"))
            }
        })
        .collect::<Result<Vec<_>>>()?;

    let mut summary = ImportSummary::default();

    if import_args.enqueue {
        let task_ids = tasks
            .iter()
            .map(|task| task.id.to_string())
            .collect::<Vec<_>>()
            .join(",");

        println!("import task submitted");
        if tasks.len() == 1 {
            println!("task id: {}", tasks[0].id);
        } else {
            println!("task ids: {task_ids}");
        }
        println!("status: queued");
        println!("scan profile: {}", import_args.profile.label());
        println!("roots queued: {}", roots.len());
        println!(
            "scan file limit: {}",
            import_args
                .max_files
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string())
        );
        println!(
            "import hardware tier: {}",
            import_args.hardware_tier.label()
        );
        println!(
            "import private/anonymous budget MiB: {}",
            import_args.max_private_or_anonymous_mb
        );
        println!(
            "import index writer heap MiB: {}",
            bytes_to_mib(import_args.index_writer_heap_bytes)
        );
        return Ok(());
    }

    let import_started = Instant::now();
    for (task, root) in tasks.iter().zip(roots.iter()) {
        let _task_owner_lock = ImportTaskOwnerLock::acquire(data_dir, &task.id)
            .map_err(|_| CliError::user("unable to acquire import task owner lock"))?;
        let claimed_task =
            import_processing::claim_task_for_local_execution(&store, task, current_timestamp()?)?;
        let root_offset = import_started.elapsed();
        let root_summary = import_root_with_options(
            data_dir,
            &store,
            &claimed_task,
            &root.canonical,
            now,
            import_options.clone(),
        )
        .map_err(CliError::import)?;
        merge_import_summary(&mut summary, root_summary, root_offset);
    }

    let task_ids = tasks
        .iter()
        .map(|task| task.id.to_string())
        .collect::<Vec<_>>()
        .join(",");

    println!("import task submitted");
    if tasks.len() == 1 {
        println!("task id: {}", tasks[0].id);
    } else {
        println!("task ids: {task_ids}");
    }
    println!("status: completed");
    println!(
        "resume classifier promotion: {}",
        match (
            import_args.linear_promotion_artifact.is_some(),
            linear_promotion.enabled(),
        ) {
            (_, true) => "enabled",
            (true, false) => "fail_closed_disabled",
            (false, false) => "not_configured",
        }
    );
    println!("scan profile: {}", import_args.profile.label());
    println!("roots scanned: {}", roots.len());
    println!("files discovered: {}", summary.files_discovered);
    println!("content bytes read: {}", summary.content_bytes_read);
    println!("searchable documents: {}", summary.searchable_documents);
    println!("ocr required documents: {}", summary.ocr_required_documents);
    println!("ocr jobs queued: {}", summary.ocr_jobs_queued);
    println!("failed documents: {}", summary.failed_documents);
    println!("deleted documents: {}", summary.deleted_documents);
    println!("scan errors: {}", summary.scan_errors);
    println!(
        "scan budget exhausted: {}",
        if summary.scan_budget.is_some_and(|budget| budget.exhausted) {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "scan file limit: {}",
        import_args
            .max_files
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
    println!(
        "import hardware tier: {}",
        import_args.hardware_tier.label()
    );
    println!(
        "import private/anonymous budget MiB: {}",
        import_args.max_private_or_anonymous_mb
    );
    println!(
        "import index writer heap MiB: {}",
        bytes_to_mib(import_args.index_writer_heap_bytes)
    );
    print_import_throughput(&summary);
    print_import_milestone_timings(&summary);
    print_import_stage_timings(&summary);
    print_import_worker_metrics(&summary);

    Ok(())
}

fn witness_command(args: &[String]) -> Result<()> {
    let witness_args = parse_witness_args(args)?;
    let (source_roots, scan_profile) = expand_witness_root_selection(&witness_args.root_selection)?;
    let selection = collect_witness_inputs(&source_roots, witness_args.max_files, scan_profile)?;
    let temp_dirs = WitnessTempDirs::create()?;
    copy_witness_inputs(&selection.selected, &temp_dirs.input_root)?;
    let (summary, witness_ocr, witness_benchmark_corpus, witness_fields, witness_search) = {
        let data_directory_owner = import_processing::acquire_owner_for_mutation(
            &temp_dirs.data_dir,
            import_processing::OfflineImportProcessingMutation::PrivateWitness,
        )?;
        let store = open_owned_store(&data_directory_owner)?;
        let now = current_timestamp()?;
        let task = ImportTask {
            id: new_import_task_id()?,
            root_path: path_string(&temp_dirs.input_root),
            status: ImportTaskStatus::Queued,
            queued_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
        };
        let import_options = ImportOptions {
            scan_profile,
            max_files: None,
            parse_workers: ImportParseWorkers::default(),
            index_writer_heap_bytes: ImportResourcePolicy::detect().index_writer_heap_bytes,
            linear_promotion: LinearPromotionPolicy::default(),
            search_vectorization: SearchPublicationVectorization::default(),
        };
        let processing_contract = import_processing::current_contract(&import_options)?;
        import_processing::normalize_orphaned_running_tasks(&store, now)?;
        import_processing::activate_contract(&store, &processing_contract, now)?;
        prepare_migration_rebuild_artifacts(&store, now).map_err(CliError::import)?;
        import_processing::ensure_local_import_ready(
            &store,
            &processing_contract,
            now,
            &import_options.search_vectorization,
        )?;
        let scope = new_import_scan_scope(
            &task,
            path_string(&temp_dirs.input_root),
            StoreImportRootKind::Explicit,
            None,
            scan_profile,
            None,
            now,
        )?;
        import_processing::insert_new_configured_task_head(
            &store,
            &task,
            &scope,
            &processing_contract,
        )?;
        let _task_owner_lock = ImportTaskOwnerLock::acquire(&temp_dirs.data_dir, &task.id)
            .map_err(|_| CliError::user("unable to acquire import task owner lock"))?;
        let claimed_task =
            import_processing::claim_task_for_local_execution(&store, &task, current_timestamp()?)?;
        let summary = import_root_with_options(
            &temp_dirs.data_dir,
            &store,
            &claimed_task,
            &temp_dirs.input_root,
            now,
            import_options,
        )
        .map_err(CliError::import)?;
        let witness_ocr = if witness_args.run_ocr {
            run_witness_ocr_jobs(
                &temp_dirs.data_dir,
                &store,
                &witness_args.ocr_worker_args,
                witness_args.ocr_max_documents,
                now,
            )?
        } else {
            WitnessOcrStatus::NotRequested
        };
        let witness_read_store = open_store(&temp_dirs.data_dir)?;
        let witness_benchmark_corpus = if witness_args.probe_benchmark_corpus {
            WitnessBenchmarkCorpusStatus::Completed {
                summary: benchmark_corpus_summary(&temp_dirs.data_dir, &witness_read_store)?,
            }
        } else {
            WitnessBenchmarkCorpusStatus::NotRequested
        };
        let witness_fields = if witness_args.probe_fields {
            run_witness_field_probe(&witness_read_store)?
        } else {
            WitnessFieldStatus::NotRequested
        };
        let witness_search = if witness_args.probe_search {
            run_witness_search_probe(&temp_dirs.data_dir, &witness_read_store)?
        } else {
            WitnessSearchStatus::NotRequested
        };
        (
            summary,
            witness_ocr,
            witness_benchmark_corpus,
            witness_fields,
            witness_search,
        )
    };
    let private_data_removed = temp_dirs.cleanup();

    println!("resume-ir local witness");
    println!("source root: <redacted>");
    println!(
        "root preset: {}",
        witness_args.root_selection.preset_label().unwrap_or("none")
    );
    println!("scan profile: {}", scan_profile.label());
    println!("formats: pdf,docx,doc");
    println!("files selected: {}", selection.selected.len());
    println!(
        "unsupported entries skipped: {}",
        selection.unsupported_entries
    );
    println!("filesystem scan errors: {}", selection.scan_errors);
    println!(
        "scan budget exhausted: {}",
        yes_no(selection.budget_exhausted)
    );
    println!("witness import status: completed");
    println!("searchable documents: {}", summary.searchable_documents);
    println!("ocr required documents: {}", summary.ocr_required_documents);
    println!("ocr jobs queued: {}", summary.ocr_jobs_queued);
    println!("failed documents: {}", summary.failed_documents);
    print_import_failure_counts(&summary);
    print_witness_ocr_status(&witness_ocr);
    print_witness_benchmark_corpus_status(&witness_benchmark_corpus);
    print_witness_field_status(&witness_fields);
    print_witness_search_status(&witness_search);
    println!(
        "private witness data: {}",
        if private_data_removed {
            "removed"
        } else {
            "cleanup_failed"
        }
    );

    if !private_data_removed {
        return Err(CliError::user(
            "unable to remove private local witness data",
        ));
    }

    Ok(())
}

fn print_import_failure_counts(summary: &ImportSummary) {
    for kind in WITNESS_IMPORT_FAILURE_KINDS {
        println!(
            "import failure {}: {}",
            kind.label(),
            summary.failure_counts.get(*kind)
        );
    }
}

fn parse_witness_args(args: &[String]) -> Result<WitnessArgs> {
    let mut root = None;
    let mut root_preset = None;
    let mut max_files = WITNESS_DEFAULT_MAX_FILES;
    let mut run_ocr = false;
    let mut probe_search = false;
    let mut probe_fields = false;
    let mut probe_benchmark_corpus = false;
    let mut seen_ocr_option = false;
    let mut ocr_worker_args = default_ocr_worker_args();
    let mut ocr_max_documents = None;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                if root_preset.is_some() {
                    return Err(witness_usage());
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                if root.is_some() {
                    return Err(witness_usage());
                }
                root = Some(PathBuf::from(value));
                index += 2;
            }
            "--root-preset" => {
                if root.is_some() || root_preset.is_some() {
                    return Err(witness_usage());
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                root_preset = Some(parse_root_preset(value)?);
                index += 2;
            }
            "--max-files" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                max_files = value
                    .parse::<usize>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(witness_usage)?;
                index += 2;
            }
            "--run-ocr" => {
                run_ocr = true;
                index += 1;
            }
            "--probe-search" => {
                probe_search = true;
                index += 1;
            }
            "--probe-fields" => {
                probe_fields = true;
                index += 1;
            }
            "--probe-benchmark-corpus" => {
                probe_benchmark_corpus = true;
                index += 1;
            }
            "--ocr-command" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                if ocr_worker_args.command.is_some() {
                    return Err(witness_usage());
                }
                ocr_worker_args.command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-tesseract-command" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                if ocr_worker_args.tesseract_command.is_some() {
                    return Err(witness_usage());
                }
                ocr_worker_args.tesseract_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-render-command" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                if ocr_worker_args.render_command.is_some() {
                    return Err(witness_usage());
                }
                ocr_worker_args.render_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-pdftoppm-command" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                if ocr_worker_args.pdftoppm_command.is_some() {
                    return Err(witness_usage());
                }
                ocr_worker_args.pdftoppm_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--ocr-engine-profile" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                if value.trim().is_empty() {
                    return Err(witness_usage());
                }
                ocr_worker_args.engine_profile = value.clone();
                index += 2;
            }
            "--ocr-lang" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                if value.trim().is_empty() {
                    return Err(witness_usage());
                }
                ocr_worker_args.lang = value.clone();
                index += 2;
            }
            "--ocr-profile" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                if value.trim().is_empty() {
                    return Err(witness_usage());
                }
                ocr_worker_args.profile = value.clone();
                index += 2;
            }
            "--ocr-render-dpi" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                ocr_worker_args.render_dpi = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(witness_usage)?;
                index += 2;
            }
            "--ocr-page-timeout-ms" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                ocr_worker_args.page_timeout_ms = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(witness_usage)?;
                index += 2;
            }
            "--ocr-max-pages-per-document" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                ocr_worker_args.max_pages_per_document = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(witness_usage)?;
                index += 2;
            }
            "--ocr-max-documents" => {
                seen_ocr_option = true;
                let Some(value) = args.get(index + 1) else {
                    return Err(witness_usage());
                };
                if ocr_max_documents.is_some() {
                    return Err(witness_usage());
                }
                ocr_max_documents = Some(
                    value
                        .parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(witness_usage)?,
                );
                index += 2;
            }
            _ => return Err(witness_usage()),
        }
    }

    if seen_ocr_option && !run_ocr {
        return Err(witness_usage());
    }
    if ocr_worker_args.command.is_some() && ocr_worker_args.tesseract_command.is_some() {
        return Err(witness_usage());
    }
    if ocr_worker_args.render_command.is_some() && ocr_worker_args.pdftoppm_command.is_some() {
        return Err(witness_usage());
    }

    let root_selection = if let Some(root) = root {
        WitnessRootSelection::Explicit(root)
    } else if let Some(root_preset) = root_preset {
        WitnessRootSelection::Preset(root_preset)
    } else {
        return Err(witness_usage());
    };

    Ok(WitnessArgs {
        root_selection,
        max_files,
        run_ocr,
        probe_search,
        probe_fields,
        probe_benchmark_corpus,
        ocr_max_documents,
        ocr_worker_args,
    })
}

fn witness_usage_text() -> &'static str {
    "usage: resume-cli witness (--root <path>|--root-preset local-discovery) [--max-files <count>] [--probe-search] [--probe-fields] [--probe-benchmark-corpus] [--run-ocr [--ocr-max-documents <n>] [--ocr-command <path>|--ocr-tesseract-command <path>] [--ocr-render-command <path>|--ocr-pdftoppm-command <path>] [--ocr-engine-profile <name>] [--ocr-lang <lang>] [--ocr-profile <profile>] [--ocr-render-dpi <dpi>] [--ocr-page-timeout-ms <ms>] [--ocr-max-pages-per-document <n>]]"
}

fn witness_usage() -> CliError {
    CliError::usage(witness_usage_text())
}

fn default_ocr_worker_args() -> OcrWorkerArgs {
    OcrWorkerArgs {
        command: None,
        tesseract_command: None,
        render_command: None,
        pdftoppm_command: None,
        engine_profile: "local-command".to_string(),
        lang: "eng".to_string(),
        profile: "balanced".to_string(),
        render_dpi: 300,
        page_timeout_ms: 30_000,
        max_pages_per_document: DEFAULT_OCR_MAX_PAGES_PER_DOCUMENT,
    }
}

fn run_witness_ocr_jobs(
    data_dir: &Path,
    store: &OwnedMetaStore,
    worker_args: &OcrWorkerArgs,
    max_documents: Option<usize>,
    now: UnixTimestamp,
) -> Result<WitnessOcrStatus> {
    match ocr_preclaim_decision(store).map_err(CliError::import)? {
        OcrPreclaimDecision::Ready => {}
        OcrPreclaimDecision::NotReady(_) => {
            return Ok(WitnessOcrStatus::Blocked {
                reason: "search publication is not ready",
                documents_processed: 0,
                documents_failed: 0,
                cache_writes: 0,
                cache_hits: 0,
                budget_exhausted: false,
            });
        }
    }
    if worker_args.command.is_none() && worker_args.tesseract_command.is_none() {
        return Ok(WitnessOcrStatus::Blocked {
            reason: "local OCR command not configured",
            documents_processed: 0,
            documents_failed: 0,
            cache_writes: 0,
            cache_hits: 0,
            budget_exhausted: false,
        });
    }

    let mut documents_processed = 0_usize;
    let mut documents_failed = 0_usize;
    let mut cache_writes = 0_usize;
    let mut cache_hits = 0_usize;

    loop {
        let documents_attempted = documents_processed + documents_failed;
        if max_documents.is_some_and(|limit| documents_attempted >= limit) {
            let summary = store.status_summary().map_err(CliError::store)?;
            return Ok(WitnessOcrStatus::Completed {
                documents_processed,
                documents_failed,
                cache_writes,
                cache_hits,
                budget_exhausted: summary.ocr_jobs_queued > 0,
            });
        }

        let Some(job) = store.claim_next_ocr_job(now).map_err(CliError::store)? else {
            return Ok(WitnessOcrStatus::Completed {
                documents_processed,
                documents_failed,
                cache_writes,
                cache_hits,
                budget_exhausted: false,
            });
        };

        match run_claimed_ocr_job(data_dir, store, &job, worker_args, now) {
            Ok(summary) => {
                documents_processed += summary.documents_processed;
                cache_writes += summary.cache_writes;
                cache_hits += summary.cache_hits;
            }
            Err(_) => {
                documents_failed += 1;
                store
                    .finish_ocr_attempt_failure(&job, OcrAttemptFailure::Retryable, now)
                    .map_err(CliError::store)?;
                if max_documents.is_some() {
                    continue;
                }
                return Ok(WitnessOcrStatus::Blocked {
                    reason: "local OCR command failed or unavailable",
                    documents_processed,
                    documents_failed,
                    cache_writes,
                    cache_hits,
                    budget_exhausted: false,
                });
            }
        }
    }
}

fn print_witness_ocr_status(status: &WitnessOcrStatus) {
    match status {
        WitnessOcrStatus::NotRequested => {
            println!("witness ocr status: not_requested");
            println!("ocr documents processed: 0");
            println!("ocr documents failed: 0");
            println!("ocr cache writes: 0");
            println!("ocr cache hits: 0");
            println!("ocr document budget exhausted: no");
        }
        WitnessOcrStatus::Completed {
            documents_processed,
            documents_failed,
            cache_writes,
            cache_hits,
            budget_exhausted,
        } => {
            println!("witness ocr status: completed");
            println!("ocr documents processed: {documents_processed}");
            println!("ocr documents failed: {documents_failed}");
            println!("ocr cache writes: {cache_writes}");
            println!("ocr cache hits: {cache_hits}");
            println!(
                "ocr document budget exhausted: {}",
                yes_no(*budget_exhausted)
            );
        }
        WitnessOcrStatus::Blocked {
            reason,
            documents_processed,
            documents_failed,
            cache_writes,
            cache_hits,
            budget_exhausted,
        } => {
            println!("witness ocr status: blocked");
            println!("ocr block reason: {reason}");
            println!("ocr documents processed: {documents_processed}");
            println!("ocr documents failed: {documents_failed}");
            println!("ocr cache writes: {cache_writes}");
            println!("ocr cache hits: {cache_hits}");
            println!(
                "ocr document budget exhausted: {}",
                yes_no(*budget_exhausted)
            );
        }
    }
}

fn print_witness_benchmark_corpus_status(status: &WitnessBenchmarkCorpusStatus) {
    match status {
        WitnessBenchmarkCorpusStatus::NotRequested => {
            println!("witness benchmark corpus status: not_requested");
            println!("benchmark corpus documents: 0");
            println!("benchmark corpus searchable documents: 0");
            println!("benchmark corpus vector indexed documents: 0");
            println!("benchmark corpus active vector documents: 0");
            println!("benchmark corpus vector count: 0");
            println!("benchmark corpus vector tombstones: 0");
            println!("benchmark corpus vector index: unavailable");
            println!("benchmark corpus vector backend: none");
            println!("benchmark corpus hot index fully covered: no");
        }
        WitnessBenchmarkCorpusStatus::Completed { summary } => {
            println!("witness benchmark corpus status: completed");
            println!("benchmark corpus documents: {}", summary.document_count);
            println!(
                "benchmark corpus searchable documents: {}",
                summary.searchable_document_count
            );
            println!(
                "benchmark corpus vector indexed documents: {}",
                summary.vector_indexed_document_count
            );
            println!(
                "benchmark corpus active vector documents: {}",
                summary.active_vector_document_count
            );
            println!("benchmark corpus vector count: {}", summary.vector_count);
            println!(
                "benchmark corpus vector tombstones: {}",
                summary.vector_deleted_count
            );
            println!(
                "benchmark corpus vector index: {}",
                summary.vector_index_state
            );
            println!(
                "benchmark corpus vector backend: {}",
                summary.vector_search_backend
            );
            println!(
                "benchmark corpus hot index fully covered: {}",
                yes_no(summary.hot_index_fully_covered)
            );
        }
    }
}

fn run_witness_field_probe(store: &ReadMetaStore) -> Result<WitnessFieldStatus> {
    let mut documents = 0_usize;
    let mut mentions = 0_usize;
    let mut counts = WitnessFieldCounts::default();

    for document in store.visible_documents().map_err(CliError::store)? {
        let mut document_has_mentions = false;
        for (entity_type, count) in store
            .visible_entity_type_counts_for_document(&document.id)
            .map_err(CliError::store)?
        {
            let Some(label) = witness_field_label(&entity_type) else {
                continue;
            };
            counts.add(label, count);
            mentions += count;
            document_has_mentions = true;
        }

        if document_has_mentions {
            documents += 1;
        }
    }

    if mentions == 0 {
        Ok(WitnessFieldStatus::Blocked {
            reason: "no witness field mentions",
            documents,
            mentions,
            counts,
        })
    } else {
        Ok(WitnessFieldStatus::Completed {
            documents,
            mentions,
            counts,
        })
    }
}

fn witness_field_label(entity_type: &EntityType) -> Option<&'static str> {
    match entity_type {
        EntityType::Name => Some("name"),
        EntityType::Email => Some("email"),
        EntityType::Phone => Some("phone"),
        EntityType::WeChat => Some("wechat"),
        EntityType::School => Some("school"),
        EntityType::SchoolTier => Some("school_tier"),
        EntityType::Degree => Some("degree"),
        EntityType::Major => Some("major"),
        EntityType::Company => Some("company"),
        EntityType::Title => Some("title"),
        EntityType::Education => Some("education"),
        EntityType::Skills | EntityType::Skill => Some("skill"),
        EntityType::Certificate => Some("certificate"),
        EntityType::Date => Some("date"),
        EntityType::DateRange => Some("date_range"),
        EntityType::YearsExperience => Some("years_experience"),
        EntityType::Location => Some("location"),
        EntityType::Other(_) => None,
    }
}

fn print_witness_field_status(status: &WitnessFieldStatus) {
    match status {
        WitnessFieldStatus::NotRequested => {
            println!("witness field status: not_requested");
            println!("field probe documents: 0");
            println!("field probe mentions: 0");
            print_witness_field_counts(&WitnessFieldCounts::default());
        }
        WitnessFieldStatus::Completed {
            documents,
            mentions,
            counts,
        } => {
            println!("witness field status: completed");
            println!("field probe documents: {documents}");
            println!("field probe mentions: {mentions}");
            print_witness_field_counts(counts);
        }
        WitnessFieldStatus::Blocked {
            reason,
            documents,
            mentions,
            counts,
        } => {
            println!("witness field status: blocked");
            println!("field block reason: {reason}");
            println!("field probe documents: {documents}");
            println!("field probe mentions: {mentions}");
            print_witness_field_counts(counts);
        }
    }
}

fn print_witness_field_counts(counts: &WitnessFieldCounts) {
    for label in WITNESS_FIELD_LABELS {
        println!("field probe {label} mentions: {}", counts.get(label));
    }
}

fn run_witness_search_probe(data_dir: &Path, store: &ReadMetaStore) -> Result<WitnessSearchStatus> {
    let candidates = witness_search_probe_candidates(store)?;
    if candidates.is_empty() {
        return Ok(WitnessSearchStatus::Blocked {
            reason: "no searchable witness text",
            hits: 0,
        });
    }

    let mut coordinator = match QueryCoordinator::open(data_dir) {
        Ok(coordinator) => coordinator,
        Err(_) => {
            return Ok(WitnessSearchStatus::Blocked {
                reason: "search service unavailable",
                hits: 0,
            });
        }
    };

    let mut best_hits = 0_usize;
    for query in candidates {
        let hits = coordinator
            .with_query(|scope| {
                let candidates = scope.fulltext_candidates(
                    &query,
                    HitLimit::new(WITNESS_SEARCH_PROBE_LIMIT)?,
                    None,
                )?;
                let projections = candidates
                    .iter()
                    .map(|candidate| candidate.projection.clone())
                    .collect::<Vec<_>>();
                scope.hydrate_exact_hits(&projections)
            })
            .map_err(search_runtime_cli_error)?;
        best_hits = best_hits.max(hits.len());
        if !hits.is_empty() {
            return Ok(WitnessSearchStatus::Completed { hits: hits.len() });
        }
    }

    Ok(WitnessSearchStatus::Blocked {
        reason: "search probe returned no visible results",
        hits: best_hits,
    })
}

fn witness_search_probe_candidates(store: &ReadMetaStore) -> Result<Vec<String>> {
    let mut candidates = Vec::new();

    for document in store.visible_documents().map_err(CliError::store)? {
        let Some(projection) = store
            .active_search_projection_for_document(&document.id)
            .map_err(CliError::store)?
        else {
            continue;
        };
        let Some(version) = store
            .resume_version_by_id(&projection.resume_version_id)
            .map_err(CliError::store)?
        else {
            return Err(CliError::user("active search projection is invalid"));
        };
        if let Some(text) = version.clean_text.as_deref() {
            collect_witness_search_tokens(text, &mut candidates);
            if candidates.len() >= WITNESS_SEARCH_PROBE_MAX_CANDIDATES {
                return Ok(candidates);
            }
        }
    }

    Ok(candidates)
}

fn collect_witness_search_tokens(text: &str, candidates: &mut Vec<String>) {
    let mut token = String::new();

    for character in text.chars() {
        if character.is_alphabetic() {
            token.push(character);
            continue;
        }

        push_witness_search_token(&mut token, candidates);
        if candidates.len() >= WITNESS_SEARCH_PROBE_MAX_CANDIDATES {
            return;
        }
    }

    push_witness_search_token(&mut token, candidates);
}

fn push_witness_search_token(token: &mut String, candidates: &mut Vec<String>) {
    let char_count = token.chars().count();
    let min_chars = if token.is_ascii() { 4 } else { 2 };
    if char_count >= min_chars && candidates.len() < WITNESS_SEARCH_PROBE_MAX_CANDIDATES {
        let candidate = token.chars().take(32).collect::<String>();
        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }
    token.clear();
}

fn print_witness_search_status(status: &WitnessSearchStatus) {
    match status {
        WitnessSearchStatus::NotRequested => {
            println!("witness search status: not_requested");
            println!("search probe hits: 0");
        }
        WitnessSearchStatus::Completed { hits } => {
            println!("witness search status: completed");
            println!("search probe hits: {hits}");
        }
        WitnessSearchStatus::Blocked { reason, hits } => {
            println!("witness search status: blocked");
            println!("search block reason: {reason}");
            println!("search probe hits: {hits}");
        }
    }
}

fn canonical_witness_root(root: &Path) -> Result<PathBuf> {
    let metadata = fs::metadata(root)
        .map_err(|_| CliError::user("witness root must exist and be a directory"))?;
    if !metadata.is_dir() {
        return Err(CliError::user("witness root must exist and be a directory"));
    }
    fs::canonicalize(root).map_err(|_| CliError::user("witness root must exist and be a directory"))
}

fn expand_witness_root_selection(
    root_selection: &WitnessRootSelection,
) -> Result<(Vec<PathBuf>, ScanProfile)> {
    match root_selection {
        WitnessRootSelection::Explicit(root) => {
            Ok((vec![canonical_witness_root(root)?], ScanProfile::Explicit))
        }
        WitnessRootSelection::Preset(RootPreset::LocalDiscovery) => {
            Ok((local_discovery_roots()?, ScanProfile::Discovery))
        }
    }
}

fn collect_witness_inputs(
    roots: &[PathBuf],
    max_files: usize,
    scan_profile: ScanProfile,
) -> Result<WitnessSelection> {
    let mut selection = WitnessSelection::default();

    for (root_index, root) in roots.iter().enumerate() {
        if selection.selected.len() >= max_files {
            selection.budget_exhausted = true;
            return Ok(selection);
        }
        let remaining_files = max_files - selection.selected.len();
        let report = crawl_directory_with_options(
            root,
            CrawlerScanOptions {
                profile: scan_profile,
                max_files: Some(remaining_files),
            },
        )
        .map_err(|_| CliError::user("unable to scan private witness root"))?;
        let root_budget_exhausted = report.budget_exhausted.is_some();
        selection.scan_errors += report.errors.len();
        selection.unsupported_entries += report.ignored_count;

        for file in report.files {
            let source_path = PathBuf::from(file.normalized_path.as_str());
            if witness_supported_extension(&source_path).is_some() {
                selection.selected.push(source_path);
                if selection.selected.len() >= max_files {
                    selection.budget_exhausted =
                        root_budget_exhausted || root_index + 1 < roots.len();
                    return Ok(selection);
                }
            } else {
                selection.unsupported_entries += 1;
            }
        }

        if root_budget_exhausted {
            selection.budget_exhausted = true;
            return Ok(selection);
        }
    }

    Ok(selection)
}

fn copy_witness_inputs(paths: &[PathBuf], import_root: &Path) -> Result<()> {
    fs::create_dir_all(import_root)
        .map_err(|_| CliError::user("unable to prepare private witness input"))?;

    for (index, path) in paths.iter().enumerate() {
        let extension = witness_supported_extension(path)
            .ok_or_else(|| CliError::user("witness input extension is unsupported"))?;
        let destination = import_root.join(format!("sample-{index:06}.{extension}"));
        fs::copy(path, destination)
            .map_err(|_| CliError::user("unable to copy private witness input"))?;
    }

    Ok(())
}

fn witness_supported_extension(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pdf") => Some("pdf"),
        Some("docx") => Some("docx"),
        Some("doc") => Some("doc"),
        _ => None,
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

struct WitnessArgs {
    root_selection: WitnessRootSelection,
    max_files: usize,
    run_ocr: bool,
    probe_search: bool,
    probe_fields: bool,
    probe_benchmark_corpus: bool,
    ocr_max_documents: Option<usize>,
    ocr_worker_args: OcrWorkerArgs,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum WitnessRootSelection {
    Explicit(PathBuf),
    Preset(RootPreset),
}

impl WitnessRootSelection {
    fn preset_label(&self) -> Option<&'static str> {
        match self {
            Self::Explicit(_) => None,
            Self::Preset(RootPreset::LocalDiscovery) => Some("local-discovery"),
        }
    }
}

#[derive(Default)]
struct WitnessSelection {
    selected: Vec<PathBuf>,
    unsupported_entries: usize,
    scan_errors: usize,
    budget_exhausted: bool,
}

enum WitnessOcrStatus {
    NotRequested,
    Completed {
        documents_processed: usize,
        documents_failed: usize,
        cache_writes: usize,
        cache_hits: usize,
        budget_exhausted: bool,
    },
    Blocked {
        reason: &'static str,
        documents_processed: usize,
        documents_failed: usize,
        cache_writes: usize,
        cache_hits: usize,
        budget_exhausted: bool,
    },
}

enum WitnessBenchmarkCorpusStatus {
    NotRequested,
    Completed { summary: BenchmarkCorpusSummary },
}

enum WitnessSearchStatus {
    NotRequested,
    Completed { hits: usize },
    Blocked { reason: &'static str, hits: usize },
}

#[derive(Clone, Default)]
struct WitnessFieldCounts {
    by_label: BTreeMap<&'static str, usize>,
}

impl WitnessFieldCounts {
    fn add(&mut self, label: &'static str, count: usize) {
        *self.by_label.entry(label).or_default() += count;
    }

    fn get(&self, label: &str) -> usize {
        self.by_label.get(label).copied().unwrap_or(0)
    }
}

enum WitnessFieldStatus {
    NotRequested,
    Completed {
        documents: usize,
        mentions: usize,
        counts: WitnessFieldCounts,
    },
    Blocked {
        reason: &'static str,
        documents: usize,
        mentions: usize,
        counts: WitnessFieldCounts,
    },
}

struct WitnessTempDirs {
    root: PathBuf,
    input_root: PathBuf,
    data_dir: PathBuf,
}

impl WitnessTempDirs {
    fn create() -> Result<Self> {
        let root = unique_witness_temp_root()?;
        let input_root = root.join("input");
        let data_dir = root.join("data");
        fs::create_dir_all(&input_root)
            .map_err(|_| CliError::user("unable to prepare private witness input"))?;
        fs::create_dir_all(&data_dir)
            .map_err(|_| CliError::user("unable to prepare private witness data"))?;
        Ok(Self {
            root,
            input_root,
            data_dir,
        })
    }

    fn cleanup(&self) -> bool {
        remove_witness_temp_root(&self.root)
    }
}

impl Drop for WitnessTempDirs {
    fn drop(&mut self) {
        let _ = remove_witness_temp_root(&self.root);
    }
}

fn remove_witness_temp_root(root: &Path) -> bool {
    match fs::remove_dir_all(root) {
        Ok(()) => true,
        Err(_) => !root.exists(),
    }
}

fn unique_witness_temp_root() -> Result<PathBuf> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliError::user("system clock is before the Unix epoch"))?;
    let root = std::env::temp_dir().join(format!(
        "resume-ir-local-witness-{}-{}",
        std::process::id(),
        duration.as_nanos()
    ));
    fs::create_dir_all(&root)
        .map_err(|_| CliError::user("unable to prepare private witness data"))?;
    Ok(root)
}

fn import_ipc_command(endpoint: &IpcImportEndpoint, import_args: &ImportArgs) -> Result<()> {
    let token_file = import_args
        .ipc_token_file
        .as_ref()
        .ok_or_else(import_usage)?;
    import_ipc_command_with_token_file(endpoint, token_file, import_args)
}

fn import_ipc_command_with_token_file(
    endpoint: &IpcImportEndpoint,
    token_file: &Path,
    import_args: &ImportArgs,
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .map_err(|_| CliError::user("unable to read daemon import ipc token"))?;
    let token = validate_daemon_ipc_token(&token, "daemon import ipc token is invalid")?;

    let roots = expand_import_root_selection(&import_args.root_selection)?;
    let root_values = roots
        .iter()
        .map(|root| serde_json::Value::String(path_string(root)))
        .collect::<Vec<_>>();
    let root_preset = import_args.root_selection.preset_label();
    let body = serde_json::json!({
        "roots": root_values,
        "root_preset": root_preset,
        "profile": import_args.profile.label(),
        "max_files": import_args.max_files,
    })
    .to_string();

    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon import ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon import ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon import ipc"))?;
    let request = format!(
        "POST /imports HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.addr,
        token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon import ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon import ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon import ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 202 ") && !status_line.starts_with("HTTP/1.0 202 ") {
        return Err(CliError::user("daemon import ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon import ipc returned invalid json"))?;
    render_import_ipc_result(&body);
    Ok(())
}

fn validate_daemon_ipc_token(token: &str, invalid_message: &'static str) -> Result<String> {
    parse_daemon_ipc_auth(token, invalid_message).map(|(_, token)| token)
}

fn parse_daemon_ipc_auth(value: &str, invalid_message: &'static str) -> Result<(String, String)> {
    let value: serde_json::Value =
        serde_json::from_str(value).map_err(|_| CliError::user(invalid_message))?;
    let valid_shape = value.as_object().is_some_and(|object| {
        object.len() == 3
            && object.contains_key("schema_version")
            && object.contains_key("instance_id")
            && object.contains_key("token")
    });
    let instance_id = json_str(&value, "instance_id");
    let token = json_str(&value, "token");
    if !valid_shape
        || json_str(&value, "schema_version") != Some(IPC_AUTH_SCHEMA_VERSION)
        || !instance_id.is_some_and(valid_daemon_generation_value)
        || !token.is_some_and(valid_daemon_generation_value)
    {
        return Err(CliError::user(invalid_message));
    }
    Ok((
        instance_id.unwrap_or_default().to_string(),
        token.unwrap_or_default().to_string(),
    ))
}

fn valid_daemon_generation_value(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn render_import_ipc_result(body: &serde_json::Value) {
    let task_ids = body
        .get("task_ids")
        .and_then(serde_json::Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let roots_queued = json_u64(body, "accepted_roots");
    let scan_file_limit = body
        .get("scan_file_limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".to_string());

    println!("import task submitted");
    match task_ids.as_slice() {
        [task_id] => println!("task id: {task_id}"),
        [] => println!("task ids: none"),
        ids => println!("task ids: {}", ids.join(",")),
    }
    println!("status: queued");
    println!(
        "scan profile: {}",
        json_str(body, "scan_profile").unwrap_or("unknown")
    );
    println!("roots queued: {roots_queued}");
    println!("scan file limit: {scan_file_limit}");
}

fn cancel_import_ipc_command_with_token_file(
    endpoint: &IpcImportCancelEndpoint,
    token_file: &Path,
    task_id: &ImportTaskId,
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .map_err(|_| CliError::user("unable to read daemon import cancel ipc token"))?;
    let token = validate_daemon_ipc_token(&token, "daemon import cancel ipc token is invalid")?;
    let body = serde_json::json!({
        "task_id": task_id.to_string(),
    })
    .to_string();

    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon import cancel ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon import cancel ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon import cancel ipc"))?;
    let request = format!(
        "POST /imports/cancel HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.addr,
        token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon import cancel ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon import cancel ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon import cancel ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 202 ") && !status_line.starts_with("HTTP/1.0 202 ") {
        return Err(CliError::user("daemon import cancel ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon import cancel ipc returned invalid json"))?;
    render_import_cancel_ipc_result(&body);
    Ok(())
}

fn render_import_cancel_ipc_result(body: &serde_json::Value) {
    println!("import task cancelled");
    println!(
        "task id: {}",
        json_str(body, "task_id").unwrap_or("unknown")
    );
    println!("status: cancelled");
}

fn merge_import_summary(total: &mut ImportSummary, next: ImportSummary, root_offset: Duration) {
    total.files_discovered += next.files_discovered;
    total.scan_errors += next.scan_errors;
    total.ignored_entries += next.ignored_entries;
    total.content_bytes_read += next.content_bytes_read;
    total.searchable_documents += next.searchable_documents;
    total.ocr_required_documents += next.ocr_required_documents;
    total.ocr_jobs_queued += next.ocr_jobs_queued;
    total.failed_documents += next.failed_documents;
    for (kind, count) in next.failure_counts.entries() {
        total.failure_counts.add(kind, count);
    }
    total.deleted_documents += next.deleted_documents;
    total.stage_timings.add_assign(&next.stage_timings);
    total.worker_metrics.add_assign(&next.worker_metrics);
    merge_import_milestone_timings(
        &mut total.milestone_timings,
        next.milestone_timings,
        root_offset,
    );
    if next.scan_budget.is_some()
        && (total.scan_budget.is_none() || next.scan_budget.is_some_and(|budget| budget.exhausted))
    {
        total.scan_budget = next.scan_budget;
    }
}

fn merge_import_milestone_timings(
    total: &mut ImportMilestoneTimings,
    next: ImportMilestoneTimings,
    root_offset: Duration,
) {
    total.first_searchable = earliest_duration(
        total.first_searchable,
        offset_duration(next.first_searchable, root_offset),
    );
    total.ttf100_searchable = earliest_duration(
        total.ttf100_searchable,
        offset_duration(next.ttf100_searchable, root_offset),
    );
    total.ttf1000_searchable = earliest_duration(
        total.ttf1000_searchable,
        offset_duration(next.ttf1000_searchable, root_offset),
    );
    total.full_import_ready = latest_duration(
        total.full_import_ready,
        offset_duration(next.full_import_ready, root_offset),
    );
    total.full_index_ready = latest_duration(
        total.full_index_ready,
        offset_duration(next.full_index_ready, root_offset),
    );
}

fn offset_duration(duration: Option<Duration>, offset: Duration) -> Option<Duration> {
    duration.map(|duration| duration + offset)
}

fn earliest_duration(current: Option<Duration>, next: Option<Duration>) -> Option<Duration> {
    match (current, next) {
        (Some(current), Some(next)) => Some(current.min(next)),
        (Some(current), None) => Some(current),
        (None, Some(next)) => Some(next),
        (None, None) => None,
    }
}

fn latest_duration(current: Option<Duration>, next: Option<Duration>) -> Option<Duration> {
    match (current, next) {
        (Some(current), Some(next)) => Some(current.max(next)),
        (Some(current), None) => Some(current),
        (None, Some(next)) => Some(next),
        (None, None) => None,
    }
}

fn print_import_milestone_timings(summary: &ImportSummary) {
    print_import_milestone(
        "first searchable ms",
        summary.milestone_timings.first_searchable,
    );
    print_import_milestone(
        "ttf100 searchable ms",
        summary.milestone_timings.ttf100_searchable,
    );
    print_import_milestone(
        "ttf1000 searchable ms",
        summary.milestone_timings.ttf1000_searchable,
    );
    print_import_milestone(
        "full import ready ms",
        summary.milestone_timings.full_import_ready,
    );
    print_import_milestone(
        "full index ready ms",
        summary.milestone_timings.full_index_ready,
    );
}

fn print_import_milestone(label: &str, duration: Option<Duration>) {
    match duration {
        Some(duration) => println!("{label}: {:.3}", duration_millis(duration)),
        None => println!("{label}: n/a"),
    }
}

fn print_import_throughput(summary: &ImportSummary) {
    let elapsed_seconds = summary
        .milestone_timings
        .full_import_ready
        .or(summary.milestone_timings.full_index_ready)
        .map(|duration| duration.as_secs_f64())
        .filter(|seconds| *seconds > 0.0);

    print_import_rate(
        "docs per second",
        elapsed_seconds.map(|seconds| summary.files_discovered as f64 / seconds),
    );
    print_import_rate(
        "MiB per second",
        elapsed_seconds
            .map(|seconds| summary.content_bytes_read as f64 / (1024.0 * 1024.0) / seconds),
    );
    println!(
        "scan complete ms: {:.3}",
        duration_millis(summary.stage_timings.scan)
    );
}

fn print_import_rate(label: &str, rate: Option<f64>) {
    match rate {
        Some(rate) if rate.is_finite() => println!("{label}: {rate:.3}"),
        _ => println!("{label}: n/a"),
    }
}

fn print_import_stage_timings(summary: &ImportSummary) {
    println!(
        "stage scan ms: {:.3}",
        duration_millis(summary.stage_timings.scan)
    );
    println!(
        "stage parse ms: {:.3}",
        duration_millis(summary.stage_timings.parse)
    );
    println!(
        "stage db ms: {:.3}",
        duration_millis(summary.stage_timings.db)
    );
    println!(
        "stage index ms: {:.3}",
        duration_millis(summary.stage_timings.index)
    );
    println!(
        "stage ocr ms: {:.3}",
        duration_millis(summary.stage_timings.ocr)
    );
    println!(
        "stage embedding ms: {:.3}",
        duration_millis(summary.stage_timings.embedding)
    );
}

fn print_import_worker_metrics(summary: &ImportSummary) {
    println!(
        "parse worker count: {}",
        summary.worker_metrics.parse_worker_count
    );
    println!(
        "parse jobs queued: {}",
        summary.worker_metrics.parse_jobs_queued
    );
    println!(
        "parse prepare ms: {:.3}",
        duration_millis(summary.worker_metrics.parse_prepare)
    );
    println!(
        "parse worker wall ms: {:.3}",
        duration_millis(summary.worker_metrics.parse_worker_wall)
    );
    println!(
        "parse worker active ms: {:.3}",
        duration_millis(summary.worker_metrics.parse_worker_active)
    );
    println!(
        "parse queue full events: {}",
        summary.worker_metrics.parse_queue_full_events
    );
    println!(
        "parse queue wait ms: {:.3}",
        duration_millis(summary.worker_metrics.parse_queue_wait)
    );
    println!(
        "parse result wait ms: {:.3}",
        duration_millis(summary.worker_metrics.parse_result_wait)
    );
    println!(
        "cancel check count: {}",
        summary.worker_metrics.cancel_check_count
    );
    println!(
        "cancel check max gap ms: {:.3}",
        duration_millis(summary.worker_metrics.cancel_check_max_gap)
    );
    println!(
        "cancel check max gap phase: {}",
        summary.worker_metrics.cancel_check_max_gap_phase.as_label()
    );
    println!(
        "index publication setup ms: {:.3}",
        duration_millis(summary.worker_metrics.index_publication_timings.setup)
    );
    println!(
        "index publication documents ms: {:.3}",
        duration_millis(summary.worker_metrics.index_publication_timings.documents)
    );
    println!(
        "index publication commit ms: {:.3}",
        duration_millis(summary.worker_metrics.index_publication_timings.commit)
    );
    println!(
        "index publication plaintext validation ms: {:.3}",
        duration_millis(
            summary
                .worker_metrics
                .index_publication_timings
                .plaintext_validation
        )
    );
    println!(
        "index publication encrypted publication ms: {:.3}",
        duration_millis(
            summary
                .worker_metrics
                .index_publication_timings
                .encrypted_publication
        )
    );
    println!(
        "index publication encrypted validation ms: {:.3}",
        duration_millis(
            summary
                .worker_metrics
                .index_publication_timings
                .encrypted_validation
        )
    );
    println!(
        "pdf parse document load ms: {:.3}",
        duration_millis(summary.worker_metrics.pdf_parse_timings.document_load)
    );
    println!(
        "pdf parse page content fetch ms: {:.3}",
        duration_millis(summary.worker_metrics.pdf_parse_timings.page_content_fetch)
    );
    println!(
        "pdf parse text operator prefilter ms: {:.3}",
        duration_millis(
            summary
                .worker_metrics
                .pdf_parse_timings
                .text_operator_prefilter
        )
    );
    println!(
        "pdf parse font encoding ms: {:.3}",
        duration_millis(summary.worker_metrics.pdf_parse_timings.font_encoding)
    );
    println!(
        "pdf parse content decode ms: {:.3}",
        duration_millis(summary.worker_metrics.pdf_parse_timings.content_decode)
    );
    println!(
        "pdf parse content string parse sampled ms: {:.3}",
        duration_millis(
            summary
                .worker_metrics
                .pdf_parse_timings
                .content_string_parse
        )
    );
    println!(
        "pdf parse text collection ms: {:.3}",
        duration_millis(summary.worker_metrics.pdf_parse_timings.text_collection)
    );
    println!(
        "pdf parse text byte decode sampled ms: {:.3}",
        duration_millis(summary.worker_metrics.pdf_parse_timings.text_byte_decode)
    );
    println!(
        "pdf parse text accumulation sampled ms: {:.3}",
        duration_millis(summary.worker_metrics.pdf_parse_timings.text_accumulation)
    );
    println!(
        "pdf parse content string operands: {}",
        summary
            .worker_metrics
            .pdf_parse_timings
            .content_string_operands
    );
    println!(
        "pdf parse content string bytes: {}",
        summary
            .worker_metrics
            .pdf_parse_timings
            .content_string_bytes
    );
    println!(
        "pdf parse text decode runs: {}",
        summary.worker_metrics.pdf_parse_timings.text_decode_runs
    );
    println!(
        "pdf parse text decode input bytes: {}",
        summary
            .worker_metrics
            .pdf_parse_timings
            .text_decode_input_bytes
    );
    println!(
        "import post-parser normalization ms: {:.3}",
        duration_millis(summary.worker_metrics.post_parser_timings.normalization)
    );
    println!(
        "import post-parser sectionization ms: {:.3}",
        duration_millis(summary.worker_metrics.post_parser_timings.sectionization)
    );
}

fn duration_millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn bytes_to_mib(bytes: usize) -> usize {
    bytes / (1024 * 1024)
}

fn initial_import_scan_scope(
    task: &ImportTask,
    root: &CanonicalImportRoot,
    import_args: &ImportArgs,
    updated_at: UnixTimestamp,
) -> Result<ImportScanScope> {
    let (root_kind, root_preset) = import_scan_scope_root(&import_args.root_selection);
    new_import_scan_scope(
        task,
        path_string(&root.requested),
        root_kind,
        root_preset,
        import_args.profile,
        import_args.max_files,
        updated_at,
    )
}

fn new_import_scan_scope(
    task: &ImportTask,
    requested_root_path: String,
    root_kind: StoreImportRootKind,
    root_preset: Option<StoreImportRootPreset>,
    scan_profile: ScanProfile,
    max_files: Option<usize>,
    updated_at: UnixTimestamp,
) -> Result<ImportScanScope> {
    Ok(ImportScanScope {
        import_task_id: task.id.clone(),
        root_kind,
        root_preset,
        scan_profile: import_scan_profile(scan_profile),
        requested_root_path,
        canonical_root_path: task.root_path.clone(),
        files_discovered: 0,
        ignored_entries: 0,
        scan_errors: 0,
        searchable_documents: 0,
        ocr_required_documents: 0,
        ocr_jobs_queued: 0,
        failed_documents: 0,
        deleted_documents: 0,
        scan_budget_kind: max_files.map(|_| StoreImportScanBudgetKind::Files),
        scan_budget_limit: max_files.map(usize_to_u64).transpose()?,
        scan_budget_observed: max_files.map(|_| 0),
        scan_budget_exhausted: false,
        updated_at,
    })
}

fn import_scan_scope_root(
    selection: &ImportRootSelection,
) -> (StoreImportRootKind, Option<StoreImportRootPreset>) {
    match selection {
        ImportRootSelection::Explicit(_) => (StoreImportRootKind::Explicit, None),
        ImportRootSelection::Preset(RootPreset::LocalDiscovery) => (
            StoreImportRootKind::Preset,
            Some(StoreImportRootPreset::LocalDiscovery),
        ),
    }
}

fn import_scan_profile(profile: ScanProfile) -> StoreImportScanProfile {
    match profile {
        ScanProfile::Explicit => StoreImportScanProfile::Explicit,
        ScanProfile::Discovery => StoreImportScanProfile::Discovery,
    }
}

fn usize_to_u64(value: usize) -> Result<u64> {
    u64::try_from(value).map_err(|_| CliError::user("import summary count is too large"))
}

#[derive(Clone)]
struct CanonicalImportRoot {
    requested: PathBuf,
    canonical: PathBuf,
}

fn path_string(path: &Path) -> String {
    path.as_os_str().to_string_lossy().into_owned()
}

struct ImportArgs {
    root_selection: ImportRootSelection,
    profile: ScanProfile,
    max_files: Option<usize>,
    parse_workers: ImportParseWorkers,
    linear_promotion_artifact: Option<PathBuf>,
    index_writer_heap_bytes: usize,
    hardware_tier: ImportHardwareTier,
    max_private_or_anonymous_mb: u16,
    enqueue: bool,
    ipc_auto: bool,
    ipc_endpoint: Option<IpcImportEndpoint>,
    ipc_token_file: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ImportRootSelection {
    Explicit(Vec<PathBuf>),
    Preset(RootPreset),
}

impl ImportRootSelection {
    fn preset_label(&self) -> Option<&'static str> {
        match self {
            Self::Explicit(_) => None,
            Self::Preset(RootPreset::LocalDiscovery) => Some("local-discovery"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RootPreset {
    LocalDiscovery,
}

impl RootPreset {
    fn default_profile(self) -> ScanProfile {
        match self {
            Self::LocalDiscovery => ScanProfile::Discovery,
        }
    }
}

fn parse_import_args(args: &[String]) -> Result<ImportArgs> {
    let mut roots = Vec::<PathBuf>::new();
    let mut root_preset = None;
    let mut profile = None;
    let mut profile_seen = false;
    let mut max_files = None;
    let mut parse_workers = None;
    let mut linear_promotion_artifact = None;
    let mut enqueue = false;
    let mut ipc_auto = false;
    let mut ipc_endpoint = None;
    let mut ipc_token_file = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--enqueue" => {
                if enqueue {
                    return Err(import_usage());
                }
                enqueue = true;
            }
            "--root" => {
                if root_preset.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                let root = PathBuf::from(value);
                if roots.iter().any(|existing| existing == &root) {
                    return Err(import_usage());
                }
                roots.push(root);
            }
            "--root-preset" => {
                if root_preset.is_some() || !roots.is_empty() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                root_preset = Some(parse_root_preset(value)?);
            }
            "--profile" => {
                if profile_seen {
                    return Err(import_usage());
                }
                profile_seen = true;
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                profile = Some(parse_scan_profile(value)?);
            }
            "--max-files" => {
                if max_files.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                max_files = Some(parse_positive_usize(value)?);
            }
            "--parse-workers" => {
                if parse_workers.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                parse_workers = Some(parse_import_parse_workers(value)?);
            }
            "--resume-classifier-model" => {
                if linear_promotion_artifact.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                linear_promotion_artifact = Some(PathBuf::from(value));
            }
            "--ipc" => {
                if ipc_auto || ipc_endpoint.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                if value == "auto" {
                    ipc_auto = true;
                } else {
                    ipc_endpoint = Some(parse_import_ipc_endpoint(value)?);
                }
            }
            "--ipc-token-file" => {
                if ipc_token_file.is_some() {
                    return Err(import_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(import_usage());
                };
                ipc_token_file = Some(PathBuf::from(value));
            }
            _ => return Err(import_usage()),
        }
        index += 1;
    }
    if ipc_auto && ipc_token_file.is_some() {
        return Err(import_usage());
    }
    if !ipc_auto && ipc_endpoint.is_some() != ipc_token_file.is_some() {
        return Err(import_usage());
    }
    if linear_promotion_artifact.is_some() && (enqueue || ipc_auto || ipc_endpoint.is_some()) {
        return Err(import_usage());
    }

    let (root_selection, default_profile) = if !roots.is_empty() {
        (ImportRootSelection::Explicit(roots), ScanProfile::Explicit)
    } else if let Some(root_preset) = root_preset {
        (
            ImportRootSelection::Preset(root_preset),
            root_preset.default_profile(),
        )
    } else {
        return Err(import_usage());
    };

    let max_files = match (&root_selection, max_files) {
        (ImportRootSelection::Preset(RootPreset::LocalDiscovery), None) => {
            Some(LOCAL_DISCOVERY_DEFAULT_MAX_FILES)
        }
        (_, max_files) => max_files,
    };

    let default_resource_policy = ImportResourcePolicy::detect();

    Ok(ImportArgs {
        root_selection,
        profile: profile.unwrap_or(default_profile),
        max_files,
        parse_workers: parse_workers.unwrap_or(default_resource_policy.parse_workers),
        linear_promotion_artifact,
        index_writer_heap_bytes: default_resource_policy.index_writer_heap_bytes,
        hardware_tier: default_resource_policy.hardware_tier,
        max_private_or_anonymous_mb: default_resource_policy.max_private_or_anonymous_mb,
        enqueue,
        ipc_auto,
        ipc_endpoint,
        ipc_token_file,
    })
}

fn parse_import_ipc_endpoint(value: &str) -> Result<IpcImportEndpoint> {
    let rest = value.strip_prefix("http://").ok_or_else(import_usage)?;
    let (authority, path) = rest.split_once('/').ok_or_else(import_usage)?;
    if path != "imports" && path != "status" {
        return Err(import_usage());
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| import_usage())?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage("import ipc endpoint must be loopback"));
    }

    Ok(IpcImportEndpoint { addr })
}

fn parse_import_cancel_ipc_endpoint(value: &str) -> Result<IpcImportCancelEndpoint> {
    let rest = value.strip_prefix("http://").ok_or_else(cancel_usage)?;
    let (authority, path) = rest.split_once('/').ok_or_else(cancel_usage)?;
    if path != "imports/cancel" && path != "status" {
        return Err(cancel_usage());
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| cancel_usage())?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage(
            "import cancel ipc endpoint must be loopback",
        ));
    }

    Ok(IpcImportCancelEndpoint { addr })
}

fn parse_import_progress_ipc_endpoint(value: &str) -> Result<IpcImportProgressEndpoint> {
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| CliError::usage(status_usage()))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| CliError::usage(status_usage()))?;
    if path != "imports/progress" {
        return Err(CliError::usage(status_usage()));
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| CliError::usage(status_usage()))?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage(
            "import progress ipc endpoint must be loopback",
        ));
    }

    Ok(IpcImportProgressEndpoint { addr })
}

fn parse_root_preset(value: &str) -> Result<RootPreset> {
    match value {
        "local-discovery" => Ok(RootPreset::LocalDiscovery),
        _ => Err(import_usage()),
    }
}

fn parse_scan_profile(value: &str) -> Result<ScanProfile> {
    match value {
        "explicit" => Ok(ScanProfile::Explicit),
        "discovery" => Ok(ScanProfile::Discovery),
        _ => Err(import_usage()),
    }
}

fn parse_positive_usize(value: &str) -> Result<usize> {
    let parsed = value.parse::<usize>().map_err(|_| import_usage())?;
    if parsed == 0 {
        return Err(import_usage());
    }
    Ok(parsed)
}

fn parse_import_parse_workers(value: &str) -> Result<ImportParseWorkers> {
    Ok(ImportParseWorkers::new(parse_positive_usize(value)?))
}

fn import_usage_text() -> &'static str {
    "usage: resume-cli import [--enqueue] [--ipc auto|<http://127.0.0.1:port/imports|/status> --ipc-token-file <path>] (--root <path> [--root <path> ...] | --root-preset local-discovery) [--profile explicit|discovery] [--max-files <count>] [--parse-workers <count>] [--resume-classifier-model <owner-only-path>]"
}

fn import_usage() -> CliError {
    CliError::usage(import_usage_text())
}

fn expand_import_root_selection(selection: &ImportRootSelection) -> Result<Vec<PathBuf>> {
    match selection {
        ImportRootSelection::Explicit(roots) => Ok(roots.clone()),
        ImportRootSelection::Preset(RootPreset::LocalDiscovery) => local_discovery_roots(),
    }
}

fn local_discovery_roots() -> Result<Vec<PathBuf>> {
    let roots = std::env::var_os(LOCAL_DISCOVERY_ROOTS_ENV)
        .map(|value| {
            std::env::split_paths(&value)
                .filter(|path| !path.as_os_str().is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(platform_local_discovery_roots);

    if roots.is_empty() {
        return Err(CliError::user(
            "local discovery import roots are unavailable",
        ));
    }

    Ok(roots)
}

#[cfg(not(windows))]
fn platform_local_discovery_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/")]
}

#[cfg(windows)]
fn platform_local_discovery_roots() -> Vec<PathBuf> {
    (b'A'..=b'Z')
        .map(|drive| PathBuf::from(format!("{}:\\", drive as char)))
        .filter(|root| {
            fs::metadata(root)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false)
        })
        .collect()
}

fn canonical_import_roots(requested_roots: &[PathBuf]) -> Result<Vec<CanonicalImportRoot>> {
    let mut roots = requested_roots
        .iter()
        .map(|requested_root| {
            let metadata = fs::metadata(requested_root)
                .map_err(|_| CliError::user("import root must exist and be a directory"))?;
            if !metadata.is_dir() {
                return Err(CliError::user("import root must exist and be a directory"));
            }
            let canonical = fs::canonicalize(requested_root)
                .map_err(|_| CliError::user("import root must exist and be a directory"))?;
            Ok(CanonicalImportRoot {
                requested: requested_root.clone(),
                canonical,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    roots.sort_by(|left, right| left.canonical.cmp(&right.canonical));
    for window in roots.windows(2) {
        let [left, right] = window else {
            continue;
        };
        if left.canonical == right.canonical || right.canonical.starts_with(&left.canonical) {
            return Err(CliError::user(
                "import roots must be distinct and non-overlapping",
            ));
        }
    }

    Ok(roots)
}

fn search_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let search_args = parse_search_args(data_dir, args)?;
    if search_args.ipc_auto {
        let status_endpoint = discover_status_ipc_endpoint(data_dir)?;
        let endpoint = discover_search_ipc_endpoint(data_dir)?;
        ensure_auto_ipc_same_daemon(status_endpoint.addr, endpoint.addr)?;
        verify_auto_ipc_status(&status_endpoint)?;
        let token_file = auto_ipc_token_file(data_dir);
        return search_ipc_command_with_token_file(&endpoint, &token_file, &search_args);
    }
    if let Some(endpoint) = &search_args.ipc_endpoint {
        return search_ipc_command(endpoint, &search_args);
    }

    let hits = match run_local_search(data_dir, &search_args)? {
        LocalSearchOutcome::Hits(hits) => hits,
        LocalSearchOutcome::SearchServiceUnavailable => {
            println!("search service unavailable");
            println!("results: 0");
            return Ok(());
        }
    };

    print_search_hits(hits);

    Ok(())
}

fn benchmark_query_protocol_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let protocol_args = parse_benchmark_query_protocol_args(args)?;
    if !protocol_args.batch_jsonl {
        return Err(CliError::usage(benchmark_query_protocol_usage()));
    }
    benchmark_query_protocol_batch_command(data_dir, &protocol_args.search_args)
}

fn benchmark_query_protocol_batch_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let batch_input_path = benchmark_query_env("RESUME_IR_QUERY_BATCH_INPUT_PATH")?;
    let top_k = benchmark_query_top_k()?;
    let mode = benchmark_query_mode()?;
    let batch = fs::File::open(&batch_input_path)
        .map_err(|_| CliError::user("benchmark query input is unavailable"))?;
    let is_regular_file = batch
        .metadata()
        .map_err(|_| CliError::user("benchmark query input is unavailable"))?
        .file_type()
        .is_file();
    let batch = BufReader::new(batch);
    if is_regular_file {
        let requests = benchmark_query_batch_requests(batch)?;
        for request in requests {
            run_benchmark_query_protocol_batch_request(data_dir, args, top_k, mode, request)?;
        }
        return Ok(());
    }
    let mut seen_request_ids = BTreeSet::new();
    let mut query_count = 0_usize;
    for line in batch.lines() {
        let line = line.map_err(|_| CliError::user("benchmark query input is unavailable"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request = benchmark_query_batch_line_request(line)?;
        remember_benchmark_query_request_id(&mut seen_request_ids, &request.request_id)?;
        run_benchmark_query_protocol_batch_request(data_dir, args, top_k, mode, request)?;
        query_count += 1;
    }
    if query_count == 0 {
        return Err(CliError::user("benchmark query input is unavailable"));
    }
    Ok(())
}

fn benchmark_query_batch_requests(batch: impl BufRead) -> Result<Vec<BenchmarkQueryBatchRequest>> {
    let mut requests = Vec::new();
    let mut seen_request_ids = BTreeSet::new();
    for line in batch.lines() {
        let line = line.map_err(|_| CliError::user("benchmark query input is unavailable"))?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let request = benchmark_query_batch_line_request(line)?;
        remember_benchmark_query_request_id(&mut seen_request_ids, &request.request_id)?;
        requests.push(request);
    }
    if requests.is_empty() {
        return Err(CliError::user("benchmark query input is unavailable"));
    }
    Ok(requests)
}

fn remember_benchmark_query_request_id(
    seen_request_ids: &mut BTreeSet<String>,
    request_id: &str,
) -> Result<()> {
    if seen_request_ids.insert(request_id.to_string()) {
        return Ok(());
    }
    Err(CliError::user("benchmark query input is unavailable"))
}

fn run_benchmark_query_protocol_batch_request(
    data_dir: &Path,
    args: &[String],
    top_k: usize,
    mode: SearchMode,
    request: BenchmarkQueryBatchRequest,
) -> Result<()> {
    let record_started = Instant::now();
    let mut search_args = vec![
        request.query,
        "--mode".to_string(),
        mode.label().to_string(),
        "--top-k".to_string(),
        top_k.to_string(),
    ];
    search_args.extend(args.iter().cloned());
    let search_args = parse_search_args(data_dir, &search_args)?;
    let outcome = run_benchmark_query_protocol_once(data_dir, &search_args, record_started)?;
    print_benchmark_query_protocol_record(&request.request_id, &search_args, outcome);
    println!("resume-ir-query-end");
    Ok(())
}

fn run_benchmark_query_protocol_once(
    data_dir: &Path,
    search_args: &SearchArgs,
    protocol_started: Instant,
) -> Result<BenchmarkQueryProtocolOutcome> {
    let rss_before_bytes = process_memory_bytes();
    let mut stage_timings = BenchmarkQueryProtocolStageTimings {
        query_parse_ms: duration_ms(protocol_started.elapsed()),
        ..BenchmarkQueryProtocolStageTimings::default()
    };
    let hits = match run_benchmark_query_protocol_search(data_dir, search_args, &mut stage_timings)?
    {
        LocalSearchOutcome::Hits(hits) => hits,
        LocalSearchOutcome::SearchServiceUnavailable => {
            return Err(CliError::user("benchmark query search service unavailable"));
        }
    };
    Ok(BenchmarkQueryProtocolOutcome {
        hit_count: hits.len(),
        elapsed_ms: duration_ms(protocol_started.elapsed()),
        rss_delta_mb: rss_delta_mb(rss_before_bytes, process_memory_bytes()),
        stage_timings,
    })
}

fn print_benchmark_query_protocol_record(
    request_id: &str,
    search_args: &SearchArgs,
    outcome: BenchmarkQueryProtocolOutcome,
) {
    println!("{QUERY_PROTOCOL_VERSION}");
    println!("request_id={request_id}");
    println!("mode={}", search_args.mode.label());
    println!("layers={}", search_args.mode.benchmark_layers_label());
    println!("top_k={}", search_args.top_k);
    println!(
        "query_embedding_runtime={}",
        search_args.mode.benchmark_query_embedding_runtime_label()
    );
    println!(
        "query_embedding_invocations={}",
        search_args.mode.benchmark_query_embedding_invocations()
    );
    outcome.stage_timings.print_protocol_lines();
    println!("rss_delta_mb={:.3}", outcome.rss_delta_mb);
    println!("elapsed_ms={:.3}", outcome.elapsed_ms);
    println!("hits={}", outcome.hit_count);
}

#[derive(Clone, Copy, Debug)]
struct BenchmarkQueryProtocolOutcome {
    hit_count: usize,
    elapsed_ms: f64,
    rss_delta_mb: f64,
    stage_timings: BenchmarkQueryProtocolStageTimings,
}

#[derive(Clone, Copy, Debug, Default)]
struct BenchmarkQueryProtocolStageTimings {
    query_parse_ms: f64,
    prefilter_ms: f64,
    bm25_ms: f64,
    ann_ms: f64,
    fusion_ms: f64,
    bulk_hydrate_ms: f64,
    snippet_ms: f64,
}

impl BenchmarkQueryProtocolStageTimings {
    fn print_protocol_lines(self) {
        println!("stage_query_parse_ms={:.3}", self.query_parse_ms);
        println!("stage_prefilter_ms={:.3}", self.prefilter_ms);
        println!("stage_bm25_ms={:.3}", self.bm25_ms);
        println!("stage_ann_ms={:.3}", self.ann_ms);
        println!("stage_fusion_ms={:.3}", self.fusion_ms);
        println!("stage_bulk_hydrate_ms={:.3}", self.bulk_hydrate_ms);
        println!("stage_snippet_ms={:.3}", self.snippet_ms);
    }
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn rss_delta_mb(before_bytes: Option<u64>, after_bytes: Option<u64>) -> f64 {
    let Some(before_bytes) = before_bytes else {
        return 0.0;
    };
    let Some(after_bytes) = after_bytes else {
        return 0.0;
    };
    after_bytes.saturating_sub(before_bytes) as f64 / 1_048_576.0
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BenchmarkQueryProtocolArgs {
    batch_jsonl: bool,
    search_args: Vec<String>,
}

fn parse_benchmark_query_protocol_args(args: &[String]) -> Result<BenchmarkQueryProtocolArgs> {
    let mut batch_jsonl = false;
    let mut search_args = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--batch-jsonl" => {
                if batch_jsonl {
                    return Err(CliError::usage(benchmark_query_protocol_usage()));
                }
                batch_jsonl = true;
            }
            "--query-file" | "--mode" | "--top-k" | "--ipc" | "--ipc-token-file" => {
                return Err(CliError::usage(benchmark_query_protocol_usage()));
            }
            _ => search_args.push(arg.clone()),
        }
    }
    Ok(BenchmarkQueryProtocolArgs {
        batch_jsonl,
        search_args,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BenchmarkQueryBatchRequest {
    request_id: String,
    query: String,
}

fn benchmark_query_batch_line_request(line: &str) -> Result<BenchmarkQueryBatchRequest> {
    let value = serde_json::from_str::<serde_json::Value>(line)
        .map_err(|_| CliError::user("benchmark query input is unavailable"))?;
    if value
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        != Some(QUERY_BATCH_REQUEST_SCHEMA_VERSION)
    {
        return Err(CliError::user("benchmark query input is unavailable"));
    }
    let request_id = value
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|request_id| is_benchmark_query_request_id(request_id))
        .map(ToOwned::to_owned)
        .ok_or_else(|| CliError::user("benchmark query input is unavailable"))?;
    let query = value
        .get("query")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| CliError::user("benchmark query input is unavailable"))?;
    let query = normalize_query_set_query(&query)
        .ok_or_else(|| CliError::user("benchmark query input is unavailable"))?;
    Ok(BenchmarkQueryBatchRequest { request_id, query })
}

fn is_benchmark_query_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BenchmarkAgentReplayFreezeArgs {
    out: PathBuf,
    trace_root: PathBuf,
    max_queries: usize,
    min_queries: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BenchmarkAgentReplayPreflightArgs {
    out: PathBuf,
    trace_root: PathBuf,
    max_queries: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FrozenAgentReplayQueries {
    queries: Vec<String>,
    candidate_queries_sampled: usize,
    zero_hit_queries_dropped: usize,
    insufficient_query_message: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TraceQuerySelectionCounts {
    trace_logs: usize,
    trace_lines: usize,
    source_search_lines: usize,
    extracted_queries: usize,
    normalization_rejected: usize,
    duplicate_queries_dropped: usize,
    candidate_queries_sampled: usize,
    zero_hit_queries_dropped: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TraceQueryPreflight {
    counts: TraceQuerySelectionCounts,
    query_index_available: bool,
    corpus_summary: BenchmarkCorpusSummary,
    candidate_bucket_counts: BTreeMap<&'static str, usize>,
    candidate_bucket_deficits: BTreeMap<&'static str, usize>,
    corpus_valid_queries: usize,
    corpus_valid_bucket_counts: BTreeMap<&'static str, usize>,
    required_bucket_counts: BTreeMap<&'static str, usize>,
    corpus_valid_bucket_deficits: BTreeMap<&'static str, usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuerySetSummaryDigests {
    query_set_sha256: String,
    tune_sha256: String,
    holdout_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct QuerySetSplit<'a> {
    tune_queries: Vec<&'a str>,
    holdout_queries: Vec<&'a str>,
    tune_bucket_counts: BTreeMap<&'static str, usize>,
    holdout_bucket_counts: BTreeMap<&'static str, usize>,
}

fn benchmark_query_set_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err(CliError::usage(benchmark_query_set_usage()));
    };
    match action {
        "preflight-agent-replay" => {
            benchmark_query_set_preflight_agent_replay_command(data_dir, &args[1..])
        }
        "freeze-agent-replay" => {
            benchmark_query_set_freeze_agent_replay_command(data_dir, &args[1..])
        }
        _ => Err(CliError::usage(benchmark_query_set_usage())),
    }
}

fn benchmark_query_set_preflight_agent_replay_command(
    data_dir: &Path,
    args: &[String],
) -> Result<()> {
    let args = parse_benchmark_agent_replay_preflight_args(args)?;
    let store_path = metadata_store_path(data_dir).map_err(CliError::store)?;
    let store = if store_path
        .try_exists()
        .map_err(|_| CliError::user("query set blocked: metadata availability is unknown"))?
    {
        Some(ReadMetaStore::open_data_dir(data_dir).map_err(CliError::store)?)
    } else {
        None
    };
    let preflight = preflight_trace_backed_private_queries(
        data_dir,
        store.as_ref(),
        &args.trace_root,
        args.max_queries,
    )?;
    let counts = preflight.counts;
    let corpus_summary = &preflight.corpus_summary;
    let d10k_corpus_deficits = d10k_corpus_deficits(corpus_summary);
    let summary = serde_json::json!({
        "schema_version": QUERY_SET_TRACE_PREFLIGHT_SCHEMA_VERSION,
        "privacy_boundary": "redacted_local_aggregate",
        "query_source": QuerySetSourceKind::TraceSourceSearchV1.as_str(),
        "target_query_count": args.max_queries,
        "document_count": corpus_summary.document_count,
        "searchable_document_count": corpus_summary.searchable_document_count,
        "vector_indexed_document_count": corpus_summary.vector_indexed_document_count,
        "d10k_min_document_count": CURRENT_STAGE_D10K_DOCUMENT_MIN,
        "d10k_min_searchable_document_count": CURRENT_STAGE_D10K_SEARCHABLE_DOCUMENT_MIN,
        "d10k_min_vector_indexed_document_count": CURRENT_STAGE_D10K_VECTOR_DOCUMENT_MIN,
        "d10k_corpus_ready": d10k_corpus_ready(corpus_summary),
        "d10k_corpus_deficits": d10k_corpus_deficits,
        "trace_logs": counts.trace_logs,
        "trace_lines": counts.trace_lines,
        "source_search_lines": counts.source_search_lines,
        "extracted_queries": counts.extracted_queries,
        "normalization_rejected": counts.normalization_rejected,
        "duplicate_queries_dropped": counts.duplicate_queries_dropped,
        "candidate_queries_sampled": counts.candidate_queries_sampled,
        "zero_hit_queries_dropped": counts.zero_hit_queries_dropped,
        "query_index_available": preflight.query_index_available,
        "candidate_bucket_counts": preflight.candidate_bucket_counts,
        "candidate_bucket_deficits": preflight.candidate_bucket_deficits,
        "corpus_valid_queries": preflight.corpus_valid_queries,
        "corpus_valid_bucket_counts": preflight.corpus_valid_bucket_counts,
        "required_bucket_counts": preflight.required_bucket_counts,
        "corpus_valid_bucket_deficits": preflight.corpus_valid_bucket_deficits,
        "contains_raw_query_text": false,
        "contains_raw_resume_text": false,
        "contains_candidate_results": false,
        "contains_local_paths": false,
    });
    let summary_text = serde_json::to_string_pretty(&summary)
        .map_err(|_| CliError::user("query set blocked: trace preflight is unavailable"))?;
    if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            create_private_query_artifact_parent(parent)?;
        }
    }
    write_private_query_artifact(&args.out, format!("{summary_text}\n").as_bytes())?;
    println!("query set trace preflight: written");
    println!("schema: {QUERY_SET_TRACE_PREFLIGHT_SCHEMA_VERSION}");
    println!("privacy boundary: redacted_local_aggregate");
    println!("queries: <redacted>");
    Ok(())
}

fn benchmark_query_set_freeze_agent_replay_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let args = parse_benchmark_agent_replay_freeze_args(args)?;
    let store = ReadMetaStore::open_data_dir(data_dir).map_err(CliError::store)?;
    if args.max_queries == D10K_TRACE_QUERY_SET_COUNT {
        let corpus_summary = benchmark_corpus_summary(data_dir, &store)?;
        if !d10k_corpus_ready(&corpus_summary) {
            return Err(CliError::user(d10k_corpus_not_ready_message(
                &corpus_summary,
            )));
        }
    }
    let frozen =
        freeze_trace_backed_private_queries(data_dir, &store, &args.trace_root, args.max_queries)?;
    let queries = frozen.queries.clone();
    if queries.len() < args.min_queries {
        return Err(CliError::user(frozen.insufficient_query_message));
    }

    let summary = write_frozen_private_query_set(data_dir, &args.out, &queries, &frozen)?;
    print_query_set_result("frozen", &queries, &frozen, &summary);
    Ok(())
}

fn write_frozen_private_query_set(
    data_dir: &Path,
    out: &Path,
    queries: &[String],
    frozen: &FrozenAgentReplayQueries,
) -> Result<QuerySetSummaryDigests> {
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            create_private_query_artifact_parent(parent)?;
        }
    }
    let mut output = String::new();
    for (index, query) in queries.iter().enumerate() {
        let shape = QuerySetSampleShape::from_query(query);
        let bucket = query_set_bucket_for_query(query);
        output.push_str(
            &serde_json::json!({
                "schema_version": QUERY_SET_SCHEMA_VERSION,
                "sample_id": format!("local-query-{number:06}", number = index + 1),
                "bucket": bucket,
                "query": query,
                "source_kind": QuerySetSourceKind::TraceSourceSearchV1.as_str(),
                "query_shape": {
                    "term_count": shape.term_count(),
                    "has_boolean": shape.has_boolean(),
                    "has_location": shape.has_location(),
                    "has_years": shape.has_years(),
                    "has_degree": shape.has_degree(),
                    "has_skill": shape.has_skill(),
                    "has_phrase": shape.has_phrase(),
                },
            })
            .to_string(),
        );
        output.push('\n');
    }
    write_private_query_artifact(out, output.as_bytes())?;
    write_query_set_redacted_summary(data_dir, out, queries, frozen)
}

fn create_private_query_artifact_parent(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .map_err(|_| CliError::user("query set blocked: output is unavailable"))?;
    set_private_query_artifact_dir_permissions(path)
}

#[cfg(unix)]
fn set_private_query_artifact_dir_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| CliError::user("query set blocked: output is unavailable"))
}

#[cfg(not(unix))]
fn set_private_query_artifact_dir_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn write_private_query_artifact(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp_path = private_query_artifact_tmp_path(path)?;
    let result = write_private_query_artifact_temp(&tmp_path, bytes).and_then(|_| {
        fs::rename(&tmp_path, path)
            .map_err(|_| CliError::user("query set blocked: output is unavailable"))
    });
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

fn private_query_artifact_tmp_path(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .filter(|file_name| !file_name.is_empty())
        .ok_or_else(|| CliError::user("query set blocked: output is unavailable"))?;
    Ok(path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id())))
}

#[cfg(unix)]
fn write_private_query_artifact_temp(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|_| CliError::user("query set blocked: output is unavailable"))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| CliError::user("query set blocked: output is unavailable"))?;
    file.write_all(bytes)
        .map_err(|_| CliError::user("query set blocked: output is unavailable"))
}

#[cfg(not(unix))]
fn write_private_query_artifact_temp(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(|_| CliError::user("query set blocked: output is unavailable"))
}

fn print_query_set_result(
    state: &str,
    queries: &[String],
    frozen: &FrozenAgentReplayQueries,
    summary: &QuerySetSummaryDigests,
) {
    println!("query set: {state}");
    println!("query set summary: written");
    println!("schema: {QUERY_SET_SCHEMA_VERSION}");
    println!("summary schema: {QUERY_SET_SUMMARY_SCHEMA_VERSION}");
    println!("privacy boundary: local_only_private_query_set");
    println!(
        "query source: {}",
        QuerySetSourceKind::TraceSourceSearchV1.as_str()
    );
    println!("queries: {}", queries.len());
    println!(
        "candidate queries sampled: {}",
        frozen.candidate_queries_sampled
    );
    println!(
        "zero-hit queries dropped: {}",
        frozen.zero_hit_queries_dropped
    );
    println!("query set sha256: {}", summary.query_set_sha256);
    println!("tune sha256: {}", summary.tune_sha256);
    println!("holdout sha256: {}", summary.holdout_sha256);
    println!("hmac split: true");
    println!("queries: <redacted>");
    println!("sample ids: <redacted>");
    println!("paths: <redacted>");
}

fn parse_benchmark_agent_replay_preflight_args(
    args: &[String],
) -> Result<BenchmarkAgentReplayPreflightArgs> {
    let mut out = None;
    let mut trace_root = None;
    let mut max_queries = D10K_TRACE_QUERY_SET_COUNT;
    let mut max_queries_seen = false;
    let mut index = 0_usize;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                if out.is_some() {
                    return Err(CliError::usage(benchmark_query_set_usage()));
                }
                out = Some(take_benchmark_query_set_path(args, &mut index)?);
            }
            "--trace-root" => {
                if trace_root.is_some() {
                    return Err(CliError::usage(benchmark_query_set_usage()));
                }
                trace_root = Some(take_benchmark_query_set_path(args, &mut index)?);
            }
            "--max-queries" => {
                if max_queries_seen {
                    return Err(CliError::usage(benchmark_query_set_usage()));
                }
                max_queries = take_benchmark_query_set_positive_usize(args, &mut index)?;
                max_queries_seen = true;
            }
            "--min-queries" => return Err(CliError::usage(benchmark_query_set_usage())),
            _ => return Err(CliError::usage(benchmark_query_set_usage())),
        }
    }
    let trace_root = match trace_root {
        Some(trace_root) => trace_root,
        None => query_artifact_root_from_env()?,
    };
    let out = match out {
        Some(out) => out,
        None => local_evidence_output_path(QUERY_SET_TRACE_PREFLIGHT_DEFAULT_FILE)?,
    };
    ensure_query_artifact_outside_git_worktree(&out)?;
    Ok(BenchmarkAgentReplayPreflightArgs {
        out,
        trace_root,
        max_queries,
    })
}

fn parse_benchmark_agent_replay_freeze_args(
    args: &[String],
) -> Result<BenchmarkAgentReplayFreezeArgs> {
    let mut out = None;
    let mut trace_root = None;
    let mut max_queries = D10K_TRACE_QUERY_SET_COUNT;
    let mut max_queries_seen = false;
    let mut min_queries = None;
    let mut index = 0_usize;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                if out.is_some() {
                    return Err(CliError::usage(benchmark_query_set_usage()));
                }
                out = Some(take_benchmark_query_set_path(args, &mut index)?);
            }
            "--trace-root" => {
                if trace_root.is_some() {
                    return Err(CliError::usage(benchmark_query_set_usage()));
                }
                trace_root = Some(take_benchmark_query_set_path(args, &mut index)?);
            }
            "--max-queries" => {
                if max_queries_seen {
                    return Err(CliError::usage(benchmark_query_set_usage()));
                }
                max_queries = take_benchmark_query_set_positive_usize(args, &mut index)?;
                max_queries_seen = true;
            }
            "--min-queries" => {
                if min_queries.is_some() {
                    return Err(CliError::usage(benchmark_query_set_usage()));
                }
                min_queries = Some(take_benchmark_query_set_positive_usize(args, &mut index)?);
            }
            _ => return Err(CliError::usage(benchmark_query_set_usage())),
        }
    }
    let min_queries = min_queries.unwrap_or(max_queries);
    if min_queries > max_queries {
        return Err(CliError::usage(benchmark_query_set_usage()));
    }
    if max_queries == D10K_TRACE_QUERY_SET_COUNT && min_queries != D10K_TRACE_QUERY_SET_COUNT {
        return Err(CliError::user(
            "query set blocked: D10K agent replay freeze requires 500 queries",
        ));
    }
    let trace_root = match trace_root {
        Some(trace_root) => trace_root,
        None => query_artifact_root_from_env()?,
    };
    let out = match out {
        Some(out) => out,
        None => local_evidence_output_path(PRIVATE_QUERY_SET_DEFAULT_FILE)?,
    };
    ensure_query_artifact_outside_git_worktree(&out)?;
    Ok(BenchmarkAgentReplayFreezeArgs {
        out,
        trace_root,
        max_queries,
        min_queries,
    })
}

fn query_artifact_root_from_env() -> Result<PathBuf> {
    let value = std::env::var_os(QUERY_ARTIFACT_ROOT_ENV)
        .ok_or_else(|| CliError::usage(benchmark_query_set_usage()))?;
    if value.as_os_str().is_empty() {
        return Err(CliError::usage(benchmark_query_set_usage()));
    }
    Ok(PathBuf::from(value))
}

fn local_evidence_output_path(file_name: &str) -> Result<PathBuf> {
    let value = std::env::var_os(LOCAL_EVIDENCE_DIR_ENV)
        .ok_or_else(|| CliError::usage(benchmark_query_set_usage()))?;
    if value.as_os_str().is_empty() {
        return Err(CliError::usage(benchmark_query_set_usage()));
    }
    Ok(PathBuf::from(value).join(file_name))
}

fn ensure_query_artifact_outside_git_worktree(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        normalize_lexical_path(path)
    } else {
        let current_dir = std::env::current_dir()
            .map_err(|_| CliError::user("query set blocked: output is unavailable"))?;
        normalize_lexical_path(&current_dir.join(path))
    };
    let mut current = absolute.as_path();
    loop {
        let git_marker = current.join(".git");
        if git_marker.is_dir() || git_marker.is_file() {
            return Err(CliError::user(
                "query set blocked: local query artifacts must not be written inside a git worktree",
            ));
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent;
    }
    Ok(())
}

fn normalize_lexical_path(path: &Path) -> PathBuf {
    use std::path::Component;

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn take_benchmark_query_set_path(args: &[String], index: &mut usize) -> Result<PathBuf> {
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(benchmark_query_set_usage()));
    };
    if value.trim().is_empty() {
        return Err(CliError::usage(benchmark_query_set_usage()));
    }
    *index += 2;
    Ok(PathBuf::from(value))
}

fn take_benchmark_query_set_positive_usize(args: &[String], index: &mut usize) -> Result<usize> {
    let Some(value) = args.get(*index + 1) else {
        return Err(CliError::usage(benchmark_query_set_usage()));
    };
    let parsed = value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CliError::usage(benchmark_query_set_usage()))?;
    *index += 2;
    Ok(parsed)
}

fn benchmark_query_set_usage() -> &'static str {
    "usage: resume-cli benchmark-query-set preflight-agent-replay [--out <path>] [--trace-root <path>] [--max-queries <count>]\n       resume-cli benchmark-query-set freeze-agent-replay [--out <path>] [--trace-root <path>] [--max-queries <count>] [--min-queries <count>]\n       --out defaults to RESUME_IR_LOCAL_EVIDENCE_DIR/<default> when omitted\n       --trace-root defaults to RESUME_IR_QUERY_ARTIFACT_ROOT when omitted"
}

fn write_query_set_redacted_summary(
    data_dir: &Path,
    query_set_path: &Path,
    queries: &[String],
    frozen: &FrozenAgentReplayQueries,
) -> Result<QuerySetSummaryDigests> {
    let hasher = ContactHasher::load_or_create(data_dir).map_err(CliError::privacy)?;
    let bucket_counts = query_set_bucket_counts(queries);
    let split = split_query_set_for_holdout(&hasher, queries)?;
    let digests = QuerySetSummaryDigests {
        query_set_sha256: hasher
            .hmac_hex(
                "resume-ir:query-set-summary:v2:all",
                &build_query_set_hmac_payload(queries),
            )
            .map_err(CliError::privacy)?,
        tune_sha256: hasher
            .hmac_hex(
                "resume-ir:query-set-summary:v2:tune",
                &build_query_set_hmac_payload(&split.tune_queries),
            )
            .map_err(CliError::privacy)?,
        holdout_sha256: hasher
            .hmac_hex(
                "resume-ir:query-set-summary:v2:holdout",
                &build_query_set_hmac_payload(&split.holdout_queries),
            )
            .map_err(CliError::privacy)?,
    };
    let summary = serde_json::json!({
        "schema_version": QUERY_SET_SUMMARY_SCHEMA_VERSION,
        "privacy_boundary": "redacted_local_aggregate",
        "query_source": QuerySetSourceKind::TraceSourceSearchV1.as_str(),
        "query_count": queries.len(),
        "tune_query_count": split.tune_queries.len(),
        "holdout_query_count": split.holdout_queries.len(),
        "bucket_counts": bucket_counts,
        "tune_bucket_counts": split.tune_bucket_counts,
        "holdout_bucket_counts": split.holdout_bucket_counts,
        "candidate_queries_sampled": frozen.candidate_queries_sampled,
        "zero_hit_queries_dropped": frozen.zero_hit_queries_dropped,
        "query_set_sha256": &digests.query_set_sha256,
        "tune_sha256": &digests.tune_sha256,
        "holdout_sha256": &digests.holdout_sha256,
        "hmac_split": true,
        "contains_raw_query_text": false,
        "contains_raw_resume_text": false,
        "contains_candidate_results": false,
        "contains_local_paths": false,
    });
    let summary_text = serde_json::to_string_pretty(&summary)
        .map_err(|_| CliError::user("query set blocked: summary is unavailable"))?;
    let summary_path = query_set_summary_path(query_set_path);
    write_private_query_artifact(&summary_path, format!("{summary_text}\n").as_bytes())?;
    Ok(digests)
}

fn split_query_set_for_holdout<'a>(
    hasher: &ContactHasher,
    queries: &'a [String],
) -> Result<QuerySetSplit<'a>> {
    let mut split_sides = Vec::with_capacity(queries.len());
    let mut assignment_digests = Vec::with_capacity(queries.len());
    for query in queries {
        let assignment_digest = hasher
            .hmac_hex("resume-ir:query-set-summary:v2:assign", query.as_bytes())
            .map_err(CliError::privacy)?;
        let bucket = u8::from_str_radix(&assignment_digest[..2], 16)
            .map_err(|_| CliError::user("query set blocked: summary is unavailable"))?;
        split_sides.push(bucket >= 0x33);
        assignment_digests.push(assignment_digest);
    }
    rebalance_query_set_split_buckets(queries, &assignment_digests, &mut split_sides);
    if queries.len() > 1 && split_sides.iter().all(|side| *side) {
        if let Some(side) = split_sides.last_mut() {
            *side = false;
        }
    }
    if queries.len() > 1 && split_sides.iter().all(|side| !*side) {
        if let Some(side) = split_sides.last_mut() {
            *side = true;
        }
    }
    let mut tune_queries = Vec::new();
    let mut holdout_queries = Vec::new();
    let mut tune_bucket_counts = empty_query_set_bucket_counts();
    let mut holdout_bucket_counts = empty_query_set_bucket_counts();
    for (query, is_tune) in queries.iter().zip(split_sides.iter().copied()) {
        let bucket = query_set_bucket_for_query(query);
        if is_tune {
            tune_queries.push(query.as_str());
            increment_query_set_bucket_count(&mut tune_bucket_counts, bucket);
        } else {
            holdout_queries.push(query.as_str());
            increment_query_set_bucket_count(&mut holdout_bucket_counts, bucket);
        }
    }
    Ok(QuerySetSplit {
        tune_queries,
        holdout_queries,
        tune_bucket_counts,
        holdout_bucket_counts,
    })
}

fn rebalance_query_set_split_buckets(
    queries: &[String],
    assignment_digests: &[String],
    split_sides: &mut [bool],
) {
    for bucket in QUERY_BUCKETS {
        let mut indexes = queries
            .iter()
            .enumerate()
            .filter_map(|(index, query)| {
                (query_set_bucket_for_query(query) == bucket).then_some(index)
            })
            .collect::<Vec<_>>();
        if indexes.len() <= 1 {
            continue;
        }
        let has_tune = indexes.iter().any(|index| split_sides[*index]);
        let has_holdout = indexes.iter().any(|index| !split_sides[*index]);
        if has_tune && has_holdout {
            continue;
        }
        indexes.sort_by(|left, right| {
            assignment_digests[*left]
                .cmp(&assignment_digests[*right])
                .then_with(|| left.cmp(right))
        });
        if let Some(index) = indexes.first().copied() {
            split_sides[index] = !split_sides[index];
        }
    }
}

fn query_set_bucket_counts<T: AsRef<str>>(queries: &[T]) -> BTreeMap<&'static str, usize> {
    let mut counts = QUERY_BUCKETS
        .into_iter()
        .map(|bucket| (bucket, 0_usize))
        .collect::<BTreeMap<_, _>>();
    for query in queries {
        let bucket = query_set_bucket_for_query(query.as_ref());
        if let Some(count) = counts.get_mut(bucket) {
            *count += 1;
        }
    }
    counts
}

fn build_query_set_hmac_payload<T: AsRef<str>>(queries: &[T]) -> Vec<u8> {
    let mut payload = Vec::new();
    update_query_set_payload_string(&mut payload, QUERY_SET_SCHEMA_VERSION);
    update_query_set_payload_string(&mut payload, QUERY_SET_SUMMARY_SCHEMA_VERSION);
    update_query_set_payload_string(
        &mut payload,
        QuerySetSourceKind::TraceSourceSearchV1.as_str(),
    );
    payload.extend((queries.len() as u64).to_le_bytes());
    for query in queries {
        let query = query.as_ref();
        let shape = QuerySetSampleShape::from_query(query);
        let bucket = query_set_bucket_for_query(query);
        update_query_set_payload_string(&mut payload, query);
        update_query_set_payload_string(&mut payload, bucket);
        payload.extend((shape.term_count() as u64).to_le_bytes());
        payload.push(u8::from(shape.has_boolean()));
        payload.push(u8::from(shape.has_location()));
        payload.push(u8::from(shape.has_years()));
        payload.push(u8::from(shape.has_degree()));
        payload.push(u8::from(shape.has_skill()));
        payload.push(u8::from(shape.has_phrase()));
    }
    payload
}

fn query_set_bucket_for_query(query: &str) -> &'static str {
    QuerySetSampleShape::from_query(query).bucket()
}

fn update_query_set_payload_string(payload: &mut Vec<u8>, value: &str) {
    payload.extend((value.len() as u64).to_le_bytes());
    payload.extend(value.as_bytes());
}

fn query_set_summary_path(query_set_path: &Path) -> PathBuf {
    let file_name = query_set_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("query-set");
    let base_name = file_name.strip_suffix(".local.jsonl").unwrap_or(file_name);
    query_set_path.with_file_name(format!("{base_name}.summary.json"))
}

fn preflight_trace_backed_private_queries(
    data_dir: &Path,
    store: Option<&ReadMetaStore>,
    trace_root: &Path,
    max_queries: usize,
) -> Result<TraceQueryPreflight> {
    if !trace_root.is_dir() {
        return Err(CliError::user(
            "query set blocked: trace root is unavailable",
        ));
    }
    let mut coordinator = QueryCoordinator::open(data_dir).ok();
    let query_index_available = coordinator
        .as_mut()
        .is_some_and(|coordinator| coordinator.with_query(|_| Ok(())).is_ok());
    if !query_index_available {
        coordinator = None;
    }
    let corpus_summary = match store {
        Some(store) => benchmark_corpus_summary(data_dir, store)?,
        None => BenchmarkCorpusSummary::unavailable(),
    };

    let mut trace_paths = Vec::new();
    collect_runtime_trace_logs(trace_root, &mut trace_paths)?;
    let mut counts = TraceQuerySelectionCounts::default();
    let mut candidate_bucket_counts = empty_query_set_bucket_counts();
    let mut corpus_valid_bucket_counts = empty_query_set_bucket_counts();
    let mut seen = BTreeSet::new();

    for trace_path in trace_paths {
        counts.trace_logs += 1;
        let trace_file = fs::File::open(&trace_path)
            .map_err(|_| CliError::user("query set blocked: trace log is unavailable"))?;
        let mut reader = BufReader::new(trace_file);
        let mut line_buffer = Vec::new();
        while let Some(line) = read_bounded_trace_log_line(&mut reader, &mut line_buffer)? {
            counts.trace_lines += 1;
            if is_source_search_trace_line(&line) {
                counts.source_search_lines += 1;
            }
            let Some(query) = extract_source_search_trace_query(&line) else {
                continue;
            };
            counts.extracted_queries += 1;
            let Some(query) = normalize_trace_query_value(&query) else {
                counts.normalization_rejected += 1;
                continue;
            };
            if !seen.insert(query.clone()) {
                counts.duplicate_queries_dropped += 1;
                continue;
            }
            counts.candidate_queries_sampled += 1;
            increment_query_set_bucket_count(
                &mut candidate_bucket_counts,
                QuerySetSampleShape::from_query(&query).bucket(),
            );
            if let Some(coordinator) = coordinator.as_mut() {
                let has_local_hit = trace_query_has_local_hit(coordinator, &query)?;
                if !has_local_hit {
                    counts.zero_hit_queries_dropped += 1;
                    continue;
                }
                increment_query_set_bucket_count(
                    &mut corpus_valid_bucket_counts,
                    QuerySetSampleShape::from_query(&query).bucket(),
                );
            }
        }
    }

    let required_bucket_counts =
        trace_query_full_freeze_bucket_targets(max_queries).unwrap_or_default();
    let candidate_bucket_deficits =
        query_set_bucket_deficits(&required_bucket_counts, &candidate_bucket_counts);
    let corpus_valid_bucket_deficits =
        query_set_bucket_deficits(&required_bucket_counts, &corpus_valid_bucket_counts);
    let corpus_valid_queries = corpus_valid_bucket_counts.values().copied().sum();
    Ok(TraceQueryPreflight {
        counts,
        query_index_available,
        corpus_summary,
        candidate_bucket_counts,
        candidate_bucket_deficits,
        corpus_valid_queries,
        corpus_valid_bucket_counts,
        required_bucket_counts,
        corpus_valid_bucket_deficits,
    })
}

fn d10k_corpus_ready(summary: &BenchmarkCorpusSummary) -> bool {
    summary.document_count >= CURRENT_STAGE_D10K_DOCUMENT_MIN
        && summary.searchable_document_count >= CURRENT_STAGE_D10K_SEARCHABLE_DOCUMENT_MIN
        && summary.vector_indexed_document_count >= CURRENT_STAGE_D10K_VECTOR_DOCUMENT_MIN
}

fn d10k_corpus_deficits(summary: &BenchmarkCorpusSummary) -> BTreeMap<&'static str, u64> {
    BTreeMap::from([
        (
            "document_count",
            CURRENT_STAGE_D10K_DOCUMENT_MIN.saturating_sub(summary.document_count),
        ),
        (
            "searchable_document_count",
            CURRENT_STAGE_D10K_SEARCHABLE_DOCUMENT_MIN
                .saturating_sub(summary.searchable_document_count),
        ),
        (
            "vector_indexed_document_count",
            CURRENT_STAGE_D10K_VECTOR_DOCUMENT_MIN
                .saturating_sub(summary.vector_indexed_document_count),
        ),
    ])
}

fn d10k_corpus_not_ready_message(summary: &BenchmarkCorpusSummary) -> String {
    let deficits = d10k_corpus_deficits(summary)
        .into_iter()
        .map(|(field, deficit)| format!("{field}={deficit}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "query set blocked: D10K agent replay freeze requires a D10K-shaped indexed corpus; corpus deficits: {deficits}"
    )
}

fn freeze_trace_backed_private_queries(
    data_dir: &Path,
    _store: &ReadMetaStore,
    trace_root: &Path,
    max_queries: usize,
) -> Result<FrozenAgentReplayQueries> {
    if !trace_root.is_dir() {
        return Err(CliError::user(
            "query set blocked: trace root is unavailable",
        ));
    }
    let mut coordinator = QueryCoordinator::open(data_dir)
        .map_err(|_| CliError::user("query set blocked: search service is unavailable"))?;
    coordinator
        .with_query(|_| Ok(()))
        .map_err(|_| CliError::user("query set blocked: search service is unavailable"))?;

    let mut trace_paths = Vec::new();
    collect_runtime_trace_logs(trace_root, &mut trace_paths)?;
    if trace_paths.is_empty() {
        let counts = TraceQuerySelectionCounts::default();
        return Ok(FrozenAgentReplayQueries {
            queries: Vec::new(),
            candidate_queries_sampled: 0,
            zero_hit_queries_dropped: 0,
            insufficient_query_message: trace_query_insufficient_message(
                None,
                &empty_query_set_bucket_counts(),
                &counts,
                0,
            ),
        });
    }

    let mut queries = Vec::new();
    let mut seen = BTreeSet::new();
    let bucket_targets = trace_query_full_freeze_bucket_targets(max_queries);
    let mut selected_bucket_counts = empty_query_set_bucket_counts();
    let mut counts = TraceQuerySelectionCounts::default();

    for trace_path in trace_paths {
        counts.trace_logs += 1;
        let trace_file = fs::File::open(&trace_path)
            .map_err(|_| CliError::user("query set blocked: trace log is unavailable"))?;
        let mut reader = BufReader::new(trace_file);
        let mut line_buffer = Vec::new();
        while let Some(line) = read_bounded_trace_log_line(&mut reader, &mut line_buffer)? {
            counts.trace_lines += 1;
            if is_source_search_trace_line(&line) {
                counts.source_search_lines += 1;
            }
            let Some(query) = extract_source_search_trace_query(&line) else {
                continue;
            };
            counts.extracted_queries += 1;
            let Some(query) = normalize_trace_query_value(&query) else {
                counts.normalization_rejected += 1;
                continue;
            };
            if !seen.insert(query.clone()) {
                counts.duplicate_queries_dropped += 1;
                continue;
            }
            counts.candidate_queries_sampled += 1;
            let has_local_hit = trace_query_has_local_hit(&mut coordinator, &query)?;
            if !has_local_hit {
                counts.zero_hit_queries_dropped += 1;
                continue;
            }
            let bucket = QuerySetSampleShape::from_query(&query).bucket();
            if trace_query_bucket_can_accept(
                bucket_targets.as_ref(),
                &selected_bucket_counts,
                bucket,
            ) {
                queries.push(query);
                increment_query_set_bucket_count(&mut selected_bucket_counts, bucket);
                if trace_query_selection_complete(
                    &queries,
                    max_queries,
                    bucket_targets.as_ref(),
                    &selected_bucket_counts,
                ) {
                    return Ok(FrozenAgentReplayQueries {
                        queries,
                        candidate_queries_sampled: counts.candidate_queries_sampled,
                        zero_hit_queries_dropped: counts.zero_hit_queries_dropped,
                        insufficient_query_message: TRACE_QUERY_INSUFFICIENT_BASE_MESSAGE
                            .to_string(),
                    });
                }
            }
        }
    }

    let selected_queries = queries.len();
    Ok(FrozenAgentReplayQueries {
        queries,
        candidate_queries_sampled: counts.candidate_queries_sampled,
        zero_hit_queries_dropped: counts.zero_hit_queries_dropped,
        insufficient_query_message: trace_query_insufficient_message(
            bucket_targets.as_ref(),
            &selected_bucket_counts,
            &counts,
            selected_queries,
        ),
    })
}

fn query_set_bucket_deficits(
    required_bucket_counts: &BTreeMap<&'static str, usize>,
    observed_bucket_counts: &BTreeMap<&'static str, usize>,
) -> BTreeMap<&'static str, usize> {
    QUERY_BUCKETS
        .into_iter()
        .filter_map(|bucket| {
            let required = required_bucket_counts.get(bucket).copied().unwrap_or(0);
            let observed = observed_bucket_counts.get(bucket).copied().unwrap_or(0);
            let deficit = required.saturating_sub(observed);
            (deficit > 0).then_some((bucket, deficit))
        })
        .collect()
}

fn trace_query_insufficient_message(
    bucket_targets: Option<&BTreeMap<&'static str, usize>>,
    selected_bucket_counts: &BTreeMap<&'static str, usize>,
    counts: &TraceQuerySelectionCounts,
    selected_queries: usize,
) -> String {
    let mut message = TRACE_QUERY_INSUFFICIENT_BASE_MESSAGE.to_string();
    if let Some(bucket_targets) = bucket_targets {
        let deficits = QUERY_BUCKETS
            .into_iter()
            .filter_map(|bucket| {
                let target = bucket_targets.get(bucket).copied().unwrap_or(0);
                let selected = selected_bucket_counts.get(bucket).copied().unwrap_or(0);
                (selected < target).then(|| format!("{bucket}={}", target - selected))
            })
            .collect::<Vec<_>>();
        if !deficits.is_empty() {
            message.push_str("; bucket deficits: ");
            message.push_str(&deficits.join(","));
        }
    }
    message.push_str("; trace selection counts: ");
    message.push_str(&format!(
        "trace_logs={} trace_lines={} source_search_lines={} extracted_queries={} normalization_rejected={} duplicate_queries_dropped={} candidate_queries_sampled={} zero_hit_queries_dropped={} selected_queries={}",
        counts.trace_logs,
        counts.trace_lines,
        counts.source_search_lines,
        counts.extracted_queries,
        counts.normalization_rejected,
        counts.duplicate_queries_dropped,
        counts.candidate_queries_sampled,
        counts.zero_hit_queries_dropped,
        selected_queries
    ));
    message
}

fn trace_query_full_freeze_bucket_targets(
    max_queries: usize,
) -> Option<BTreeMap<&'static str, usize>> {
    if max_queries != D10K_TRACE_QUERY_SET_COUNT {
        return None;
    }
    Some(
        D10K_TRACE_QUERY_BUCKET_MIN_COUNTS
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
    )
}

fn empty_query_set_bucket_counts() -> BTreeMap<&'static str, usize> {
    QUERY_BUCKETS
        .into_iter()
        .map(|bucket| (bucket, 0_usize))
        .collect()
}

fn trace_query_bucket_can_accept(
    bucket_targets: Option<&BTreeMap<&'static str, usize>>,
    selected_bucket_counts: &BTreeMap<&'static str, usize>,
    bucket: &'static str,
) -> bool {
    let Some(bucket_targets) = bucket_targets else {
        return true;
    };
    selected_bucket_counts.get(bucket).copied().unwrap_or(0)
        < bucket_targets.get(bucket).copied().unwrap_or(0)
}

fn increment_query_set_bucket_count(
    selected_bucket_counts: &mut BTreeMap<&'static str, usize>,
    bucket: &'static str,
) {
    if let Some(count) = selected_bucket_counts.get_mut(bucket) {
        *count += 1;
    }
}

fn trace_query_selection_complete(
    queries: &[String],
    max_queries: usize,
    bucket_targets: Option<&BTreeMap<&'static str, usize>>,
    selected_bucket_counts: &BTreeMap<&'static str, usize>,
) -> bool {
    let Some(bucket_targets) = bucket_targets else {
        return queries.len() >= max_queries;
    };
    bucket_targets
        .iter()
        .all(|(bucket, target)| selected_bucket_counts.get(bucket).copied().unwrap_or(0) >= *target)
}

fn collect_runtime_trace_logs(root: &Path, trace_paths: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(root)
        .map_err(|_| CliError::user("query set blocked: trace root is unavailable"))?;
    let mut child_dirs = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|_| CliError::user("query set blocked: trace root is unavailable"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| CliError::user("query set blocked: trace root is unavailable"))?;
        if file_type.is_dir() {
            child_dirs.push(path);
            continue;
        }
        if file_type.is_file()
            && path.file_name().and_then(|name| name.to_str()) == Some("trace.log")
            && path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                == Some("runtime")
        {
            trace_paths.push(path);
        }
    }
    child_dirs.sort();
    for child_dir in child_dirs {
        collect_runtime_trace_logs(&child_dir, trace_paths)?;
    }
    trace_paths.sort();
    Ok(())
}

fn read_bounded_trace_log_line<R: BufRead>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> Result<Option<String>> {
    buffer.clear();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|_| CliError::user("query set blocked: trace log is unreadable"))?;
        if available.is_empty() {
            if buffer.is_empty() {
                return Ok(None);
            }
            break;
        }
        let take_len = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |index| index + 1);
        if buffer.len().saturating_add(take_len) > TRACE_QUERY_LINE_MAX_BYTES {
            return Err(CliError::user(
                "query set blocked: trace log line is too large",
            ));
        }
        let found_newline = available[take_len - 1] == b'\n';
        buffer.extend_from_slice(&available[..take_len]);
        reader.consume(take_len);
        if found_newline {
            break;
        }
    }
    if buffer.ends_with(b"\n") {
        buffer.pop();
    }
    if buffer.ends_with(b"\r") {
        buffer.pop();
    }
    String::from_utf8(buffer.clone())
        .map(Some)
        .map_err(|_| CliError::user("query set blocked: trace log is unreadable"))
}

fn extract_source_search_trace_query(line: &str) -> Option<String> {
    let mut segments = line.split(" | ");
    let timestamp = segments.next()?;
    if !timestamp.starts_with('[') {
        return None;
    }
    if segments.next()? != "tool_called" {
        return None;
    }
    let remaining = segments.collect::<Vec<_>>();
    let tool_index = remaining
        .iter()
        .position(|segment| *segment == "tool=source_search")?;
    let summary_index = tool_index + 1;
    if summary_index >= remaining.len() {
        return None;
    }
    let query = remaining[summary_index].to_string();
    if is_source_search_trace_non_keyword_segment(&query) {
        return None;
    }
    Some(query)
}

fn is_source_search_trace_line(line: &str) -> bool {
    let mut segments = line.split(" | ");
    let Some(timestamp) = segments.next() else {
        return false;
    };
    if !timestamp.starts_with('[') || segments.next() != Some("tool_called") {
        return false;
    }
    segments.any(|segment| segment == "tool=source_search")
}

fn is_source_search_trace_non_keyword_segment(query: &str) -> bool {
    matches!(query.split_once('='), Some((key, _)) if is_source_search_trace_metadata_key(key))
}

fn is_source_search_trace_metadata_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 32
        && key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn normalize_trace_query_value(value: &str) -> Option<String> {
    let normalized = normalize_query_set_query(value)?;
    if normalized.contains('@')
        || normalized.contains('\\')
        || normalized.contains("://")
        || contains_disallowed_trace_query_slash(&normalized)
        || contains_sensitive_digit_run(&normalized)
    {
        return None;
    }
    Some(normalized)
}

fn contains_disallowed_trace_query_slash(value: &str) -> bool {
    value
        .split_whitespace()
        .any(trace_query_token_has_disallowed_slash)
}

fn trace_query_token_has_disallowed_slash(token: &str) -> bool {
    if !token.contains('/') {
        return false;
    }
    if token.starts_with('/')
        || token.ends_with('/')
        || token.contains("//")
        || token.matches('/').count() > 1
        || token.contains('.')
        || token.contains(':')
        || token.contains('~')
    {
        return true;
    }
    let Some((left, right)) = token.split_once('/') else {
        return true;
    };
    !is_safe_slash_query_side(left) || !is_safe_slash_query_side(right)
}

fn is_safe_slash_query_side(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= 16
        && value
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '+' | '#'))
}

fn contains_sensitive_digit_run(value: &str) -> bool {
    let mut digit_run = 0_usize;
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            digit_run += 1;
            if digit_run >= 6 {
                return true;
            }
        } else {
            digit_run = 0;
        }
    }
    false
}

fn trace_query_has_local_hit(coordinator: &mut QueryCoordinator, query: &str) -> Result<bool> {
    let plan = match plan_search(query, 1) {
        Ok(plan) => plan,
        Err(_) => return Ok(false),
    };
    coordinator
        .with_query(|scope| {
            let candidates =
                scope.fulltext_candidates(plan.query_text(), HitLimit::new(plan.limit())?, None)?;
            if candidates.is_empty() {
                return Ok(false);
            }
            let projections = candidates
                .iter()
                .map(|candidate| candidate.projection.clone())
                .collect::<Vec<_>>();
            scope
                .hydrate_exact_hits(&projections)
                .map(|hits| !hits.is_empty())
        })
        .map_err(search_runtime_cli_error)
}

fn benchmark_corpus_summary_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let args = parse_benchmark_corpus_summary_args(args)?;
    let store = open_store(data_dir)?;
    let summary = benchmark_corpus_summary(data_dir, &store)?;

    if args.json {
        let report = serde_json::json!({
            "schema_version": "benchmark-corpus-summary.v1",
            "privacy_boundary": "redacted_local_aggregate",
            "document_count": summary.document_count,
            "searchable_document_count": summary.searchable_document_count,
            "vector_indexed_document_count": summary.vector_indexed_document_count,
            "active_vector_document_count": summary.active_vector_document_count,
            "vector_count": summary.vector_count,
            "vector_deleted_count": summary.vector_deleted_count,
            "vector_index_state": summary.vector_index_state,
            "vector_search_backend": summary.vector_search_backend,
            "hot_index_fully_covered": summary.hot_index_fully_covered,
            "document_status_counts": summary.document_status_counts,
            "ingest_job_status_counts": summary.ingest_job_status_counts,
            "ingest_job_kind_status_counts": summary.ingest_job_kind_status_counts,
            "ingest_job_failure_counts": summary.ingest_job_failure_counts,
            "contains_raw_resume_text": false,
            "contains_resume_paths": false,
            "contains_queries": false,
            "contains_sample_ids": false,
        });
        let report = serde_json::to_string_pretty(&report)
            .map_err(|_| CliError::user("benchmark corpus summary unavailable"))?;
        println!("{report}");
        return Ok(());
    }

    println!("resume-ir benchmark corpus summary");
    println!("privacy boundary: redacted local aggregate");
    println!("documents: {}", summary.document_count);
    println!(
        "searchable documents: {}",
        summary.searchable_document_count
    );
    println!(
        "vector indexed documents: {}",
        summary.vector_indexed_document_count
    );
    println!(
        "active vector documents: {}",
        summary.active_vector_document_count
    );
    println!("vector count: {}", summary.vector_count);
    println!("vector tombstones: {}", summary.vector_deleted_count);
    println!("vector index: {}", summary.vector_index_state);
    println!("vector backend: {}", summary.vector_search_backend);
    println!(
        "hot index fully covered: {}",
        summary.hot_index_fully_covered
    );
    println!(
        "document status counts: {}",
        redacted_count_map_label(&summary.document_status_counts)
    );
    println!(
        "ingest job status counts: {}",
        redacted_count_map_label(&summary.ingest_job_status_counts)
    );
    println!(
        "ingest job failure counts: {}",
        redacted_count_map_label(&summary.ingest_job_failure_counts)
    );
    println!("raw resume text: <redacted>");
    println!("resume paths: <redacted>");
    println!("queries: <redacted>");
    println!("sample ids: <redacted>");
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BenchmarkCorpusSummary {
    document_count: u64,
    searchable_document_count: u64,
    vector_indexed_document_count: u64,
    active_vector_document_count: u64,
    vector_count: u64,
    vector_deleted_count: u64,
    vector_index_state: &'static str,
    vector_search_backend: &'static str,
    hot_index_fully_covered: bool,
    document_status_counts: BTreeMap<String, u64>,
    ingest_job_status_counts: BTreeMap<String, u64>,
    ingest_job_kind_status_counts: BTreeMap<String, BTreeMap<String, u64>>,
    ingest_job_failure_counts: BTreeMap<String, u64>,
}

impl BenchmarkCorpusSummary {
    fn unavailable() -> Self {
        Self {
            document_count: 0,
            searchable_document_count: 0,
            vector_indexed_document_count: 0,
            active_vector_document_count: 0,
            vector_count: 0,
            vector_deleted_count: 0,
            vector_index_state: "unavailable",
            vector_search_backend: "none",
            hot_index_fully_covered: false,
            document_status_counts: BTreeMap::new(),
            ingest_job_status_counts: BTreeMap::new(),
            ingest_job_kind_status_counts: BTreeMap::new(),
            ingest_job_failure_counts: BTreeMap::new(),
        }
    }
}

fn benchmark_corpus_summary(
    _data_dir: &Path,
    store: &ReadMetaStore,
) -> Result<BenchmarkCorpusSummary> {
    let document_count = store.visible_document_count().map_err(CliError::store)?;
    let documents = store.visible_documents().map_err(CliError::store)?;
    let ingest_jobs = store.ingest_jobs().map_err(CliError::store)?;
    let document_status_counts = benchmark_document_status_counts(&documents);
    let ingest_job_status_counts = benchmark_ingest_job_status_counts(&ingest_jobs);
    let ingest_job_kind_status_counts = benchmark_ingest_job_kind_status_counts(&ingest_jobs);
    let ingest_job_failure_counts = benchmark_ingest_job_failure_counts(&ingest_jobs);
    let projection = store.search_projection_state().map_err(CliError::store)?;
    let publication = projection.publication.as_deref();
    let searchable_document_count = publication
        .and_then(|publication| publication.fulltext.as_ref())
        .map(|fulltext| fulltext.document_count())
        .unwrap_or(0);
    let vector = publication.and_then(|publication| publication.vector.as_ref());
    let vector_indexed_document_count = vector.map(|vector| vector.document_count()).unwrap_or(0);
    let active_vector_document_count = vector_indexed_document_count;
    let vector_count = vector.map(|vector| vector.vector_count()).unwrap_or(0);
    let vector_deleted_count = 0;
    let vector_enabled =
        vector.is_some_and(|vector| matches!(vector.mode(), VectorSnapshotMode::Enabled { .. }));
    let vector_index_state = if publication.is_some() {
        "available"
    } else {
        "unavailable"
    };
    let vector_search_backend = if vector_enabled { "hnsw_ann" } else { "none" };
    let hot_index_fully_covered = document_count > 0
        && searchable_document_count >= document_count
        && vector_indexed_document_count >= document_count;

    Ok(BenchmarkCorpusSummary {
        document_count,
        searchable_document_count,
        vector_indexed_document_count,
        active_vector_document_count,
        vector_count,
        vector_deleted_count,
        vector_index_state,
        vector_search_backend,
        hot_index_fully_covered,
        document_status_counts,
        ingest_job_status_counts,
        ingest_job_kind_status_counts,
        ingest_job_failure_counts,
    })
}

fn benchmark_document_status_counts(documents: &[Document]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for document in documents {
        increment_count(&mut counts, document_status_label(document.status));
    }
    counts
}

fn benchmark_ingest_job_status_counts(jobs: &[meta_store::IngestJob]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for job in jobs {
        increment_count(&mut counts, ingest_job_status_label(job.status));
    }
    counts
}

fn benchmark_ingest_job_kind_status_counts(
    jobs: &[meta_store::IngestJob],
) -> BTreeMap<String, BTreeMap<String, u64>> {
    let mut counts = BTreeMap::new();
    for job in jobs {
        let kind_counts = counts
            .entry(ingest_job_kind_label(job.kind).to_string())
            .or_default();
        increment_count(kind_counts, ingest_job_status_label(job.status));
    }
    counts
}

fn benchmark_ingest_job_failure_counts(jobs: &[meta_store::IngestJob]) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for job in jobs {
        if let Some(failure_kind) = job.failure_kind {
            increment_count(&mut counts, ingest_job_failure_kind_label(failure_kind));
        }
    }
    counts
}

fn increment_count(counts: &mut BTreeMap<String, u64>, label: &'static str) {
    *counts.entry(label.to_string()).or_default() += 1;
}

fn redacted_count_map_label(counts: &BTreeMap<String, u64>) -> String {
    if counts.is_empty() {
        return "{}".to_string();
    }

    counts
        .iter()
        .map(|(label, count)| format!("{label}={count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BenchmarkCorpusSummaryArgs {
    json: bool,
}

fn parse_benchmark_corpus_summary_args(args: &[String]) -> Result<BenchmarkCorpusSummaryArgs> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            _ => return Err(CliError::usage(benchmark_corpus_summary_usage())),
        }
    }
    Ok(BenchmarkCorpusSummaryArgs { json })
}

fn benchmark_corpus_summary_usage() -> &'static str {
    "usage: resume-cli benchmark-corpus-summary [--json]"
}

fn benchmark_query_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| CliError::user("benchmark query input is unavailable"))
}

fn benchmark_query_top_k() -> Result<usize> {
    let value = std::env::var("RESUME_IR_QUERY_TOP_K")
        .map_err(|_| CliError::user("benchmark query top-k is invalid"))?;
    parse_positive_usize(&value).map_err(|_| CliError::user("benchmark query top-k is invalid"))
}

fn benchmark_query_mode() -> Result<SearchMode> {
    let value = std::env::var("RESUME_IR_QUERY_MODE")
        .map_err(|_| CliError::user("benchmark query mode is invalid"))?;
    SearchMode::parse(&value).ok_or_else(|| CliError::user("benchmark query mode is invalid"))
}

fn benchmark_query_protocol_usage() -> &'static str {
    "usage: resume-cli benchmark-query-protocol --batch-jsonl [--embedding-command <path>] [--model-id <id>] [--dimension <n>] [--vector-top-k <n>] [--embedding-timeout-ms <ms>]"
}

enum LocalSearchOutcome {
    Hits(Vec<SearchOutputHit>),
    SearchServiceUnavailable,
}

fn run_local_search(data_dir: &Path, search_args: &SearchArgs) -> Result<LocalSearchOutcome> {
    execute_local_search(data_dir, search_args, None)
}

fn run_benchmark_query_protocol_search(
    data_dir: &Path,
    search_args: &SearchArgs,
    timings: &mut BenchmarkQueryProtocolStageTimings,
) -> Result<LocalSearchOutcome> {
    execute_local_search(data_dir, search_args, Some(timings))
}

#[derive(Clone)]
struct PreparedSemanticQuery {
    model_id: String,
    dimension: usize,
    query: SemanticQueryVector,
}

#[derive(Clone)]
struct LocalRankedCandidate {
    projection: core_domain::ActiveSearchProjection,
    score: f32,
    file_name: String,
    snippet: String,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum LocalFoldIdentity {
    Candidate(CandidateId),
    Version(ResumeVersionId),
}

fn execute_local_search(
    data_dir: &Path,
    search_args: &SearchArgs,
    mut timings: Option<&mut BenchmarkQueryProtocolStageTimings>,
) -> Result<LocalSearchOutcome> {
    let mut coordinator = QueryCoordinator::open(data_dir).map_err(search_runtime_cli_error)?;
    let semantic = prepare_local_semantic_query(&mut coordinator, search_args)?;
    let candidate_limit = search_args
        .top_k
        .saturating_mul(5)
        .clamp(search_args.top_k, 100);
    let semantic_candidate_limit = search_args
        .vector_top_k
        .unwrap_or(candidate_limit)
        .clamp(search_args.top_k, 100);
    let plan_started = Instant::now();
    let plan = plan_search(&search_args.query, candidate_limit)
        .map_err(|_| CliError::user("search query is outside semantic bounds"))?;
    if let Some(timings) = timings.as_deref_mut() {
        timings.query_parse_ms += duration_ms(plan_started.elapsed());
    }
    let query = plan.query_text().to_string();
    let filter = search_projection_filter(&search_args.filters)?;
    let hit_limit = HitLimit::new(candidate_limit).map_err(search_runtime_cli_error)?;
    let semantic_hit_limit =
        HitLimit::new(semantic_candidate_limit).map_err(search_runtime_cli_error)?;
    let selection_limit = SelectionLimit::new(meta_store::MAX_BOUNDED_FILTER_SELECTION)
        .map_err(search_runtime_cli_error)?;

    let result = coordinator.with_query(|scope| {
        validate_local_semantic_contract(scope.semantic_contract(), semantic.as_ref())?;
        let filter_selection = if filter.predicates().is_empty() {
            None
        } else {
            let started = Instant::now();
            let selection = scope.filter_selection(&filter, selection_limit)?;
            if let Some(timings) = timings.as_deref_mut() {
                timings.prefilter_ms += duration_ms(started.elapsed());
            }
            Some(selection)
        };

        let candidates = match search_args.mode {
            SearchMode::FullText => local_fulltext_candidates(
                &scope,
                &query,
                semantic_hit_limit,
                filter_selection.as_ref(),
                timings.as_deref_mut(),
            )?,
            SearchMode::Semantic => local_semantic_candidates(
                &scope,
                semantic
                    .as_ref()
                    .ok_or_else(SearchRuntimeError::integrity_violation)?
                    .query
                    .clone(),
                hit_limit,
                filter_selection.as_ref(),
                timings.as_deref_mut(),
            )?,
            SearchMode::Hybrid => {
                let lexical = local_fulltext_candidates(
                    &scope,
                    &query,
                    semantic_hit_limit,
                    filter_selection.as_ref(),
                    timings.as_deref_mut(),
                )?;
                let semantic_candidates = local_semantic_candidates(
                    &scope,
                    semantic
                        .as_ref()
                        .ok_or_else(SearchRuntimeError::integrity_violation)?
                        .query
                        .clone(),
                    hit_limit,
                    filter_selection.as_ref(),
                    timings.as_deref_mut(),
                )?;
                let started = Instant::now();
                let fused = fuse_local_candidates(lexical, semantic_candidates, candidate_limit);
                if let Some(timings) = timings.as_deref_mut() {
                    timings.fusion_ms += duration_ms(started.elapsed());
                }
                fused
            }
        };
        hydrate_local_candidates(&scope, candidates, search_args.top_k, timings)
    });
    match result {
        Ok(hits) => Ok(LocalSearchOutcome::Hits(hits)),
        Err(error) if error.code() == SearchRuntimeErrorCode::Unavailable => {
            Ok(LocalSearchOutcome::SearchServiceUnavailable)
        }
        Err(error) => Err(search_runtime_cli_error(error)),
    }
}

fn prepare_local_semantic_query(
    coordinator: &mut QueryCoordinator,
    search_args: &SearchArgs,
) -> Result<Option<PreparedSemanticQuery>> {
    if search_args.mode == SearchMode::FullText {
        return Ok(None);
    }
    let contract = coordinator
        .with_query(|scope| Ok(scope.semantic_contract()))
        .map_err(search_runtime_cli_error)?;
    let SemanticContract::Enabled {
        model_id: expected_model,
        dimension: expected_dimension,
    } = contract
    else {
        return Err(CliError::user(
            "semantic search unavailable: SEMANTIC_DISABLED",
        ));
    };
    let command = search_args.embedding_command.clone().ok_or_else(|| {
        CliError::user("semantic search unavailable: embedding runtime is not configured")
    })?;
    let model_id = search_args.model_id.as_deref().ok_or_else(|| {
        CliError::user("semantic search unavailable: embedding model is not configured")
    })?;
    let dimension = search_args.dimension.unwrap_or(expected_dimension);
    if model_id != expected_model || dimension != expected_dimension {
        return Err(CliError::user(
            "semantic search unavailable: embedding contract mismatch",
        ));
    }
    let embedder = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), model_id, expected_dimension)
            .map_err(CliError::embedding)?
            .with_timeout_ms(search_args.embedding_timeout_ms)
            .map_err(CliError::embedding)?,
    );
    let input = EmbeddingInput::new("query", search_args.query.as_str());
    let query = embedder
        .embed_batch(
            &[input],
            EmbeddingBudget::new(1, search_args.query.len().max(1)),
        )
        .map_err(CliError::embedding)?
        .into_iter()
        .next()
        .ok_or_else(|| CliError::user("semantic search query embedding is unavailable"))?;
    Ok(Some(PreparedSemanticQuery {
        model_id: model_id.to_string(),
        dimension,
        query: SemanticQueryVector::new(query.values().to_vec())
            .map_err(search_runtime_cli_error)?,
    }))
}

fn validate_local_semantic_contract(
    contract: SemanticContract,
    prepared: Option<&PreparedSemanticQuery>,
) -> std::result::Result<(), SearchRuntimeError> {
    match (contract, prepared) {
        (_, None) => Ok(()),
        (SemanticContract::Disabled, Some(_)) => Err(SearchRuntimeError::integrity_violation()),
        (
            SemanticContract::Enabled {
                model_id,
                dimension,
            },
            Some(prepared),
        ) if model_id == prepared.model_id && dimension == prepared.dimension => Ok(()),
        (SemanticContract::Enabled { .. }, Some(_)) => {
            Err(SearchRuntimeError::integrity_violation())
        }
    }
}

fn local_fulltext_candidates(
    scope: &QueryScope<'_>,
    query: &str,
    limit: HitLimit,
    selection: Option<&FilterSelection>,
    timings: Option<&mut BenchmarkQueryProtocolStageTimings>,
) -> std::result::Result<Vec<LocalRankedCandidate>, SearchRuntimeError> {
    let started = Instant::now();
    let hits = scope.fulltext_candidates(query, limit, selection)?;
    if let Some(timings) = timings {
        timings.bm25_ms += duration_ms(started.elapsed());
    }
    Ok(hits.into_iter().map(local_fulltext_candidate).collect())
}

fn local_fulltext_candidate(hit: FullTextCandidate) -> LocalRankedCandidate {
    LocalRankedCandidate {
        projection: hit.projection,
        score: hit.score,
        file_name: hit.file_name,
        snippet: hit.snippet,
    }
}

fn local_semantic_candidates(
    scope: &QueryScope<'_>,
    query: SemanticQueryVector,
    limit: HitLimit,
    selection: Option<&FilterSelection>,
    timings: Option<&mut BenchmarkQueryProtocolStageTimings>,
) -> std::result::Result<Vec<LocalRankedCandidate>, SearchRuntimeError> {
    let started = Instant::now();
    let hits = scope.semantic_candidates(query, limit, selection)?;
    if let Some(timings) = timings {
        timings.ann_ms += duration_ms(started.elapsed());
    }
    Ok(hits.into_iter().map(local_semantic_candidate).collect())
}

fn local_semantic_candidate(hit: SemanticCandidate) -> LocalRankedCandidate {
    LocalRankedCandidate {
        projection: hit.projection,
        score: hit.score,
        file_name: String::new(),
        snippet: "semantic match".to_string(),
    }
}

fn fuse_local_candidates(
    lexical: Vec<LocalRankedCandidate>,
    semantic: Vec<LocalRankedCandidate>,
    limit: usize,
) -> Vec<LocalRankedCandidate> {
    let mut by_document = BTreeMap::<String, LocalRankedCandidate>::new();
    for candidate in semantic.iter().chain(lexical.iter()) {
        by_document
            .entry(candidate.projection.document_id.to_string())
            .and_modify(|stored| {
                if stored.file_name.is_empty() && !candidate.file_name.is_empty() {
                    stored.file_name.clone_from(&candidate.file_name);
                    stored.snippet.clone_from(&candidate.snippet);
                }
            })
            .or_insert_with(|| candidate.clone());
    }
    let recall = HybridRecall::new(
        local_ranked_for_fusion(&lexical),
        local_ranked_for_fusion(&semantic),
    );
    fuse_hybrid_rrf(recall, 60.0, limit)
        .into_iter()
        .filter_map(|ranked| {
            by_document.remove(ranked.doc_id()).map(|mut candidate| {
                candidate.score = ranked.score();
                candidate
            })
        })
        .collect()
}

fn local_ranked_for_fusion(candidates: &[LocalRankedCandidate]) -> Vec<RankedHit> {
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            RankedHit::new(
                candidate.projection.document_id.to_string(),
                index + 1,
                candidate.score,
            )
        })
        .collect()
}

fn hydrate_local_candidates(
    scope: &QueryScope<'_>,
    candidates: Vec<LocalRankedCandidate>,
    top_k: usize,
    timings: Option<&mut BenchmarkQueryProtocolStageTimings>,
) -> std::result::Result<Vec<SearchOutputHit>, SearchRuntimeError> {
    let projections = candidates
        .iter()
        .map(|candidate| candidate.projection.clone())
        .collect::<Vec<_>>();
    let started = Instant::now();
    let hydrated = scope.hydrate_exact_hits(&projections)?;
    if let Some(timings) = timings {
        timings.bulk_hydrate_ms += duration_ms(started.elapsed());
    }
    let mut seen = BTreeSet::new();
    let mut output = Vec::with_capacity(top_k.min(hydrated.len()));
    for (candidate, metadata) in candidates.into_iter().zip(hydrated) {
        if metadata.selection.document_id != candidate.projection.document_id
            || metadata.selection.resume_version_id != candidate.projection.resume_version_id
        {
            return Err(SearchRuntimeError::integrity_violation());
        }
        if !seen.insert(local_fold_identity(&metadata)) {
            continue;
        }
        output.push(SearchOutputHit {
            rank: output.len() + 1,
            selection: metadata.selection,
            file_name: if candidate.file_name.is_empty() {
                metadata.document.file_name
            } else {
                candidate.file_name
            },
            snippet: candidate.snippet,
        });
        if output.len() == top_k {
            break;
        }
    }
    Ok(output)
}

fn local_fold_identity(hit: &HydratedSearchHit) -> LocalFoldIdentity {
    hit.candidate_id
        .clone()
        .map(LocalFoldIdentity::Candidate)
        .unwrap_or_else(|| LocalFoldIdentity::Version(hit.selection.resume_version_id.clone()))
}

fn search_runtime_cli_error(error: SearchRuntimeError) -> CliError {
    CliError::user(match error.code() {
        SearchRuntimeErrorCode::Unavailable => "search service unavailable",
        SearchRuntimeErrorCode::Integrity => "search runtime integrity failure",
        SearchRuntimeErrorCode::SemanticDisabled => {
            "semantic search unavailable: SEMANTIC_DISABLED"
        }
        SearchRuntimeErrorCode::SelectionTooLarge => "search filter selection is too large",
        SearchRuntimeErrorCode::InvalidRequest => "search request is invalid",
    })
}

fn search_ipc_command(endpoint: &IpcSearchEndpoint, search_args: &SearchArgs) -> Result<()> {
    let token_file = search_args
        .ipc_token_file
        .as_ref()
        .ok_or_else(|| CliError::usage(search_usage()))?;
    search_ipc_command_with_token_file(endpoint, token_file, search_args)
}

fn search_ipc_command_with_token_file(
    endpoint: &IpcSearchEndpoint,
    token_file: &Path,
    search_args: &SearchArgs,
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .map_err(|_| CliError::user("unable to read daemon search ipc token"))?;
    let token = validate_daemon_ipc_token(&token, "daemon search ipc token is invalid")?;
    let request_id = new_search_ipc_request_id()?;
    let body = search_ipc_request_body(&request_id, search_args);

    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon search ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon search ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon search ipc"))?;
    let request = format!(
        "POST /search HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.addr,
        token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon search ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon search ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon search ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200 ") && !status_line.starts_with("HTTP/1.0 200 ") {
        return Err(CliError::user("daemon search ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon search ipc returned invalid json"))?;
    render_search_ipc_result(&body, &request_id)?;
    Ok(())
}

fn search_ipc_request_body(request_id: &str, search_args: &SearchArgs) -> String {
    serde_json::json!({
        "schema_version": SEARCH_IPC_REQUEST_SCHEMA_VERSION,
        "request_id": request_id,
        "client_capability": "codex_validation",
        "deadline_ms": SEARCH_IPC_DEFAULT_DEADLINE_MS,
        "payload": {
            "query": search_args.query.as_str(),
            "mode": search_args.mode.label(),
            "top_k": search_args.top_k,
            "filters": search_filters_json(&search_args.filters),
        },
    })
    .to_string()
}

fn new_search_ipc_request_id() -> Result<String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliError::user("system clock is before the Unix epoch"))?;
    Ok(format!(
        "cli-search-{}-{}",
        std::process::id(),
        duration.as_nanos()
    ))
}

fn search_filters_json(filters: &SearchFilters) -> serde_json::Value {
    serde_json::json!({
        "degree_min": filters.degree_min().map(DegreeLevel::canonical),
        "names_any": filters.names_any(),
        "school_tiers_any": filters
            .school_tiers_any()
            .iter()
            .map(|school_tier| school_tier.canonical())
            .collect::<Vec<_>>(),
        "schools_any": filters.schools_any(),
        "majors_any": filters.majors_any(),
        "certificates_any": filters.certificates_any(),
        "date_range_overlaps": filters
            .date_range_overlaps()
            .map(|date_range| date_range.canonical()),
        "companies_any": filters.companies_any(),
        "titles_any": filters.titles_any(),
        "locations_any": filters.locations_any(),
        "skills_any": filters.skills_any(),
        "contact_hashes_any": filters.contact_hashes_any(),
        "years_experience_min": filters.years_experience_min(),
    })
}

fn render_search_ipc_result(body: &serde_json::Value, request_id: &str) -> Result<()> {
    if json_str(body, "schema_version") != Some(SEARCH_IPC_RESPONSE_SCHEMA_VERSION)
        || json_str(body, "status") != Some("ok")
        || json_str(body, "request_id") != Some(request_id)
    {
        return Err(CliError::user(
            "daemon search ipc returned invalid protocol",
        ));
    }
    if json_str(body, "search_index") == Some("not_ready") {
        println!("search index not available yet");
    }
    let results = body
        .get("results")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
    println!("results: {}", results.len());
    for result in results {
        let rank = result
            .get("rank")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
        let selection = result
            .get("selection")
            .and_then(parse_search_selection_json)
            .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
        let file_name = json_str(result, "file_name")
            .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
        let snippet = json_str(result, "snippet")
            .ok_or_else(|| CliError::user("daemon search ipc returned invalid protocol"))?;
        println!("rank: {rank}");
        println!("doc_id: {}", selection.document_id);
        println!("version_id: {}", selection.resume_version_id);
        println!("visible_epoch: {}", selection.visible_epoch);
        println!("file_name: {}", redact_search_file_name(file_name));
        println!("snippet: {}", redact_contact_values(snippet));
    }
    Ok(())
}

fn search_selection_json(selection: &SearchSelection) -> serde_json::Value {
    serde_json::json!({
        "doc_id": selection.document_id.as_str(),
        "version_id": selection.resume_version_id.as_str(),
        "visible_epoch": selection.visible_epoch,
    })
}

fn parse_search_selection_json(value: &serde_json::Value) -> Option<SearchSelection> {
    let object = value.as_object()?;
    if object.len() != 3 {
        return None;
    }
    let document_id = DocumentId::from_str(json_str(value, "doc_id")?).ok()?;
    let resume_version_id = ResumeVersionId::from_str(json_str(value, "version_id")?).ok()?;
    let visible_epoch = value.get("visible_epoch")?.as_u64()?;
    (visible_epoch > 0).then_some(SearchSelection {
        document_id,
        resume_version_id,
        visible_epoch,
    })
}

fn validate_search_selection_json(
    value: &serde_json::Value,
    expected: &SearchSelection,
) -> Result<()> {
    if parse_search_selection_json(value).as_ref() == Some(expected) {
        Ok(())
    } else {
        Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ))
    }
}

fn search_projection_filter(filters: &SearchFilters) -> Result<SearchProjectionFilter> {
    let mut predicates = Vec::new();
    if let Some(degree) = filters.degree_min() {
        predicates.push(SearchProjectionPredicate::EntityValuesAny {
            entity_type: EntityType::Degree,
            normalized_values: degree_filter_values(degree),
            min_confidence: FIELD_FILTER_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::Exact,
        });
    }
    push_search_text_filter(&mut predicates, EntityType::Name, filters.names_any());
    push_search_school_tier_filter(&mut predicates, filters.school_tiers_any());
    push_search_text_filter(&mut predicates, EntityType::School, filters.schools_any());
    push_search_text_filter(&mut predicates, EntityType::Major, filters.majors_any());
    push_search_text_filter(
        &mut predicates,
        EntityType::Certificate,
        filters.certificates_any(),
    );
    if let Some(range) = filters.date_range_overlaps() {
        predicates.push(SearchProjectionPredicate::DateRangeOverlap {
            start_month: range.start_month(),
            end_month: range.end_month(),
            min_confidence: FIELD_FILTER_CONFIDENCE_THRESHOLD,
        });
    }
    push_search_text_filter(
        &mut predicates,
        EntityType::Company,
        filters.companies_any(),
    );
    push_search_text_filter(&mut predicates, EntityType::Title, filters.titles_any());
    push_search_text_filter(
        &mut predicates,
        EntityType::Location,
        filters.locations_any(),
    );
    push_search_text_filter(&mut predicates, EntityType::Skill, filters.skills_any());
    if !filters.contact_hashes_any().is_empty() {
        let hashes = filters
            .contact_hashes_any()
            .iter()
            .map(|value| {
                ContactHash::from_keyed_digest(value.clone())
                    .map_err(|_| CliError::user("search contact filter is invalid"))
            })
            .collect::<Result<Vec<_>>>()?;
        predicates.push(SearchProjectionPredicate::ContactHashesAny(hashes));
    }
    if let Some(minimum) = filters.years_experience_min() {
        predicates.push(SearchProjectionPredicate::NumericEntityMinimum {
            entity_type: EntityType::YearsExperience,
            minimum,
            min_confidence: FIELD_FILTER_CONFIDENCE_THRESHOLD,
        });
    }
    SearchProjectionFilter::new(predicates)
        .map_err(|_| CliError::user("search filters are invalid"))
}

fn push_search_text_filter(
    predicates: &mut Vec<SearchProjectionPredicate>,
    entity_type: EntityType,
    values: &[String],
) {
    if !values.is_empty() {
        predicates.push(SearchProjectionPredicate::EntityValuesAny {
            entity_type,
            normalized_values: values.to_vec(),
            min_confidence: FIELD_FILTER_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::AsciiInsensitive,
        });
    }
}

fn push_search_school_tier_filter(
    predicates: &mut Vec<SearchProjectionPredicate>,
    tiers: &[SchoolTier],
) {
    if tiers.is_empty() {
        return;
    }
    let include_missing = tiers.contains(&SchoolTier::Unknown);
    let values = tiers
        .iter()
        .filter(|tier| **tier != SchoolTier::Unknown)
        .map(|tier| tier.canonical().to_string())
        .collect::<Vec<_>>();
    let predicate = match (values.is_empty(), include_missing) {
        (true, true) => SearchProjectionPredicate::MissingEntityType {
            entity_type: EntityType::SchoolTier,
            min_confidence: FIELD_FILTER_CONFIDENCE_THRESHOLD,
        },
        (false, true) => SearchProjectionPredicate::EntityValuesAnyOrMissing {
            entity_type: EntityType::SchoolTier,
            normalized_values: values,
            min_confidence: FIELD_FILTER_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::Exact,
        },
        (false, false) => SearchProjectionPredicate::EntityValuesAny {
            entity_type: EntityType::SchoolTier,
            normalized_values: values,
            min_confidence: FIELD_FILTER_CONFIDENCE_THRESHOLD,
            case: SearchFilterCase::Exact,
        },
        (true, false) => return,
    };
    predicates.push(predicate);
}

fn degree_filter_values(min_degree: DegreeLevel) -> Vec<String> {
    [
        DegreeLevel::HighSchool,
        DegreeLevel::Associate,
        DegreeLevel::Bachelor,
        DegreeLevel::Master,
        DegreeLevel::Doctor,
    ]
    .into_iter()
    .filter(|degree| *degree >= min_degree)
    .map(|degree| degree.canonical().to_string())
    .collect()
}

fn print_search_hits(hits: Vec<SearchOutputHit>) {
    println!("results: {}", hits.len());
    for hit in hits {
        println!("rank: {}", hit.rank);
        println!("doc_id: {}", hit.selection.document_id);
        println!("version_id: {}", hit.selection.resume_version_id);
        println!("visible_epoch: {}", hit.selection.visible_epoch);
        println!("file_name: {}", redact_search_file_name(&hit.file_name));
        println!("snippet: {}", hit.snippet);
    }
}

fn detail_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let detail_args = parse_detail_args(args)?;
    if detail_args.ipc_auto {
        let status_endpoint = discover_status_ipc_endpoint(data_dir)?;
        let endpoint = discover_detail_ipc_endpoint(data_dir)?;
        ensure_auto_ipc_same_daemon(status_endpoint.addr, endpoint.addr)?;
        verify_auto_ipc_status(&status_endpoint)?;
        let token_file = auto_ipc_token_file(data_dir);
        return detail_ipc_command_with_token_file(&endpoint, &token_file, &detail_args);
    }
    if let Some(endpoint) = &detail_args.ipc_endpoint {
        return detail_ipc_command(endpoint, &detail_args);
    }

    let selection = detail_selection(&detail_args)?;
    let store = open_store(data_dir)?;
    let detail = build_resume_detail(&store, &selection)?;
    print_resume_detail(&detail);
    Ok(())
}

fn detail_selection(args: &DetailArgs) -> Result<SearchSelection> {
    Ok(SearchSelection {
        document_id: DocumentId::from_str(&args.doc_id)
            .map_err(|_| CliError::user("detail doc id is invalid"))?,
        resume_version_id: ResumeVersionId::from_str(&args.version_id)
            .map_err(|_| CliError::user("detail version id is invalid"))?,
        visible_epoch: args.visible_epoch,
    })
}

fn detail_ipc_command(endpoint: &IpcDetailEndpoint, detail_args: &DetailArgs) -> Result<()> {
    let token_file = detail_args
        .ipc_token_file
        .as_ref()
        .ok_or_else(|| CliError::usage(detail_usage()))?;
    detail_ipc_command_with_token_file(endpoint, token_file, detail_args)
}

fn detail_ipc_command_with_token_file(
    endpoint: &IpcDetailEndpoint,
    token_file: &Path,
    detail_args: &DetailArgs,
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .map_err(|_| CliError::user("unable to read daemon detail ipc token"))?;
    let token = validate_daemon_ipc_token(&token, "daemon detail ipc token is invalid")?;
    let request_id = new_search_ipc_request_id()?.replacen("cli-search", "cli-detail", 1);
    let selection = detail_selection(detail_args)?;
    let body = serde_json::json!({
        "schema_version": "resume-ir.detail-request.v3",
        "request_id": request_id,
        "selection": search_selection_json(&selection),
    })
    .to_string();

    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon detail ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon detail ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon detail ipc"))?;
    let request = format!(
        "POST /details HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.addr,
        token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon detail ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon detail ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon detail ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200 ") && !status_line.starts_with("HTTP/1.0 200 ") {
        return Err(CliError::user("daemon detail ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon detail ipc returned invalid json"))?;
    render_detail_ipc_result(&body, &request_id, &selection)?;
    Ok(())
}

fn render_detail_ipc_result(
    body: &serde_json::Value,
    expected_request_id: &str,
    expected_selection: &SearchSelection,
) -> Result<()> {
    if json_str(body, "schema_version") != Some(DETAIL_SCHEMA_VERSION)
        || json_str(body, "status") != Some("ok")
        || json_str(body, "request_id") != Some(expected_request_id)
    {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }
    let selection = body
        .get("selection")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    validate_search_selection_json(selection, expected_selection)?;
    let document = body
        .get("document")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let source_byte_size = document
        .get("source_byte_size")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let parse_version = json_str(document, "parse_version")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let schema_version = json_str(document, "schema_version")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let language_set = document
        .get("language_set")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?
        .iter()
        .map(|language| {
            language
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))
        })
        .collect::<Result<Vec<_>>>()?;
    let page_count = document
        .get("page_count")
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))
        })
        .transpose()?;
    let quality_score = document
        .get("quality_score")
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_f64()
                .map(|value| value as f32)
                .filter(|value| value.is_finite())
                .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))
        })
        .transpose()?;
    let snippet = json_str(document, "snippet")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let fields = document
        .get("fields")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let field_limit = detail_ipc_count(document, "field_limit")?;
    let field_count_total = detail_ipc_count(document, "field_count_total")?;
    let field_count_returned = detail_ipc_count(document, "field_count_returned")?;
    let fields_truncated = document
        .get("fields_truncated")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if field_limit != DETAIL_FIELD_LIMIT
        || fields.len() > DETAIL_FIELD_LIMIT
        || field_count_returned != fields.len()
        || field_count_total < field_count_returned
        || fields_truncated != (field_count_returned < field_count_total)
    {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }

    let fields = fields
        .iter()
        .map(parse_detail_ipc_field)
        .collect::<Result<Vec<_>>>()?;
    let detail = ResumeDetail {
        selection: expected_selection.clone(),
        source_byte_size,
        parse_version: parse_version.to_string(),
        schema_version: schema_version.to_string(),
        language_set,
        page_count,
        quality_score,
        field_count_total,
        field_count_returned,
        fields_truncated,
        fields,
        snippet: redact_short_text(snippet, 240),
    };
    print_resume_detail(&detail);
    Ok(())
}

fn detail_ipc_count(document: &serde_json::Value, key: &str) -> Result<usize> {
    document
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))
}

fn parse_detail_ipc_field(value: &serde_json::Value) -> Result<ResumeDetailField> {
    let field_type = json_str(value, "type")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if !is_valid_detail_field_type_label(field_type) {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }
    let field_value = json_str(value, "value")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let evidence = json_str(value, "evidence")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let extractor = json_str(value, "extractor")
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    let confidence = value
        .get("confidence")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| CliError::user("daemon detail ipc returned invalid protocol"))?;
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(CliError::user(
            "daemon detail ipc returned invalid protocol",
        ));
    }

    Ok(ResumeDetailField {
        field_type: field_type.to_string(),
        value: redact_short_text(field_value, 120),
        confidence,
        evidence: redact_short_text(evidence, 120),
        extractor: redact_short_text(extractor, 80),
    })
}

fn is_valid_detail_field_type_label(value: &str) -> bool {
    matches!(
        value,
        "name"
            | "email"
            | "phone"
            | "wechat"
            | "school"
            | "degree"
            | "major"
            | "company"
            | "title"
            | "education"
            | "skills"
            | "skill"
            | "certificate"
            | "date"
            | "date_range"
            | "years_experience"
            | "location"
            | "other"
    )
}

fn build_resume_detail(store: &ReadMetaStore, selection: &SearchSelection) -> Result<ResumeDetail> {
    let request = SearchTextBytePageRequest::new(selection.clone(), 0, 240)
        .map_err(|_| CliError::user("detail selection is invalid"))?;
    let bundle = match store.search_selection_detail(&request) {
        Ok(SearchSelectionDetailResolution::Current(bundle)) => bundle,
        Ok(SearchSelectionDetailResolution::Stale) => {
            return Err(CliError::user("detail selection is stale; refresh search"));
        }
        Ok(SearchSelectionDetailResolution::NotFound) => {
            return Err(CliError::user("detail selection was not found"));
        }
        Ok(SearchSelectionDetailResolution::InvalidOffset) => {
            return Err(CliError::user("detail selection is invalid"));
        }
        Ok(SearchSelectionDetailResolution::LimitExceeded(_)) => {
            return Err(CliError::user("detail response exceeds its limit"));
        }
        Err(_) => return Err(CliError::user("detail metadata is unavailable")),
    };
    let field_count_total = bundle.details.mentions.len();
    let fields = bundle
        .details
        .mentions
        .iter()
        .take(DETAIL_FIELD_LIMIT)
        .map(resume_detail_field_from_mention)
        .collect::<Vec<_>>();
    let field_count_returned = fields.len();
    Ok(ResumeDetail {
        selection: bundle.details.selection.clone(),
        source_byte_size: bundle.details.version.source_byte_size,
        parse_version: bundle.details.version.parse_version.clone(),
        schema_version: bundle.details.version.schema_version.clone(),
        language_set: bundle.details.version.language_set.clone(),
        page_count: bundle.details.version.page_count,
        quality_score: bundle.details.version.quality_score,
        field_count_total,
        field_count_returned,
        fields_truncated: field_count_returned < field_count_total,
        fields,
        snippet: redact_short_text(&bundle.text_page.text, 240),
    })
}

fn resume_detail_field_from_mention(mention: &EntityMention) -> ResumeDetailField {
    let value = mention
        .normalized_value
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(&mention.raw_value);
    ResumeDetailField {
        field_type: entity_type_label(&mention.entity_type),
        value: redact_short_text(value, 120),
        confidence: f64::from(mention.confidence.clamp(0.0, 1.0)),
        evidence: redact_short_text(&mention.raw_value, 120),
        extractor: redact_short_text(&mention.extractor, 80),
    }
}

fn print_resume_detail(detail: &ResumeDetail) {
    println!("resume detail");
    println!("doc_id: {}", detail.selection.document_id);
    println!("version_id: {}", detail.selection.resume_version_id);
    println!("visible_epoch: {}", detail.selection.visible_epoch);
    println!("source_byte_size: {}", detail.source_byte_size);
    println!("parse_version: {}", detail.parse_version);
    println!("schema_version: {}", detail.schema_version);
    println!("languages: {}", detail.language_set.join(","));
    println!(
        "page_count: {}",
        detail
            .page_count
            .map_or_else(|| "none".to_string(), |value| value.to_string())
    );
    println!(
        "quality_score: {}",
        detail
            .quality_score
            .map_or_else(|| "none".to_string(), |value| format!("{value:.3}"))
    );
    println!(
        "fields: {}/{}",
        detail.field_count_returned, detail.field_count_total
    );
    println!("fields truncated: {}", detail.fields_truncated);
    for field in &detail.fields {
        println!(
            "field: {} | value: {} | confidence: {:.2} | evidence: {} | extractor: {}",
            field.field_type, field.value, field.confidence, field.evidence, field.extractor
        );
    }
    println!("snippet: {}", detail.snippet);
}

fn redact_short_text(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = redact_contact_values(&compact);
    truncate_chars(&redacted, max_chars)
}

fn redact_search_file_name(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let redacted = redact_contact_values(&compact);
    truncate_utf8_bytes(&redacted, SEARCH_RESULT_FILE_NAME_MAX_BYTES)
}

fn truncate_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }

    const ELLIPSIS: &str = "...";
    let mut end = max_bytes.saturating_sub(ELLIPSIS.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], ELLIPSIS)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index == max_chars {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    output
}

fn delete_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let delete_args = parse_delete_args(args)?;
    if delete_args.ipc_auto {
        let status_endpoint = discover_status_ipc_endpoint(data_dir)?;
        let endpoint = discover_delete_ipc_endpoint(data_dir)?;
        ensure_auto_ipc_same_daemon(status_endpoint.addr, endpoint.addr)?;
        verify_auto_ipc_status(&status_endpoint)?;
        let token_file = auto_ipc_token_file(data_dir);
        return delete_ipc_command_with_token_file(&endpoint, &token_file, &delete_args);
    }
    if let Some(endpoint) = &delete_args.ipc_endpoint {
        return delete_ipc_command(endpoint, &delete_args);
    }

    let data_directory_owner = import_processing::acquire_owner_for_mutation(
        data_dir,
        import_processing::OfflineImportProcessingMutation::DirectDelete,
    )?;

    let document_id = DocumentId::from_str(&delete_args.doc_id)
        .map_err(|_| CliError::user("delete doc id is invalid"))?;
    let store = open_owned_store(&data_directory_owner)?;
    let now = current_timestamp()?;
    let Some(document) = store
        .document_by_id(&document_id)
        .map_err(CliError::store)?
    else {
        return Err(CliError::user("delete document was not found"));
    };
    if document.is_deleted || document.status == DocumentStatus::Deleted {
        return Err(CliError::user("delete document was not found"));
    }
    let publication = publish_search_projection_removals(
        &store,
        &[SearchProjectionRemoval {
            document_id: document_id.clone(),
            reason: SearchProjectionRemovalReason::ConfirmedSourceDeletion,
        }],
        now,
        &SearchPublicationVectorization::default(),
    )
    .map_err(CliError::import)?;

    println!("delete completed");
    println!("doc_id: {document_id}");
    println!("status: deleted");
    println!("publication committed: true");
    println!("indexed documents: {}", publication.active_projection_count);

    Ok(())
}

struct DeleteArgs {
    doc_id: String,
    ipc_auto: bool,
    ipc_endpoint: Option<IpcDeleteEndpoint>,
    ipc_token_file: Option<PathBuf>,
}

fn parse_delete_args(args: &[String]) -> Result<DeleteArgs> {
    let mut doc_id = None;
    let mut ipc_auto = false;
    let mut ipc_endpoint = None;
    let mut ipc_token_file = None;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--doc-id" => {
                if doc_id.is_some() {
                    return Err(CliError::usage(delete_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(delete_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(delete_usage()));
                }
                doc_id = Some(value.clone());
                index += 2;
            }
            "--ipc" => {
                if ipc_auto || ipc_endpoint.is_some() {
                    return Err(CliError::usage(delete_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(delete_usage()));
                };
                if value == "auto" {
                    ipc_auto = true;
                } else {
                    ipc_endpoint = Some(parse_delete_ipc_endpoint(value)?);
                }
                index += 2;
            }
            "--ipc-token-file" => {
                if ipc_token_file.is_some() {
                    return Err(CliError::usage(delete_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(delete_usage()));
                };
                ipc_token_file = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err(CliError::usage(delete_usage())),
        }
    }

    if ipc_auto && ipc_token_file.is_some() {
        return Err(CliError::usage(delete_usage()));
    }
    if !ipc_auto && ipc_endpoint.is_some() != ipc_token_file.is_some() {
        return Err(CliError::usage(delete_usage()));
    }

    Ok(DeleteArgs {
        doc_id: doc_id.ok_or_else(|| CliError::usage(delete_usage()))?,
        ipc_auto,
        ipc_endpoint,
        ipc_token_file,
    })
}

fn delete_usage() -> &'static str {
    "usage: resume-cli delete --doc-id <doc_id> [--ipc auto|<http://127.0.0.1:port/delete|/status> --ipc-token-file <path>]"
}

fn parse_delete_ipc_endpoint(value: &str) -> Result<IpcDeleteEndpoint> {
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| CliError::usage(delete_usage()))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| CliError::usage(delete_usage()))?;
    if path != "delete" && path != "status" {
        return Err(CliError::usage(delete_usage()));
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| CliError::usage(delete_usage()))?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage("delete ipc endpoint must be loopback"));
    }

    Ok(IpcDeleteEndpoint { addr })
}

fn delete_ipc_command(endpoint: &IpcDeleteEndpoint, delete_args: &DeleteArgs) -> Result<()> {
    let token_file = delete_args
        .ipc_token_file
        .as_ref()
        .ok_or_else(|| CliError::usage(delete_usage()))?;
    delete_ipc_command_with_token_file(endpoint, token_file, delete_args)
}

fn delete_ipc_command_with_token_file(
    endpoint: &IpcDeleteEndpoint,
    token_file: &Path,
    delete_args: &DeleteArgs,
) -> Result<()> {
    let token = fs::read_to_string(token_file)
        .map_err(|_| CliError::user("unable to read daemon delete ipc token"))?;
    let token = validate_daemon_ipc_token(&token, "daemon delete ipc token is invalid")?;
    let body = serde_json::json!({
        "doc_id": delete_args.doc_id.as_str(),
    })
    .to_string();

    let mut stream = TcpStream::connect_timeout(&endpoint.addr, Duration::from_secs(2))
        .map_err(|_| CliError::user("unable to connect to daemon delete ipc"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon delete ipc"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| CliError::user("unable to configure daemon delete ipc"))?;
    let request = format!(
        "POST /delete HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.addr,
        token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| CliError::user("unable to request daemon delete ipc"))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|_| CliError::user("unable to read daemon delete ipc"))?;
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .ok_or_else(|| CliError::user("daemon delete ipc response is invalid"))?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200 ") && !status_line.starts_with("HTTP/1.0 200 ") {
        return Err(CliError::user("daemon delete ipc returned an error"));
    }

    let body: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| CliError::user("daemon delete ipc returned invalid json"))?;
    render_delete_ipc_result(&body, delete_args.doc_id.as_str())?;
    Ok(())
}

fn render_delete_ipc_result(body: &serde_json::Value, expected_doc_id: &str) -> Result<()> {
    if json_str(body, "schema_version") != Some("resume-ir.delete-response.v2")
        || json_str(body, "status") != Some("ok")
    {
        return Err(CliError::user(
            "daemon delete ipc returned invalid protocol",
        ));
    }
    let doc_id = json_str(body, "doc_id")
        .ok_or_else(|| CliError::user("daemon delete ipc returned invalid protocol"))?;
    if doc_id != expected_doc_id || DocumentId::from_str(doc_id).is_err() {
        return Err(CliError::user(
            "daemon delete ipc returned invalid protocol",
        ));
    }
    let publication_committed = body
        .get("publication_committed")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| CliError::user("daemon delete ipc returned invalid protocol"))?;
    if !publication_committed {
        return Err(CliError::user(
            "daemon delete ipc returned invalid protocol",
        ));
    }
    let indexed_documents = body
        .get("indexed_documents")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CliError::user("daemon delete ipc returned invalid protocol"))?;

    println!("delete completed");
    println!("doc_id: {doc_id}");
    println!("status: deleted");
    println!("publication committed: true");
    println!("indexed documents: {indexed_documents}");
    Ok(())
}

fn purge_command(data_dir: &Path, args: &[String]) -> Result<()> {
    if args != ["--deleted"] {
        return Err(CliError::usage(purge_usage()));
    }

    let data_directory_owner = import_processing::acquire_owner_for_mutation(
        data_dir,
        import_processing::OfflineImportProcessingMutation::PurgeDeleted,
    )?;

    let store = open_owned_store(&data_directory_owner)?;
    let deleted_document_ids = store
        .deleted_document_ids()
        .map_err(|_| CliError::user("purge could not enumerate deleted documents"))?;
    let deleted_doc_id_set = deleted_document_ids
        .iter()
        .map(|document_id| document_id.to_string())
        .collect::<BTreeSet<_>>();
    let mut deleted_content_hashes = BTreeSet::new();
    for document_id in &deleted_document_ids {
        if let Some(document) = store
            .document_by_id(document_id)
            .map_err(|_| CliError::user("purge could not inspect a deleted document"))?
        {
            if let Some(content_hash) = document.content_hash {
                deleted_content_hashes.insert(content_hash);
            }
        }
    }
    let live_content_hashes = store
        .visible_documents()
        .map_err(|_| CliError::user("purge could not inspect active documents"))?
        .into_iter()
        .filter_map(|document| document.content_hash)
        .collect::<BTreeSet<_>>();
    deleted_content_hashes.retain(|content_hash| !live_content_hashes.contains(content_hash));
    let ocr_cache_hashes = deleted_content_hashes.into_iter().collect::<Vec<_>>();
    let residual_probe =
        PurgeResidualProbe::collect(&store, &deleted_document_ids, &ocr_cache_hashes)?;

    let import_task_purge = store
        .purge_import_tasks_for_deleted_documents(&deleted_document_ids)
        .map_err(|_| CliError::user("purge could not remove deleted import state"))?;
    let ingest_job_purge = store
        .purge_ingest_jobs_for_documents(&deleted_document_ids)
        .map_err(|_| CliError::user("purge could not remove deleted ingest state"))?;
    let ocr_cache_purge = store
        .purge_ocr_page_cache_by_content_hashes(&ocr_cache_hashes)
        .map_err(|_| CliError::user("purge could not remove deleted OCR state"))?;
    let now = current_timestamp()?;
    let rebuild = if deleted_document_ids.is_empty() {
        None
    } else {
        Some(
            rebuild_search_artifacts(&store, now, &SearchPublicationVectorization::default())
                .map_err(CliError::import)?,
        )
    };
    let snapshot_purge =
        reconcile_search_artifacts(&store, now, &SearchPublicationVectorization::default())
            .map_err(CliError::import)?;
    let vector_documents_purged = deleted_doc_id_set.len();
    let purged_documents = store
        .purge_deleted_documents()
        .map_err(|_| CliError::user("purge could not compact deleted metadata"))?;
    let residual_scan = residual_probe.scan_data_dir(&data_directory_owner)?;
    if residual_scan.retained_markers > 0 {
        return Err(CliError::user(
            "purge residual scan detected retained deleted material",
        ));
    }

    println!("purge completed");
    println!("scope: deleted");
    println!("purged documents: {}", purged_documents.deleted_documents);
    println!(
        "remaining purge tombstones: {}",
        purged_documents.remaining_tombstones
    );
    println!("index rebuilt: {}", rebuild.is_some());
    println!(
        "indexed documents: {}",
        rebuild
            .as_ref()
            .map(|summary| summary.active_projection_count)
            .unwrap_or(0)
    );
    println!(
        "full-text snapshots purged: {}",
        snapshot_purge.fulltext_generations_removed
    );
    println!(
        "full-text staging purged: {}",
        snapshot_purge.fulltext_staging_directories_removed
    );
    println!(
        "vector generations purged: {}",
        snapshot_purge.vector_generations_removed
    );
    println!(
        "vector staging purged: {}",
        snapshot_purge.vector_staging_directories_removed
    );
    println!("vector documents purged: {vector_documents_purged}");
    println!("purged import tasks: {}", import_task_purge.tasks());
    println!(
        "purged import scan scopes: {}",
        import_task_purge.scan_scopes()
    );
    println!(
        "purged import scan errors: {}",
        import_task_purge.scan_errors()
    );
    println!(
        "purged import cancellations: {}",
        import_task_purge.cancellations()
    );
    println!("ingest jobs purged: {}", ingest_job_purge.jobs());
    println!(
        "embedding job specs purged: {}",
        ingest_job_purge.embedding_specs()
    );
    println!("ocr cache entries purged: {}", ocr_cache_purge.entries());
    println!("ocr word boxes purged: {}", ocr_cache_purge.word_boxes());
    println!("residual scan: clear");
    println!(
        "residual markers checked: {}",
        residual_scan.markers_checked
    );
    println!("residual files scanned: {}", residual_scan.files_scanned);
    println!("residual bytes scanned: {}", residual_scan.bytes_scanned);
    println!("metadata vacuum: yes");
    println!("physical purge scope: local best-effort, not forensic erase");

    Ok(())
}

fn purge_usage() -> &'static str {
    "usage: resume-cli purge --deleted"
}

fn task_control_command(data_dir: &Path, args: &[String], paused: bool) -> Result<()> {
    let task = parse_worker_task_control_args(args)?;
    let owner = import_processing::acquire_owner_for_mutation(
        data_dir,
        import_processing::OfflineImportProcessingMutation::TaskControl,
    )?;
    let store = open_owned_store(&owner)?;
    let now = current_timestamp()?;
    store
        .set_worker_task_paused(task, paused, now)
        .map_err(CliError::store)?;

    println!("task: {}", worker_task_label(task));
    println!("status: {}", worker_task_status_label(paused));

    Ok(())
}

fn cancel_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let cancel_args = parse_cancel_import_args(args)?;
    if cancel_args.ipc_auto {
        let status_endpoint = discover_status_ipc_endpoint(data_dir)?;
        let endpoint = discover_import_cancel_ipc_endpoint(data_dir)?;
        ensure_auto_ipc_same_daemon(status_endpoint.addr, endpoint.addr)?;
        verify_auto_ipc_status(&status_endpoint)?;
        let token_file = auto_ipc_token_file(data_dir);
        return cancel_import_ipc_command_with_token_file(
            &endpoint,
            &token_file,
            &cancel_args.task_id,
        );
    }
    if let Some(endpoint) = &cancel_args.ipc_endpoint {
        let token_file = cancel_args
            .ipc_token_file
            .as_ref()
            .ok_or_else(cancel_usage)?;
        return cancel_import_ipc_command_with_token_file(
            endpoint,
            token_file,
            &cancel_args.task_id,
        );
    }

    let data_directory_owner = import_processing::acquire_owner_for_mutation(
        data_dir,
        import_processing::OfflineImportProcessingMutation::DirectCancel,
    )?;

    let store = open_owned_store(&data_directory_owner)?;
    let now = current_timestamp()?;
    store
        .cancel_import_task(&cancel_args.task_id, now)
        .map_err(CliError::store)?;

    println!("import task cancelled");
    println!("task id: {}", cancel_args.task_id);
    println!("status: cancelled");

    Ok(())
}

struct CancelImportArgs {
    task_id: ImportTaskId,
    ipc_auto: bool,
    ipc_endpoint: Option<IpcImportCancelEndpoint>,
    ipc_token_file: Option<PathBuf>,
}

fn parse_cancel_import_args(args: &[String]) -> Result<CancelImportArgs> {
    if args.first().map(String::as_str) != Some("import") {
        return Err(cancel_usage());
    }

    let mut task_id = None;
    let mut ipc_auto = false;
    let mut ipc_endpoint = None;
    let mut ipc_token_file = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--task-id" => {
                if task_id.is_some() {
                    return Err(cancel_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(cancel_usage());
                };
                task_id = Some(ImportTaskId::from_str(value).map_err(|_| cancel_usage())?);
            }
            "--ipc" => {
                if ipc_auto || ipc_endpoint.is_some() {
                    return Err(cancel_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(cancel_usage());
                };
                if value == "auto" {
                    ipc_auto = true;
                } else {
                    ipc_endpoint = Some(parse_import_cancel_ipc_endpoint(value)?);
                }
            }
            "--ipc-token-file" => {
                if ipc_token_file.is_some() {
                    return Err(cancel_usage());
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(cancel_usage());
                };
                ipc_token_file = Some(PathBuf::from(value));
            }
            _ => return Err(cancel_usage()),
        }
        index += 1;
    }
    if ipc_auto && ipc_token_file.is_some() {
        return Err(cancel_usage());
    }
    if !ipc_auto && ipc_endpoint.is_some() != ipc_token_file.is_some() {
        return Err(cancel_usage());
    }

    Ok(CancelImportArgs {
        task_id: task_id.ok_or_else(cancel_usage)?,
        ipc_auto,
        ipc_endpoint,
        ipc_token_file,
    })
}

fn cancel_usage_text() -> &'static str {
    "usage: resume-cli cancel import [--ipc auto|<http://127.0.0.1:port/imports/cancel|/status> --ipc-token-file <path>] --task-id <id>"
}

fn cancel_usage() -> CliError {
    CliError::usage(cancel_usage_text())
}

fn parse_worker_task_control_args(args: &[String]) -> Result<WorkerTaskKind> {
    if args.len() != 2 || args.first().map(String::as_str) != Some("--task") {
        return Err(task_control_usage());
    }

    parse_worker_task_kind(&args[1])
}

fn parse_worker_task_kind(value: &str) -> Result<WorkerTaskKind> {
    match value {
        "ocr" => Ok(WorkerTaskKind::Ocr),
        _ => Err(task_control_usage()),
    }
}

fn worker_task_label(task: WorkerTaskKind) -> &'static str {
    match task {
        WorkerTaskKind::Ocr => "ocr",
    }
}

fn worker_task_status_label(paused: bool) -> &'static str {
    if paused {
        "paused"
    } else {
        "running"
    }
}

fn task_control_usage_text() -> &'static str {
    "usage: resume-cli pause --task ocr OR resume --task ocr"
}

fn task_control_usage() -> CliError {
    CliError::usage(task_control_usage_text())
}

fn ocr_worker_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let worker_args = parse_ocr_worker_args(args)?;
    let owner = import_processing::acquire_owner_for_mutation(
        data_dir,
        import_processing::OfflineImportProcessingMutation::OcrWorker,
    )?;
    let store = open_owned_store(&owner)?;
    let now = current_timestamp()?;
    match ocr_preclaim_decision(&store).map_err(CliError::import)? {
        OcrPreclaimDecision::Ready => {}
        OcrPreclaimDecision::NotReady(_) => {
            println!("ocr worker: not ready");
            println!("documents processed: 0");
            println!("cache writes: 0");
            println!("cache hits: 0");
            return Ok(());
        }
    }
    if store
        .worker_task_control(WorkerTaskKind::Ocr)
        .map_err(CliError::store)?
        .paused
    {
        println!("ocr worker: paused");
        println!("documents processed: 0");
        println!("cache writes: 0");
        println!("cache hits: 0");
        return Ok(());
    }

    if worker_args.command.is_none() && worker_args.tesseract_command.is_none() {
        return Err(CliError::user(
            "ocr worker blocked: local OCR command not configured",
        ));
    }

    let Some(job) = store.claim_next_ocr_job(now).map_err(CliError::store)? else {
        println!("ocr worker: idle");
        println!("documents processed: 0");
        println!("cache writes: 0");
        return Ok(());
    };

    let result = run_claimed_ocr_job(data_dir, &store, &job, &worker_args, now);
    match result {
        Ok(summary) => {
            println!("ocr worker: completed");
            println!("documents processed: {}", summary.documents_processed);
            println!("cache writes: {}", summary.cache_writes);
            println!("cache hits: {}", summary.cache_hits);
            Ok(())
        }
        Err(error) => {
            store
                .finish_ocr_attempt_failure(&job, OcrAttemptFailure::Retryable, now)
                .map_err(CliError::store)?;
            Err(error)
        }
    }
}

fn run_claimed_ocr_job(
    data_dir: &Path,
    store: &OwnedMetaStore,
    job: &meta_store::ClaimedOcrJob,
    worker_args: &OcrWorkerArgs,
    now: UnixTimestamp,
) -> Result<OcrWorkerSummary> {
    let Some(document) = store
        .document_by_id(&job.job.document_id)
        .map_err(CliError::store)?
    else {
        store
            .finish_ocr_attempt_failure(job, OcrAttemptFailure::Permanent, now)
            .map_err(CliError::store)?;
        return Err(CliError::user("ocr worker job document was not found"));
    };
    let content_hash = job.source_fingerprint().to_string();
    let bytes = fs::read(&document.normalized_path)
        .map_err(|_| CliError::user("ocr worker could not read document bytes"))?;
    let page_count =
        detect_ocr_page_count(&document.extension, &bytes).map_err(CliError::import)?;
    if page_count > worker_args.max_pages_per_document {
        store
            .finish_ocr_attempt_failure(
                job,
                OcrAttemptFailure::RetryableWithKind(IngestJobFailureKind::OcrPageBudgetExceeded),
                now,
            )
            .map_err(CliError::store)?;
        return Err(CliError::user(
            "ocr worker blocked: OCR page count exceeds configured limit",
        ));
    }
    let budget = OcrWorkerBudget::new(worker_args.page_timeout_ms).map_err(CliError::ocr)?;
    let cancellation = CancellationToken::new();
    let options = OcrOptions::new(worker_args.lang.as_str(), worker_args.profile.as_str())
        .map_err(CliError::ocr)?;
    let command_client = worker_args
        .command
        .clone()
        .map(|command| {
            LocalOcrCommandSpec::new(
                command,
                Vec::<String>::new(),
                worker_args.engine_profile.as_str(),
            )
            .map(LocalOcrCommandClient::new)
            .map_err(CliError::ocr)
        })
        .transpose()?;
    let tesseract_client = worker_args
        .tesseract_command
        .clone()
        .map(|tesseract_command| {
            TesseractOcrSpec::new(tesseract_command, worker_args.engine_profile.as_str())
                .map(TesseractOcrClient::new)
                .map_err(CliError::ocr)
        })
        .transpose()?;
    let renderer = worker_args
        .render_command
        .clone()
        .map(|render_command| {
            LocalPdfRenderCommandSpec::new(render_command, Vec::<String>::new())
                .map(LocalPdfRenderCommandClient::new)
                .map_err(CliError::ocr)
        })
        .transpose()?;
    let pdftoppm_renderer = worker_args
        .pdftoppm_command
        .clone()
        .map(|pdftoppm_command| {
            PdftoppmRenderSpec::new(pdftoppm_command)
                .map(PdftoppmPdfRenderer::new)
                .map_err(CliError::ocr)
        })
        .transpose()?;

    let mut page_texts = Vec::new();
    let mut confidence_sum = 0.0_f32;
    let mut confidence_count = 0_usize;
    let mut cache_writes = 0_usize;
    let mut cache_hits = 0_usize;

    for page_no in 1..=page_count {
        let cache_key = OcrPageCacheKey::new(
            content_hash.clone(),
            page_no,
            worker_args.render_dpi,
            worker_args.lang.as_str(),
            worker_args.profile.as_str(),
        )
        .map_err(CliError::store)?;

        if let Some(entry) = store
            .ocr_page_cache_entry(&cache_key)
            .map_err(CliError::store)?
            .filter(|entry| entry.status() == meta_store::OcrPageCacheStatus::Succeeded)
        {
            page_texts.push(entry.text().unwrap_or("").to_string());
            if let Some(confidence) = entry.confidence() {
                confidence_sum += confidence;
                confidence_count += 1;
            }
            cache_hits += 1;
            continue;
        }

        if command_client.is_none() {
            if let Some(tesseract_command) = worker_args.tesseract_command.as_ref() {
                match inspect_tesseract_language_availability(
                    tesseract_command,
                    worker_args.lang.as_str(),
                ) {
                    TesseractLanguageAvailability::Available => {}
                    TesseractLanguageAvailability::Missing => {
                        let entry = OcrPageCacheEntry::failed_retryable(
                            cache_key,
                            "LanguageUnavailable",
                            now,
                        )
                        .map_err(CliError::store)?;
                        store
                            .upsert_ocr_page_cache_entry(&entry)
                            .map_err(CliError::store)?;
                        return Err(CliError::user(
                            "ocr worker blocked: requested OCR language pack is unavailable",
                        ));
                    }
                    TesseractLanguageAvailability::Unknown => {
                        let entry = OcrPageCacheEntry::failed_retryable(
                            cache_key,
                            "WorkerUnavailable",
                            now,
                        )
                        .map_err(CliError::store)?;
                        store
                            .upsert_ocr_page_cache_entry(&entry)
                            .map_err(CliError::store)?;
                        return Err(CliError::user(
                            "ocr worker blocked: local OCR command failed or unavailable",
                        ));
                    }
                }
            }
        }

        let rendered_page = if let Some(renderer) = &renderer {
            match renderer.render_page(
                &bytes,
                page_no,
                worker_args.render_dpi,
                budget,
                &cancellation,
            ) {
                Ok(rendered_page) => rendered_page,
                Err(error) => {
                    let entry = OcrPageCacheEntry::failed_retryable(
                        cache_key,
                        format!("{:?}", error.kind()),
                        now,
                    )
                    .map_err(CliError::store)?;
                    store
                        .upsert_ocr_page_cache_entry(&entry)
                        .map_err(CliError::store)?;
                    return Err(CliError::user(
                        "ocr worker blocked: local OCR command failed or unavailable",
                    ));
                }
            }
        } else if let Some(renderer) = &pdftoppm_renderer {
            match renderer.render_page(
                &bytes,
                page_no,
                worker_args.render_dpi,
                budget,
                &cancellation,
            ) {
                Ok(rendered_page) => rendered_page,
                Err(error) => {
                    let entry = OcrPageCacheEntry::failed_retryable(
                        cache_key,
                        format!("{:?}", error.kind()),
                        now,
                    )
                    .map_err(CliError::store)?;
                    store
                        .upsert_ocr_page_cache_entry(&entry)
                        .map_err(CliError::store)?;
                    return Err(CliError::user(
                        "ocr worker blocked: local OCR command failed or unavailable",
                    ));
                }
            }
        } else {
            RenderedPage::new(page_no, worker_args.render_dpi, bytes.clone())
                .map_err(CliError::ocr)?
        };
        let request = OcrPageRequest::new(rendered_page, options.clone()).map_err(CliError::ocr)?;

        let page_result = if let Some(client) = &command_client {
            client.recognize_page(request, budget, &cancellation)
        } else if let Some(client) = &tesseract_client {
            client.recognize_page(request, budget, &cancellation)
        } else {
            return Err(CliError::user(
                "ocr worker blocked: local OCR command not configured",
            ));
        };
        let page = match page_result {
            Ok(page) => page,
            Err(error) => {
                let entry = OcrPageCacheEntry::failed_retryable(
                    cache_key,
                    format!("{:?}", error.kind()),
                    now,
                )
                .map_err(CliError::store)?;
                store
                    .upsert_ocr_page_cache_entry(&entry)
                    .map_err(CliError::store)?;
                return Err(CliError::user(
                    "ocr worker blocked: local OCR command failed or unavailable",
                ));
            }
        };
        let word_boxes = ocr_word_boxes_for_cache(&page)?;
        let entry = OcrPageCacheEntry::succeeded_with_word_boxes(
            cache_key,
            page.text(),
            page.confidence(),
            page.engine_profile(),
            page.duration_ms(),
            word_boxes,
            now,
        )
        .map_err(CliError::store)?;
        store
            .upsert_ocr_page_cache_entry(&entry)
            .map_err(CliError::store)?;
        page_texts.push(page.text().to_string());
        confidence_sum += page.confidence();
        confidence_count += 1;
        cache_writes += 1;
    }

    let combined_text = page_texts.join("\n");
    let confidence = (confidence_count > 0).then_some(confidence_sum / confidence_count as f32);
    let outcome = index_claimed_ocr_text(
        data_dir,
        store,
        job,
        &combined_text,
        confidence,
        Some(page_count),
        now,
        &SearchPublicationVectorization::default(),
    )
    .map_err(CliError::import)?;
    Ok(OcrWorkerSummary {
        documents_processed: usize::from(matches!(
            outcome,
            import_pipeline::OcrTextIndexOutcome::Committed(_)
        )),
        cache_writes,
        cache_hits,
    })
}

fn ocr_word_boxes_for_cache(page: &ocr_client::OcrPage) -> Result<Vec<meta_store::OcrWordBox>> {
    page.word_boxes()
        .iter()
        .map(|word_box| {
            meta_store::OcrWordBox::new(
                word_box.text(),
                word_box.left(),
                word_box.top(),
                word_box.width(),
                word_box.height(),
                word_box.confidence(),
            )
            .map_err(CliError::store)
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OcrWorkerSummary {
    documents_processed: usize,
    cache_writes: usize,
    cache_hits: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OcrWorkerArgs {
    command: Option<PathBuf>,
    tesseract_command: Option<PathBuf>,
    render_command: Option<PathBuf>,
    pdftoppm_command: Option<PathBuf>,
    engine_profile: String,
    lang: String,
    profile: String,
    render_dpi: u32,
    page_timeout_ms: u64,
    max_pages_per_document: u32,
}

fn parse_ocr_worker_args(args: &[String]) -> Result<OcrWorkerArgs> {
    let mut seen_once = false;
    let mut command = None;
    let mut tesseract_command = None;
    let mut render_command = None;
    let mut pdftoppm_command = None;
    let mut engine_profile = "local-command".to_string();
    let mut lang = "eng".to_string();
    let mut profile = "balanced".to_string();
    let mut render_dpi = 300_u32;
    let mut page_timeout_ms = 30_000_u64;
    let mut max_pages_per_document = DEFAULT_OCR_MAX_PAGES_PER_DOCUMENT;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--once" => {
                if seen_once {
                    return Err(ocr_worker_usage());
                }
                seen_once = true;
                index += 1;
            }
            "--command" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if command.is_some() {
                    return Err(ocr_worker_usage());
                }
                command = Some(PathBuf::from(value));
                index += 1;
            }
            "--tesseract-command" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if tesseract_command.is_some() {
                    return Err(ocr_worker_usage());
                }
                tesseract_command = Some(PathBuf::from(value));
                index += 1;
            }
            "--render-command" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if render_command.is_some() {
                    return Err(ocr_worker_usage());
                }
                render_command = Some(PathBuf::from(value));
                index += 1;
            }
            "--pdftoppm-command" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if pdftoppm_command.is_some() {
                    return Err(ocr_worker_usage());
                }
                pdftoppm_command = Some(PathBuf::from(value));
                index += 1;
            }
            "--engine-profile" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if value.trim().is_empty() {
                    return Err(ocr_worker_usage());
                }
                engine_profile = value.clone();
                index += 1;
            }
            "--lang" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if value.trim().is_empty() {
                    return Err(ocr_worker_usage());
                }
                lang = value.clone();
                index += 1;
            }
            "--profile" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                if value.trim().is_empty() {
                    return Err(ocr_worker_usage());
                }
                profile = value.clone();
                index += 1;
            }
            "--render-dpi" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                render_dpi = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(ocr_worker_usage)?;
                index += 1;
            }
            "--page-timeout-ms" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                page_timeout_ms = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(ocr_worker_usage)?;
                index += 1;
            }
            "--max-pages-per-document" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(ocr_worker_usage());
                };
                max_pages_per_document = value
                    .parse::<u32>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(ocr_worker_usage)?;
                index += 1;
            }
            _ => return Err(ocr_worker_usage()),
        }
    }

    if !seen_once {
        return Err(ocr_worker_usage());
    }
    if command.is_some() && tesseract_command.is_some() {
        return Err(ocr_worker_usage());
    }
    if render_command.is_some() && pdftoppm_command.is_some() {
        return Err(ocr_worker_usage());
    }

    Ok(OcrWorkerArgs {
        command,
        tesseract_command,
        render_command,
        pdftoppm_command,
        engine_profile,
        lang,
        profile,
        render_dpi,
        page_timeout_ms,
        max_pages_per_document,
    })
}

fn ocr_worker_usage_text() -> &'static str {
    "usage: resume-cli ocr-worker --once [--command <path>|--tesseract-command <path>] [--render-command <path>|--pdftoppm-command <path>] [--engine-profile <name>] [--lang <lang>] [--profile <profile>] [--render-dpi <dpi>] [--page-timeout-ms <ms>] [--max-pages-per-document <n>]"
}

fn ocr_worker_usage() -> CliError {
    CliError::usage(ocr_worker_usage_text())
}

fn doctor_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let diagnostic_args = parse_doctor_args(args)?;
    if let Some(root) = diagnostic_args
        .post_recovery_retained_lineage_convergence_boundary_root
        .as_deref()
    {
        return print_post_recovery_retained_lineage_convergence_boundary_report(data_dir, root);
    }
    if let Some(root) = diagnostic_args
        .post_pending_import_task_recovery_boundary_root
        .as_deref()
    {
        return print_post_pending_import_task_recovery_boundary_report(data_dir, root);
    }
    if let Some(root) = diagnostic_args.pending_import_task_boundary_root.as_deref() {
        return print_pending_import_task_boundary_report(data_dir, root);
    }
    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(CliError::store)?;
    let scan_error_breakdown = store
        .import_scan_error_breakdown()
        .map_err(CliError::store)?;
    let index_diagnostic = inspect_search_index(data_dir, &store);
    let vector_diagnostic = inspect_vector_index(data_dir);
    let contact_key = inspect_contact_hash_key(data_dir).map_err(CliError::privacy)?;
    let resource_telemetry = collect_resource_telemetry(data_dir);
    let ocr_runtime = inspect_ocr_runtime(&diagnostic_args.ocr_lang);
    let metadata_encryption = store.metadata_encryption_state();

    println!("resume-ir doctor");
    println!("metadata: ok");
    println!("metadata encryption: {}", metadata_encryption.label());
    println!("ocr cache encryption: {}", metadata_encryption.label());
    println!(
        "metadata encryption remediation: {}",
        metadata_encryption_remediation(metadata_encryption)
    );
    println!("indexed documents: {}", summary.indexed_documents);
    println!("searchable documents: {}", summary.searchable_documents);
    println!("ocr queue: {}", summary.ocr_queue_depth);
    println!("ocr jobs queued: {}", summary.ocr_jobs_queued);
    println!(
        "ocr page budget blocked: {}",
        summary.ocr_page_budget_blocked
    );
    if summary.ocr_page_budget_blocked > 0 {
        println!("ocr remediation: {}", OCR_PAGE_BUDGET_REMEDIATION);
    }
    println!(
        "ocr language unavailable: {}",
        summary.ocr_language_unavailable
    );
    if summary.ocr_language_unavailable > 0 {
        println!("ocr language remediation: {}", OCR_LANGUAGE_REMEDIATION);
    }
    println!("entity mentions: {}", summary.entity_mentions);
    println!("import scan scopes: {}", summary.import_scan_scopes);
    println!("import scan errors: {}", summary.import_scan_errors);
    print_import_scan_error_breakdown(&scan_error_breakdown);
    print_query_latency_summary(&summary.query_latency);
    println!("recovery queue: {}", summary.recovery_queue_depth);
    println!("index health: {}", index_health_label(summary.index_health));
    println!(
        "last snapshot: {}",
        if summary.last_snapshot_id.is_some() {
            "present"
        } else {
            "none"
        }
    );
    println!("search index: {}", index_diagnostic.index_label());
    println!("vector index: {}", vector_diagnostic.index_label());
    println!("vector index vectors: {}", vector_diagnostic.vector_count());
    println!(
        "vector index tombstones: {}",
        vector_diagnostic.deleted_count()
    );
    println!(
        "search index read target: {}",
        index_diagnostic.read_target_label()
    );
    println!("query smoke: {}", index_diagnostic.query_smoke_label());
    println!(
        "snapshot fallback: {}",
        index_diagnostic.snapshot_fallback_label()
    );
    println!(
        "staging orphans: {}",
        index_diagnostic
            .staging_orphans()
            .map_or_else(|| "not_inspected".to_string(), |count| count.to_string())
    );
    println!("contact hash key: {}", contact_key.state().label());
    println!("resource telemetry: {}", resource_telemetry.status_label());
    println!(
        "data disk total bytes: {}",
        resource_telemetry.format_disk_total()
    );
    println!(
        "data disk available bytes: {}",
        resource_telemetry.format_disk_available()
    );
    println!(
        "process memory bytes: {}",
        resource_telemetry.format_process_memory()
    );
    println!("cpu cores: {}", resource_telemetry.cpu_cores);
    println!("ocr renderer pdftoppm: {}", ocr_runtime.pdftoppm.label());
    println!("ocr engine tesseract: {}", ocr_runtime.tesseract.label());
    println!(
        "ocr language {}: {}",
        ocr_runtime.requested_language,
        ocr_runtime.requested_language_status.label()
    );
    println!("fault simulations: available");
    println!(
        "fault simulation hooks: daemon_restart,daemon_kill,index_snapshot_corrupt,disk_space_low,permission_denied,file_lock,metadata_migration,model_checksum,ocr_crash,battery_mode,external_drive_disconnect"
    );
    println!("diagnostics redaction: available");

    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingImportTaskBoundary {
    PendingImportTaskQueryFailure,
    PendingImportTaskRowMaterializationFailure,
    UnexpectedSuccessThenPostPendingTaskBoundary,
}

impl PendingImportTaskBoundary {
    fn label(self) -> &'static str {
        match self {
            Self::PendingImportTaskQueryFailure => "pending_import_task_query_failure",
            Self::PendingImportTaskRowMaterializationFailure => {
                "pending_import_task_row_materialization_failure"
            }
            Self::UnexpectedSuccessThenPostPendingTaskBoundary => {
                "unexpected_success_then_post_pending_task_boundary"
            }
        }
    }
}

fn diagnose_pending_import_task_boundary(
    data_dir: &Path,
    requested_root: &Path,
) -> Result<PendingImportTaskBoundary> {
    let roots = canonical_import_roots(&[requested_root.to_path_buf()])?;
    let root = roots
        .into_iter()
        .next()
        .ok_or_else(|| CliError::user("import root must exist and be a directory"))?;
    let store = open_store(data_dir)?;
    let canonical_root_path = path_string(&root.canonical);
    if let Err(boundary) = store.diagnose_pending_import_task_by_root(&canonical_root_path) {
        return Ok(map_pending_import_task_boundary(boundary));
    }
    let requested_root_path = path_string(&root.requested);
    if requested_root_path != canonical_root_path {
        if let Err(boundary) = store.diagnose_pending_import_task_by_root(&requested_root_path) {
            return Ok(map_pending_import_task_boundary(boundary));
        }
    }
    Ok(PendingImportTaskBoundary::UnexpectedSuccessThenPostPendingTaskBoundary)
}

fn map_pending_import_task_boundary(
    boundary: PendingImportTaskByRootDiagnostic,
) -> PendingImportTaskBoundary {
    match boundary {
        PendingImportTaskByRootDiagnostic::QueryFailure => {
            PendingImportTaskBoundary::PendingImportTaskQueryFailure
        }
        PendingImportTaskByRootDiagnostic::RowMaterializationFailure => {
            PendingImportTaskBoundary::PendingImportTaskRowMaterializationFailure
        }
    }
}

fn print_pending_import_task_boundary_report(data_dir: &Path, requested_root: &Path) -> Result<()> {
    let boundary = diagnose_pending_import_task_boundary(data_dir, requested_root)?;
    println!("resume-ir doctor");
    println!("diagnostic scope: pending_import_task_boundary");
    println!("pending import task boundary: {}", boundary.label());
    println!("paths: <redacted>");
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostPendingImportTaskRecoveryBoundary {
    StaleRunningTaskLockBound,
    StaleRunningTaskStatusUpdateFailure,
    StaleRunningTaskRowRefreshFailure,
    StaleRunningTaskRecoveredBeforePostBoundary,
    UnexpectedSuccessAfterPostBoundary,
}

impl PostPendingImportTaskRecoveryBoundary {
    fn label(self) -> &'static str {
        match self {
            Self::StaleRunningTaskLockBound => "stale_running_task_lock_bound",
            Self::StaleRunningTaskStatusUpdateFailure => "stale_running_task_status_update_failure",
            Self::StaleRunningTaskRowRefreshFailure => "stale_running_task_row_refresh_failure",
            Self::StaleRunningTaskRecoveredBeforePostBoundary => {
                "stale_running_task_recovered_before_post_boundary"
            }
            Self::UnexpectedSuccessAfterPostBoundary => {
                "unexpected_success_then_post_pending_import_task_recovery_boundary"
            }
        }
    }
}

fn diagnose_post_pending_import_task_recovery_boundary(
    data_dir: &Path,
    store: &OwnedMetaStore,
    requested_root: &Path,
) -> Result<PostPendingImportTaskRecoveryBoundary> {
    let roots = canonical_import_roots(&[requested_root.to_path_buf()])?;
    let root = roots
        .into_iter()
        .next()
        .ok_or_else(|| CliError::user("import root must exist and be a directory"))?;
    let now = current_timestamp()?;
    let canonical_root_path = path_string(&root.canonical);
    if let Some(task) = store
        .pending_import_task_by_root(&canonical_root_path)
        .map_err(CliError::store)?
    {
        return diagnose_post_pending_import_task_recovery_boundary_for_task(
            data_dir, store, task, now,
        );
    }
    let requested_root_path = path_string(&root.requested);
    if requested_root_path != canonical_root_path {
        if let Some(task) = store
            .pending_import_task_by_root(&requested_root_path)
            .map_err(CliError::store)?
        {
            return diagnose_post_pending_import_task_recovery_boundary_for_task(
                data_dir, store, task, now,
            );
        }
    }
    Ok(PostPendingImportTaskRecoveryBoundary::UnexpectedSuccessAfterPostBoundary)
}

fn diagnose_post_pending_import_task_recovery_boundary_for_task(
    data_dir: &Path,
    store: &OwnedMetaStore,
    task: ImportTask,
    now: UnixTimestamp,
) -> Result<PostPendingImportTaskRecoveryBoundary> {
    if task.status != ImportTaskStatus::Running {
        return Ok(PostPendingImportTaskRecoveryBoundary::UnexpectedSuccessAfterPostBoundary);
    }

    let owner_lock = ImportTaskOwnerLock::try_acquire(data_dir, &task.id)
        .ok()
        .flatten();
    let Some(_owner_lock) = owner_lock else {
        return Ok(PostPendingImportTaskRecoveryBoundary::StaleRunningTaskLockBound);
    };

    if store
        .update_import_task_status(&task.id, ImportTaskStatus::FailedRetryable, now)
        .is_err()
    {
        return Ok(PostPendingImportTaskRecoveryBoundary::StaleRunningTaskStatusUpdateFailure);
    }
    let Some(_) = store.import_task_by_id(&task.id).map_err(CliError::store)? else {
        return Ok(PostPendingImportTaskRecoveryBoundary::StaleRunningTaskRowRefreshFailure);
    };
    Ok(PostPendingImportTaskRecoveryBoundary::StaleRunningTaskRecoveredBeforePostBoundary)
}

fn print_post_pending_import_task_recovery_boundary_report(
    data_dir: &Path,
    requested_root: &Path,
) -> Result<()> {
    let data_directory_owner = import_processing::acquire_owner_for_mutation(
        data_dir,
        import_processing::OfflineImportProcessingMutation::DoctorRecovery,
    )?;
    let store = open_owned_store(&data_directory_owner)?;
    let boundary =
        diagnose_post_pending_import_task_recovery_boundary(data_dir, &store, requested_root)?;
    println!("resume-ir doctor");
    println!("diagnostic scope: post_pending_import_task_recovery_boundary");
    println!(
        "post pending import task recovery boundary: {}",
        boundary.label()
    );
    println!("paths: <redacted>");
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostRecoveryRetainedLineageConvergenceBoundary {
    RetainedLineageStillRecoverableAfterReentry,
    RetainedLineageRunningWithoutVisibleProgressYet,
    RetainedLineageConvergedToVisibleProgress,
    RetainedLineageConvergedPastPendingTaskBoundary,
    RetainedLineageTerminalFailedPermanent,
    UnexpectedSuccessAfterConvergenceBoundary,
}

impl PostRecoveryRetainedLineageConvergenceBoundary {
    fn label(self) -> &'static str {
        match self {
            Self::RetainedLineageStillRecoverableAfterReentry => {
                "retained_lineage_still_recoverable_after_reentry"
            }
            Self::RetainedLineageRunningWithoutVisibleProgressYet => {
                "retained_lineage_running_without_visible_progress_yet"
            }
            Self::RetainedLineageConvergedToVisibleProgress => {
                "retained_lineage_converged_to_visible_progress"
            }
            Self::RetainedLineageConvergedPastPendingTaskBoundary => {
                "retained_lineage_converged_past_pending_task_boundary"
            }
            Self::RetainedLineageTerminalFailedPermanent => {
                "retained_lineage_terminal_failed_permanent"
            }
            Self::UnexpectedSuccessAfterConvergenceBoundary => {
                "unexpected_success_then_post_recovery_retained_lineage_convergence_boundary"
            }
        }
    }
}

fn diagnose_post_recovery_retained_lineage_convergence_boundary(
    data_dir: &Path,
    requested_root: &Path,
) -> Result<PostRecoveryRetainedLineageConvergenceBoundary> {
    let roots = canonical_import_roots(&[requested_root.to_path_buf()])?;
    let root = roots
        .into_iter()
        .next()
        .ok_or_else(|| CliError::user("import root must exist and be a directory"))?;
    let store = open_store(data_dir)?;
    let Some(task) = latest_import_task_for_requested_root(&store, &root)? else {
        return Ok(PostRecoveryRetainedLineageConvergenceBoundary::UnexpectedSuccessAfterConvergenceBoundary);
    };
    let scope = store
        .import_scan_scope_by_task_id(&task.id)
        .map_err(CliError::store)?;
    classify_post_recovery_retained_lineage_convergence_boundary(data_dir, &task, scope.as_ref())
}

fn latest_import_task_for_requested_root(
    store: &ReadMetaStore,
    root: &CanonicalImportRoot,
) -> Result<Option<ImportTask>> {
    let canonical_root_path = path_string(&root.canonical);
    if let Some(task) = store
        .latest_import_task_by_root(&canonical_root_path)
        .map_err(CliError::store)?
    {
        return Ok(Some(task));
    }
    let requested_root_path = path_string(&root.requested);
    if requested_root_path == canonical_root_path {
        return Ok(None);
    }
    store
        .latest_import_task_by_root(&requested_root_path)
        .map_err(CliError::store)
}

fn classify_post_recovery_retained_lineage_convergence_boundary(
    data_dir: &Path,
    task: &ImportTask,
    scope: Option<&ImportScanScope>,
) -> Result<PostRecoveryRetainedLineageConvergenceBoundary> {
    match task.status {
        ImportTaskStatus::Queued | ImportTaskStatus::FailedRetryable => Ok(
            PostRecoveryRetainedLineageConvergenceBoundary::RetainedLineageStillRecoverableAfterReentry,
        ),
        ImportTaskStatus::Running => {
            let owner_lock = ImportTaskOwnerLock::try_acquire(data_dir, &task.id)
                .map_err(|_| CliError::user("unable to inspect import task owner lock"))?;
            let Some(_owner_lock) = owner_lock else {
                return Ok(if scope_has_visible_processed_document_progress(scope) {
                    PostRecoveryRetainedLineageConvergenceBoundary::RetainedLineageConvergedToVisibleProgress
                } else {
                    PostRecoveryRetainedLineageConvergenceBoundary::RetainedLineageRunningWithoutVisibleProgressYet
                });
            };
            Ok(
                PostRecoveryRetainedLineageConvergenceBoundary::RetainedLineageStillRecoverableAfterReentry,
            )
        }
        ImportTaskStatus::Completed => Ok(
            PostRecoveryRetainedLineageConvergenceBoundary::RetainedLineageConvergedPastPendingTaskBoundary,
        ),
        ImportTaskStatus::FailedPermanent => Ok(
            PostRecoveryRetainedLineageConvergenceBoundary::RetainedLineageTerminalFailedPermanent,
        ),
    }
}

fn scope_has_visible_processed_document_progress(scope: Option<&ImportScanScope>) -> bool {
    let Some(scope) = scope else {
        return false;
    };
    scope.searchable_documents > 0
        || scope.ocr_required_documents > 0
        || scope.ocr_jobs_queued > 0
        || scope.failed_documents > 0
        || scope.deleted_documents > 0
}

fn print_post_recovery_retained_lineage_convergence_boundary_report(
    data_dir: &Path,
    requested_root: &Path,
) -> Result<()> {
    let boundary =
        diagnose_post_recovery_retained_lineage_convergence_boundary(data_dir, requested_root)?;
    println!("resume-ir doctor");
    println!("diagnostic scope: post_recovery_retained_lineage_convergence_boundary");
    println!(
        "post recovery retained lineage convergence boundary: {}",
        boundary.label()
    );
    println!("paths: <redacted>");
    Ok(())
}

fn export_diagnostics_command(data_dir: &Path, args: &[String]) -> Result<()> {
    let diagnostic_args = parse_export_diagnostics_args(args)?;

    let store = open_store(data_dir)?;
    let summary = store.status_summary().map_err(CliError::store)?;
    let scan_error_breakdown = store
        .import_scan_error_breakdown()
        .map_err(CliError::store)?;
    let index_diagnostic = inspect_search_index(data_dir, &store);
    let vector_diagnostic = inspect_vector_index(data_dir);
    let contact_key = inspect_contact_hash_key(data_dir).map_err(CliError::privacy)?;
    let resource_telemetry = collect_resource_telemetry(data_dir);
    let ocr_runtime = inspect_ocr_runtime(&diagnostic_args.ocr_lang);
    let metadata_encryption = store.metadata_encryption_state();

    println!("{{");
    println!("  \"schema_version\": \"diagnostics.v1\",");
    println!("  \"redacted\": true,");
    println!("  \"raw_paths\": \"<redacted>\",");
    println!("  \"raw_queries\": \"<redacted>\",");
    println!("  \"raw_resume_text\": \"<redacted>\",");
    println!("  \"metadata\": {{");
    println!("    \"indexed_documents\": {},", summary.indexed_documents);
    println!(
        "    \"searchable_documents\": {},",
        summary.searchable_documents
    );
    println!("    \"ocr_queue_depth\": {},", summary.ocr_queue_depth);
    println!(
        "    \"metadata_encryption\": \"{}\",",
        metadata_encryption.label()
    );
    println!(
        "    \"ocr_cache_encryption\": \"{}\",",
        metadata_encryption.label()
    );
    println!(
        "    \"metadata_encryption_remediation\": \"{}\",",
        metadata_encryption_remediation(metadata_encryption)
    );
    println!("    \"ocr_jobs_queued\": {},", summary.ocr_jobs_queued);
    println!(
        "    \"ocr_page_budget_blocked\": {},",
        summary.ocr_page_budget_blocked
    );
    println!(
        "    \"ocr_remediation\": \"{}\",",
        if summary.ocr_page_budget_blocked > 0 {
            OCR_PAGE_BUDGET_REMEDIATION
        } else {
            "none"
        }
    );
    println!(
        "    \"ocr_language_unavailable\": {},",
        summary.ocr_language_unavailable
    );
    println!(
        "    \"ocr_language_remediation\": \"{}\",",
        if summary.ocr_language_unavailable > 0 {
            OCR_LANGUAGE_REMEDIATION
        } else {
            "none"
        }
    );
    println!("    \"entity_mentions\": {},", summary.entity_mentions);
    println!(
        "    \"import_scan_scopes\": {},",
        summary.import_scan_scopes
    );
    println!(
        "    \"import_scan_errors\": {},",
        summary.import_scan_errors
    );
    println!("    \"import_scan_error_breakdown\": [");
    print_import_scan_error_breakdown_json(&scan_error_breakdown, "      ");
    println!("    ],");
    println!(
        "    \"recovery_queue_depth\": {}",
        summary.recovery_queue_depth
    );
    println!("  }},");
    println!(
        "  \"search_index_state\": \"{}\",",
        index_diagnostic.state_label()
    );
    println!(
        "  \"vector_index_state\": \"{}\",",
        vector_diagnostic.state_label()
    );
    println!(
        "  \"vector_index_backend\": \"{}\",",
        vector_diagnostic.backend_json_label()
    );
    println!(
        "  \"vector_index_vectors\": {},",
        vector_diagnostic.vector_count()
    );
    println!(
        "  \"vector_index_tombstones\": {},",
        vector_diagnostic.deleted_count()
    );
    println!(
        "  \"search_index_read_target\": \"{}\",",
        index_diagnostic.read_target_label()
    );
    println!(
        "  \"index_health\": \"{}\",",
        index_health_label(summary.index_health)
    );
    println!(
        "  \"last_snapshot\": \"{}\",",
        if summary.last_snapshot_id.is_some() {
            "present"
        } else {
            "none"
        }
    );
    println!(
        "  \"staging_orphans\": {},",
        index_diagnostic
            .staging_orphans()
            .map_or_else(|| "null".to_string(), |count| count.to_string())
    );
    println!(
        "  \"snapshot_fallback\": \"{}\",",
        index_diagnostic.snapshot_fallback_label()
    );
    println!(
        "  \"query_smoke\": \"{}\",",
        index_diagnostic.query_smoke_json_label()
    );
    println!("  \"query_latency\": {{");
    println!(
        "    \"sample_count\": {},",
        summary.query_latency.sample_count
    );
    println!(
        "    \"p50_ms\": {},",
        format_json_optional_u64(summary.query_latency.p50_ms)
    );
    println!(
        "    \"p95_ms\": {},",
        format_json_optional_u64(summary.query_latency.p95_ms)
    );
    println!(
        "    \"p99_ms\": {},",
        format_json_optional_u64(summary.query_latency.p99_ms)
    );
    println!(
        "    \"last_result_count\": {},",
        format_json_optional_u64(summary.query_latency.last_result_count)
    );
    println!("    \"raw_queries\": \"<redacted>\"");
    println!("  }},");
    println!(
        "  \"contact_hash_key\": \"{}\",",
        contact_key.state().label()
    );
    println!("  \"resource_telemetry\": {{");
    println!("    \"status\": \"{}\",", resource_telemetry.status_label());
    println!("    \"paths\": \"<redacted>\",");
    println!(
        "    \"data_disk_total_bytes\": {},",
        resource_telemetry.format_json_disk_total()
    );
    println!(
        "    \"data_disk_available_bytes\": {},",
        resource_telemetry.format_json_disk_available()
    );
    println!(
        "    \"process_memory_bytes\": {},",
        resource_telemetry.format_json_process_memory()
    );
    println!("    \"cpu_cores\": {}", resource_telemetry.cpu_cores);
    println!("  }},");
    println!("  \"ocr_runtime\": {{");
    println!("    \"paths\": \"<redacted>\",");
    println!("    \"pdftoppm\": \"{}\",", ocr_runtime.pdftoppm.label());
    println!("    \"tesseract\": \"{}\",", ocr_runtime.tesseract.label());
    println!(
        "    \"requested_language\": \"{}\",",
        ocr_runtime.requested_language
    );
    println!(
        "    \"requested_language_status\": \"{}\"",
        ocr_runtime.requested_language_status.label()
    );
    println!("  }},");
    println!("  \"fault_simulations\": [");
    println!("    \"daemon_restart\",");
    println!("    \"daemon_kill\",");
    println!("    \"index_snapshot_corrupt\",");
    println!("    \"disk_space_low\",");
    println!("    \"permission_denied\",");
    println!("    \"file_lock\",");
    println!("    \"metadata_migration\",");
    println!("    \"model_checksum\",");
    println!("    \"ocr_crash\",");
    println!("    \"battery_mode\",");
    println!("    \"external_drive_disconnect\"");
    println!("  ],");
    println!("  \"diagnostic_scope\": {{");
    println!("    \"metadata\": \"aggregate_counts\",");
    println!("    \"search_index\": \"state_and_snapshot_health\",");
    println!("    \"vector_index\": \"state_backend_and_counts\",");
    println!("    \"query_latency\": \"aggregate_observations\",");
    println!("    \"runtime_dependencies\": \"presence_only\",");
    println!("    \"fault_simulations\": \"available_cases_only\"");
    println!("  }},");
    println!("  \"evidence_level\": \"local_aggregate_only\",");
    println!("  \"scope\": \"redacted local aggregate diagnostics; no raw resume text, paths, queries, tokens, or index segment contents included\"");
    println!("}}");

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiagnosticArgs {
    ocr_lang: String,
    pending_import_task_boundary_root: Option<PathBuf>,
    post_pending_import_task_recovery_boundary_root: Option<PathBuf>,
    post_recovery_retained_lineage_convergence_boundary_root: Option<PathBuf>,
}

fn parse_doctor_args(args: &[String]) -> Result<DiagnosticArgs> {
    let mut ocr_lang = "eng".to_string();
    let mut pending_import_task_boundary = false;
    let mut post_pending_import_task_recovery_boundary = false;
    let mut post_recovery_retained_lineage_convergence_boundary = false;
    let mut root = None;
    let mut seen_ocr_lang = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--ocr-lang" => {
                if seen_ocr_lang {
                    return Err(CliError::usage(doctor_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(doctor_usage()));
                };
                ocr_lang = parse_ocr_diagnostic_language(value, doctor_usage())?;
                seen_ocr_lang = true;
                index += 2;
            }
            "--pending-import-task-boundary" => {
                if pending_import_task_boundary {
                    return Err(CliError::usage(doctor_usage()));
                }
                if post_pending_import_task_recovery_boundary {
                    return Err(CliError::usage(doctor_usage()));
                }
                if post_recovery_retained_lineage_convergence_boundary {
                    return Err(CliError::usage(doctor_usage()));
                }
                pending_import_task_boundary = true;
                index += 1;
            }
            "--post-pending-import-task-recovery-boundary" => {
                if post_pending_import_task_recovery_boundary {
                    return Err(CliError::usage(doctor_usage()));
                }
                if pending_import_task_boundary {
                    return Err(CliError::usage(doctor_usage()));
                }
                if post_recovery_retained_lineage_convergence_boundary {
                    return Err(CliError::usage(doctor_usage()));
                }
                post_pending_import_task_recovery_boundary = true;
                index += 1;
            }
            "--post-recovery-retained-lineage-convergence-boundary" => {
                if post_recovery_retained_lineage_convergence_boundary {
                    return Err(CliError::usage(doctor_usage()));
                }
                if pending_import_task_boundary || post_pending_import_task_recovery_boundary {
                    return Err(CliError::usage(doctor_usage()));
                }
                post_recovery_retained_lineage_convergence_boundary = true;
                index += 1;
            }
            "--root" => {
                if root.is_some() {
                    return Err(CliError::usage(doctor_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(doctor_usage()));
                };
                root = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err(CliError::usage(doctor_usage())),
        }
    }
    let requires_root = pending_import_task_boundary
        || post_pending_import_task_recovery_boundary
        || post_recovery_retained_lineage_convergence_boundary;
    if requires_root != root.is_some() {
        return Err(CliError::usage(doctor_usage()));
    }
    let (
        pending_import_task_boundary_root,
        post_pending_import_task_recovery_boundary_root,
        post_recovery_retained_lineage_convergence_boundary_root,
    ) = if pending_import_task_boundary {
        (root, None, None)
    } else if post_pending_import_task_recovery_boundary {
        (None, root, None)
    } else if post_recovery_retained_lineage_convergence_boundary {
        (None, None, root)
    } else {
        (None, None, None)
    };

    Ok(DiagnosticArgs {
        ocr_lang,
        pending_import_task_boundary_root,
        post_pending_import_task_recovery_boundary_root,
        post_recovery_retained_lineage_convergence_boundary_root,
    })
}

fn parse_export_diagnostics_args(args: &[String]) -> Result<DiagnosticArgs> {
    if args.first().map(String::as_str) != Some("--redact") {
        return Err(CliError::usage(export_diagnostics_usage()));
    }
    let ocr_lang = parse_diagnostic_ocr_args(&args[1..], export_diagnostics_usage())?;
    Ok(DiagnosticArgs {
        ocr_lang,
        pending_import_task_boundary_root: None,
        post_pending_import_task_recovery_boundary_root: None,
        post_recovery_retained_lineage_convergence_boundary_root: None,
    })
}

fn doctor_usage() -> &'static str {
    "usage: resume-cli doctor [--ocr-lang <lang>] [--pending-import-task-boundary --root <path>] [--post-pending-import-task-recovery-boundary --root <path>] [--post-recovery-retained-lineage-convergence-boundary --root <path>]"
}

fn export_diagnostics_usage() -> &'static str {
    "usage: resume-cli export-diagnostics --redact [--ocr-lang <lang>]"
}

fn parse_diagnostic_ocr_args(args: &[String], usage: &'static str) -> Result<String> {
    let mut ocr_lang = "eng".to_string();
    let mut seen_ocr_lang = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--ocr-lang" => {
                if seen_ocr_lang {
                    return Err(CliError::usage(usage));
                }
                index += 1;
                let Some(value) = args.get(index) else {
                    return Err(CliError::usage(usage));
                };
                ocr_lang = parse_ocr_diagnostic_language(value, usage)?;
                seen_ocr_lang = true;
                index += 1;
            }
            _ => return Err(CliError::usage(usage)),
        }
    }

    Ok(ocr_lang)
}

fn parse_ocr_diagnostic_language(value: &str, usage: &'static str) -> Result<String> {
    if !valid_ocr_diagnostic_language(value) {
        return Err(CliError::usage(usage));
    }
    Ok(value.to_string())
}

fn valid_ocr_diagnostic_language(value: &str) -> bool {
    ocr_language_components(value).is_some()
}

fn ocr_language_components(value: &str) -> Option<Vec<&str>> {
    if value.is_empty() || value.len() > 80 {
        return None;
    }

    let mut components = Vec::new();
    for component in value.split('+') {
        if component.is_empty()
            || !component.chars().all(|character| {
                character.is_ascii_alphanumeric() || character == '_' || character == '-'
            })
        {
            return None;
        }
        components.push(component);
    }

    Some(components)
}

#[derive(Debug, Clone)]
struct ResourceTelemetry {
    data_disk_total_bytes: Option<u64>,
    data_disk_available_bytes: Option<u64>,
    process_memory_bytes: Option<u64>,
    cpu_cores: usize,
}

impl ResourceTelemetry {
    fn status_label(&self) -> &'static str {
        if self.data_disk_total_bytes.is_some()
            && self.data_disk_available_bytes.is_some()
            && self.process_memory_bytes.is_some()
            && self.cpu_cores > 0
        {
            "available"
        } else {
            "degraded"
        }
    }

    fn format_disk_total(&self) -> String {
        format_optional_u64(self.data_disk_total_bytes)
    }

    fn format_disk_available(&self) -> String {
        format_optional_u64(self.data_disk_available_bytes)
    }

    fn format_process_memory(&self) -> String {
        format_optional_u64(self.process_memory_bytes)
    }

    fn format_json_disk_total(&self) -> String {
        format_json_optional_u64(self.data_disk_total_bytes)
    }

    fn format_json_disk_available(&self) -> String {
        format_json_optional_u64(self.data_disk_available_bytes)
    }

    fn format_json_process_memory(&self) -> String {
        format_json_optional_u64(self.process_memory_bytes)
    }
}

fn collect_resource_telemetry(data_dir: &Path) -> ResourceTelemetry {
    let (data_disk_total_bytes, data_disk_available_bytes) = data_disk_telemetry(data_dir)
        .map(|disk| (Some(disk.total_bytes), Some(disk.available_bytes)))
        .unwrap_or((None, None));
    let process_memory_bytes = process_memory_bytes();
    let cpu_cores = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(0);

    ResourceTelemetry {
        data_disk_total_bytes,
        data_disk_available_bytes,
        process_memory_bytes,
        cpu_cores,
    }
}

#[derive(Debug, Clone)]
struct OcrRuntimeDiagnostic {
    pdftoppm: OcrRuntimeState,
    tesseract: OcrRuntimeState,
    requested_language: String,
    requested_language_status: OcrRuntimeState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OcrRuntimeState {
    Available,
    Missing,
    Unknown,
}

impl OcrRuntimeState {
    fn label(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Missing => "missing",
            Self::Unknown => "unknown",
        }
    }
}

fn inspect_ocr_runtime(requested_language: &str) -> OcrRuntimeDiagnostic {
    inspect_ocr_runtime_with_commands(requested_language, None, None)
}

fn inspect_ocr_runtime_with_commands(
    requested_language: &str,
    pdftoppm_command: Option<&PathBuf>,
    tesseract_command: Option<&PathBuf>,
) -> OcrRuntimeDiagnostic {
    let discovered_pdftoppm;
    let pdftoppm = match pdftoppm_command {
        Some(command) => Some(command),
        None => {
            discovered_pdftoppm = find_command_in_path("pdftoppm");
            discovered_pdftoppm.as_ref()
        }
    };
    let discovered_tesseract;
    let tesseract = match tesseract_command {
        Some(command) => Some(command),
        None => {
            discovered_tesseract = find_command_in_path("tesseract");
            discovered_tesseract.as_ref()
        }
    };
    let requested_language_status = tesseract
        .map(|path| inspect_tesseract_language(path, requested_language))
        .unwrap_or(OcrRuntimeState::Missing);

    OcrRuntimeDiagnostic {
        pdftoppm: tool_state(pdftoppm),
        tesseract: tool_state(tesseract),
        requested_language: requested_language.to_string(),
        requested_language_status,
    }
}

fn tool_state(path: Option<&PathBuf>) -> OcrRuntimeState {
    if path.is_some_and(|path| is_executable_file(path)) {
        OcrRuntimeState::Available
    } else {
        OcrRuntimeState::Missing
    }
}

fn inspect_tesseract_language(command_path: &Path, language: &str) -> OcrRuntimeState {
    let Some(requested_languages) = ocr_language_components(language) else {
        return OcrRuntimeState::Missing;
    };
    let Ok(output) = Command::new(command_path).arg("--list-langs").output() else {
        return OcrRuntimeState::Unknown;
    };
    if !output.status.success() {
        return OcrRuntimeState::Unknown;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let available_languages = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .collect::<Vec<_>>();
    if requested_languages.iter().all(|language| {
        available_languages
            .iter()
            .any(|available| available == language)
    }) {
        OcrRuntimeState::Available
    } else {
        OcrRuntimeState::Missing
    }
}

fn find_command_in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join(name))
            .find(|path| is_executable_file(path))
    })
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

#[derive(Debug, Clone, Copy)]
struct DiskTelemetry {
    total_bytes: u64,
    available_bytes: u64,
}

fn data_disk_telemetry(data_dir: &Path) -> Option<DiskTelemetry> {
    let target = nearest_existing_ancestor(data_dir)?;
    let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_storage());

    disks
        .list()
        .iter()
        .filter(|disk| target.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(|disk| DiskTelemetry {
            total_bytes: disk.total_space(),
            available_bytes: disk.available_space(),
        })
}

fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    loop {
        if current.exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

fn process_memory_bytes() -> Option<u64> {
    let pid = get_current_pid().ok()?;
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_memory().without_tasks(),
    );
    system.process(pid).map(|process| process.memory())
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn format_json_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string())
}

fn parse_search_args(data_dir: &Path, args: &[String]) -> Result<SearchArgs> {
    if args.is_empty() {
        return Err(CliError::usage(search_usage()));
    };

    let mut query = None;
    let mut top_k = 10_usize;
    let mut filters = SearchFilters::default();
    let mut mode = SearchMode::FullText;
    let mut embedding_command = None;
    let mut model_id = None;
    let mut dimension = None;
    let mut vector_top_k = None;
    let mut embedding_timeout_ms = 30_000_u64;
    let mut ipc_auto = false;
    let mut ipc_endpoint = None;
    let mut ipc_token_file = None;
    let mut contact_hashes_any = Vec::new();
    let mut contact_hasher = None;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--query-file" => {
                if query.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                query = Some(read_search_query_file(Path::new(value))?);
                index += 2;
            }
            "--ipc" => {
                if ipc_auto || ipc_endpoint.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                if value == "auto" {
                    ipc_auto = true;
                } else {
                    ipc_endpoint = Some(parse_search_ipc_endpoint(value)?);
                }
                index += 2;
            }
            "--ipc-token-file" => {
                if ipc_token_file.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                ipc_token_file = Some(PathBuf::from(value));
                index += 2;
            }
            "--mode" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                mode = SearchMode::parse(value).ok_or_else(|| CliError::usage(search_usage()))?;
                index += 2;
            }
            "--embedding-command" => {
                if embedding_command.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                embedding_command = Some(PathBuf::from(value));
                index += 2;
            }
            "--model-id" => {
                if model_id.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(search_usage()));
                }
                model_id = Some(value.clone());
                index += 2;
            }
            "--dimension" => {
                if dimension.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                dimension = Some(parse_positive_usize(value)?);
                index += 2;
            }
            "--vector-top-k" => {
                if vector_top_k.is_some() {
                    return Err(CliError::usage(search_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                vector_top_k = Some(parse_positive_usize(value)?.min(1000));
                index += 2;
            }
            "--embedding-timeout-ms" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                embedding_timeout_ms = value
                    .parse::<u64>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| CliError::usage(search_usage()))?;
                index += 2;
            }
            "--degree" | "--degree-min" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let degree = DegreeLevel::parse(value)
                    .ok_or_else(|| CliError::user("search degree filter is invalid"))?;
                filters = filters.with_degree_min(degree);
                index += 2;
            }
            "--name" | "--names-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let names = value
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .collect::<Vec<_>>();
                if names.is_empty() {
                    return Err(CliError::user("search name filter is invalid"));
                }
                filters = filters.with_names_any(names);
                index += 2;
            }
            "--school-tier" | "--school-tier-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let mut school_tiers = Vec::new();
                for school_tier in value
                    .split(',')
                    .map(str::trim)
                    .filter(|school_tier| !school_tier.is_empty())
                {
                    school_tiers.push(
                        SchoolTier::parse(school_tier).ok_or_else(|| {
                            CliError::user("search school tier filter is invalid")
                        })?,
                    );
                }
                if school_tiers.is_empty() {
                    return Err(CliError::user("search school tier filter is invalid"));
                }
                filters = filters.with_school_tiers_any(school_tiers);
                index += 2;
            }
            "--school" | "--schools-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let schools = value
                    .split(',')
                    .map(str::trim)
                    .filter(|school| !school.is_empty())
                    .collect::<Vec<_>>();
                if schools.is_empty() {
                    return Err(CliError::user("search school filter is invalid"));
                }
                filters = filters.with_schools_any(schools);
                index += 2;
            }
            "--major" | "--majors-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let majors = value
                    .split(',')
                    .map(str::trim)
                    .filter(|major| !major.is_empty())
                    .collect::<Vec<_>>();
                if majors.is_empty() {
                    return Err(CliError::user("search major filter is invalid"));
                }
                filters = filters.with_majors_any(majors);
                index += 2;
            }
            "--certificate" | "--certificates-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let certificates = value
                    .split(',')
                    .map(str::trim)
                    .filter(|certificate| !certificate.is_empty())
                    .collect::<Vec<_>>();
                if certificates.is_empty() {
                    return Err(CliError::user("search certificate filter is invalid"));
                }
                filters = filters.with_certificates_any(certificates);
                index += 2;
            }
            "--date-range-overlaps" | "--date-range-overlap" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let Some(date_range) = DateRange::parse(value) else {
                    return Err(CliError::user("search date range filter is invalid"));
                };
                filters = filters.with_date_range_overlaps(&date_range.canonical());
                index += 2;
            }
            "--company" | "--companies-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let companies = value
                    .split(',')
                    .map(str::trim)
                    .filter(|company| !company.is_empty())
                    .collect::<Vec<_>>();
                if companies.is_empty() {
                    return Err(CliError::user("search company filter is invalid"));
                }
                filters = filters.with_companies_any(companies);
                index += 2;
            }
            "--title" | "--titles-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let titles = value
                    .split(',')
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .collect::<Vec<_>>();
                if titles.is_empty() {
                    return Err(CliError::user("search title filter is invalid"));
                }
                filters = filters.with_titles_any(titles);
                index += 2;
            }
            "--location" | "--locations-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let locations = value
                    .split(',')
                    .map(str::trim)
                    .filter(|location| !location.is_empty())
                    .collect::<Vec<_>>();
                if locations.is_empty() {
                    return Err(CliError::user("search location filter is invalid"));
                }
                filters = filters.with_locations_any(locations);
                index += 2;
            }
            "--skills-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                filters = filters.with_skills_any(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|skill| !skill.is_empty()),
                );
                index += 2;
            }
            "--email" | "--emails-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let values = comma_values(value);
                if values.is_empty() {
                    return Err(CliError::user("search email filter is invalid"));
                }
                for value in values {
                    let normalized = normalize_search_email(value)
                        .ok_or_else(|| CliError::user("search email filter is invalid"))?;
                    contact_hashes_any.push(hash_search_contact(
                        data_dir,
                        &mut contact_hasher,
                        ContactKind::Email,
                        &normalized,
                    )?);
                }
                index += 2;
            }
            "--phone" | "--phones-any" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let values = comma_values(value);
                if values.is_empty() {
                    return Err(CliError::user("search phone filter is invalid"));
                }
                for value in values {
                    let normalized = normalize_search_phone(value)
                        .ok_or_else(|| CliError::user("search phone filter is invalid"))?;
                    contact_hashes_any.push(hash_search_contact(
                        data_dir,
                        &mut contact_hasher,
                        ContactKind::Phone,
                        &normalized,
                    )?);
                }
                index += 2;
            }
            "--years-experience-min" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                let years = value
                    .parse::<f32>()
                    .ok()
                    .filter(|years| years.is_finite() && *years >= 0.0)
                    .ok_or_else(|| CliError::user("search years filter is invalid"))?;
                filters = filters.with_years_experience_min(years);
                index += 2;
            }
            "--top-k" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(search_usage()));
                };
                top_k = value
                    .parse::<usize>()
                    .ok()
                    .filter(|value| *value > 0)
                    .map(|value| value.min(100))
                    .ok_or_else(|| CliError::user("search top-k is invalid"))?;
                index += 2;
            }
            value if !value.starts_with("--") && query.is_none() => {
                let value = value.trim();
                if value.is_empty() {
                    return Err(CliError::usage(search_usage()));
                }
                query = Some(value.to_string());
                index += 1;
            }
            _ => return Err(CliError::usage(search_usage())),
        }
    }
    if ipc_auto && ipc_token_file.is_some() {
        return Err(CliError::usage(search_usage()));
    }
    if !ipc_auto && ipc_endpoint.is_some() != ipc_token_file.is_some() {
        return Err(CliError::usage(search_usage()));
    }
    if !contact_hashes_any.is_empty() {
        filters = filters.with_contact_hashes_any(contact_hashes_any);
    }
    let query = query.ok_or_else(|| CliError::usage(search_usage()))?;
    let query = plan_search(&query, top_k)
        .map_err(|_| CliError::user("search query is outside semantic bounds"))?
        .query_text()
        .to_string();

    Ok(SearchArgs {
        query,
        top_k,
        filters,
        mode,
        embedding_command,
        model_id,
        dimension,
        vector_top_k,
        embedding_timeout_ms,
        ipc_auto,
        ipc_endpoint,
        ipc_token_file,
    })
}

fn search_usage() -> &'static str {
    "usage: resume-cli search (<query>|--query-file <path>) [--ipc auto|<http://127.0.0.1:port/search|/status> --ipc-token-file <path>] [--mode fulltext|semantic|hybrid] [--embedding-command <path>] [--model-id <id>] [--dimension <n>] [--vector-top-k <n>] [--embedding-timeout-ms <ms>] [--degree <level>] [--name <name[,name...]>] [--names-any <name[,name...]>] [--school-tier <tier[,tier...]>] [--school <school[,school...]>] [--schools-any <school[,school...]>] [--major <major[,major...]>] [--majors-any <major[,major...]>] [--certificate <cert[,cert...]>] [--certificates-any <cert[,cert...]>] [--date-range-overlaps <YYYY-MM/YYYY-MM|YYYY-MM/PRESENT>] [--company <company[,company...]>] [--companies-any <company[,company...]>] [--title <title[,title...]>] [--titles-any <title[,title...]>] [--location <location[,location...]>] [--locations-any <location[,location...]>] [--skills-any <skill[,skill...]>] [--email <email[,email...]>] [--phone <phone[,phone...]>] [--years-experience-min <years>] [--top-k <n>]"
}

fn read_search_query_file(path: &Path) -> Result<String> {
    let query =
        fs::read_to_string(path).map_err(|_| CliError::user("search query file is unavailable"))?;
    let query = query.trim();
    if query.is_empty() {
        return Err(CliError::user("search query file is empty"));
    }
    Ok(query.to_string())
}

fn comma_values(value: &str) -> Vec<&str> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect()
}

fn hash_search_contact(
    data_dir: &Path,
    hasher: &mut Option<ContactHasher>,
    kind: ContactKind,
    normalized_value: &str,
) -> Result<String> {
    if hasher.is_none() {
        *hasher = Some(ContactHasher::load_or_create(data_dir).map_err(CliError::privacy)?);
    }
    let hasher = hasher.as_ref().expect("contact hasher initialized");
    Ok(hasher
        .hash_contact(kind, normalized_value)
        .map_err(CliError::privacy)?
        .as_str()
        .to_string())
}

fn normalize_search_email(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().any(char::is_whitespace)
        || !value.contains('@')
        || !value.rsplit_once('@')?.1.contains('.')
    {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn normalize_search_phone(value: &str) -> Option<String> {
    let raw = value.trim();
    let digits = raw
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    if raw.starts_with('+') && (11..=15).contains(&digits.len()) {
        return Some(format!("+{digits}"));
    }
    if let Some(rest) = digits.strip_prefix("0086") {
        if is_china_mobile(rest) {
            return Some(format!("+86{rest}"));
        }
    }
    if let Some(rest) = digits.strip_prefix("86") {
        if is_china_mobile(rest) {
            return Some(format!("+86{rest}"));
        }
    }
    if is_china_mobile(&digits) {
        return Some(format!("+86{digits}"));
    }
    if digits.len() == 10 {
        return Some(format!("+1{digits}"));
    }
    if digits.len() == 11 && digits.starts_with('1') {
        return Some(format!("+{digits}"));
    }
    None
}

fn is_china_mobile(digits: &str) -> bool {
    let bytes = digits.as_bytes();
    digits.len() == 11
        && bytes.first() == Some(&b'1')
        && bytes.get(1).is_some_and(|byte| matches!(byte, b'3'..=b'9'))
}

fn parse_search_ipc_endpoint(value: &str) -> Result<IpcSearchEndpoint> {
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| CliError::usage(search_usage()))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| CliError::usage(search_usage()))?;
    if path != "search" && path != "status" {
        return Err(CliError::usage(search_usage()));
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| CliError::usage(search_usage()))?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage("search ipc endpoint must be loopback"));
    }

    Ok(IpcSearchEndpoint { addr })
}

fn parse_detail_args(args: &[String]) -> Result<DetailArgs> {
    let mut doc_id = None;
    let mut version_id = None;
    let mut visible_epoch = None;
    let mut ipc_auto = false;
    let mut ipc_endpoint = None;
    let mut ipc_token_file = None;
    let mut index = 0_usize;

    while index < args.len() {
        match args[index].as_str() {
            "--doc-id" => {
                if doc_id.is_some() {
                    return Err(CliError::usage(detail_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(detail_usage()));
                };
                if value.trim().is_empty() {
                    return Err(CliError::usage(detail_usage()));
                }
                doc_id = Some(value.clone());
                index += 2;
            }
            "--version-id" => {
                if version_id.is_some() {
                    return Err(CliError::usage(detail_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(detail_usage()));
                };
                version_id = Some(value.clone());
                index += 2;
            }
            "--visible-epoch" => {
                if visible_epoch.is_some() {
                    return Err(CliError::usage(detail_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(detail_usage()));
                };
                visible_epoch = value.parse::<u64>().ok().filter(|epoch| *epoch > 0);
                if visible_epoch.is_none() {
                    return Err(CliError::usage(detail_usage()));
                }
                index += 2;
            }
            "--ipc" => {
                if ipc_auto || ipc_endpoint.is_some() {
                    return Err(CliError::usage(detail_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(detail_usage()));
                };
                if value == "auto" {
                    ipc_auto = true;
                } else {
                    ipc_endpoint = Some(parse_detail_ipc_endpoint(value)?);
                }
                index += 2;
            }
            "--ipc-token-file" => {
                if ipc_token_file.is_some() {
                    return Err(CliError::usage(detail_usage()));
                }
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(detail_usage()));
                };
                ipc_token_file = Some(PathBuf::from(value));
                index += 2;
            }
            _ => return Err(CliError::usage(detail_usage())),
        }
    }

    if ipc_auto && ipc_token_file.is_some() {
        return Err(CliError::usage(detail_usage()));
    }
    if !ipc_auto && ipc_endpoint.is_some() != ipc_token_file.is_some() {
        return Err(CliError::usage(detail_usage()));
    }

    Ok(DetailArgs {
        doc_id: doc_id.ok_or_else(|| CliError::usage(detail_usage()))?,
        version_id: version_id.ok_or_else(|| CliError::usage(detail_usage()))?,
        visible_epoch: visible_epoch.ok_or_else(|| CliError::usage(detail_usage()))?,
        ipc_auto,
        ipc_endpoint,
        ipc_token_file,
    })
}

fn detail_usage() -> &'static str {
    "usage: resume-cli detail --doc-id <doc_id> --version-id <version_id> --visible-epoch <epoch> [--ipc auto|<http://127.0.0.1:port/details|/status> --ipc-token-file <path>]"
}

fn parse_detail_ipc_endpoint(value: &str) -> Result<IpcDetailEndpoint> {
    let rest = value
        .strip_prefix("http://")
        .ok_or_else(|| CliError::usage(detail_usage()))?;
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| CliError::usage(detail_usage()))?;
    if path != "details" && path != "status" {
        return Err(CliError::usage(detail_usage()));
    }

    let addr = SocketAddr::from_str(authority).map_err(|_| CliError::usage(detail_usage()))?;
    if !addr.ip().is_loopback() {
        return Err(CliError::usage("detail ipc endpoint must be loopback"));
    }

    Ok(IpcDetailEndpoint { addr })
}

#[derive(Clone)]
struct SearchOutputHit {
    rank: usize,
    selection: SearchSelection,
    file_name: String,
    snippet: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchMode {
    FullText,
    Semantic,
    Hybrid,
}

impl SearchMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "fulltext" | "keyword" => Some(Self::FullText),
            "semantic" => Some(Self::Semantic),
            "hybrid" => Some(Self::Hybrid),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::FullText => "fulltext",
            Self::Semantic => "semantic",
            Self::Hybrid => "hybrid",
        }
    }

    fn benchmark_layers_label(self) -> &'static str {
        match self {
            Self::FullText => "fulltext",
            Self::Semantic => "vector",
            Self::Hybrid => "fulltext+field+vector+rrf",
        }
    }

    fn benchmark_query_embedding_runtime_label(self) -> &'static str {
        match self {
            Self::FullText => "none",
            Self::Semantic | Self::Hybrid => "local-command",
        }
    }

    fn benchmark_query_embedding_invocations(self) -> usize {
        match self {
            Self::FullText => 0,
            Self::Semantic | Self::Hybrid => 1,
        }
    }
}

#[derive(Clone)]
struct SearchArgs {
    query: String,
    top_k: usize,
    filters: SearchFilters,
    mode: SearchMode,
    embedding_command: Option<PathBuf>,
    model_id: Option<String>,
    dimension: Option<usize>,
    vector_top_k: Option<usize>,
    embedding_timeout_ms: u64,
    ipc_auto: bool,
    ipc_endpoint: Option<IpcSearchEndpoint>,
    ipc_token_file: Option<PathBuf>,
}

struct DetailArgs {
    doc_id: String,
    version_id: String,
    visible_epoch: u64,
    ipc_auto: bool,
    ipc_endpoint: Option<IpcDetailEndpoint>,
    ipc_token_file: Option<PathBuf>,
}

struct ResumeDetail {
    selection: SearchSelection,
    source_byte_size: u64,
    parse_version: String,
    schema_version: String,
    language_set: Vec<String>,
    page_count: Option<u32>,
    quality_score: Option<f32>,
    field_count_total: usize,
    field_count_returned: usize,
    fields_truncated: bool,
    fields: Vec<ResumeDetailField>,
    snippet: String,
}

struct ResumeDetailField {
    field_type: String,
    value: String,
    confidence: f64,
    evidence: String,
    extractor: String,
}

fn inspect_search_index(data_dir: &Path, store: &ReadMetaStore) -> SearchIndexDiagnostic {
    let staging_orphans = None;
    let Ok(state) = store.search_projection_state() else {
        return SearchIndexDiagnostic::Corrupt { staging_orphans };
    };
    if state.service_state != meta_store::SearchProjectionServiceState::Ready {
        return SearchIndexDiagnostic::Unavailable { staging_orphans };
    }
    let Ok(mut coordinator) = QueryCoordinator::open(data_dir) else {
        return SearchIndexDiagnostic::Corrupt { staging_orphans };
    };
    let started_at = Instant::now();
    match coordinator
        .with_query(|scope| scope.fulltext_candidates("diagnostic", HitLimit::new(1)?, None))
    {
        Ok(hits) => SearchIndexDiagnostic::Available {
            elapsed_ms: started_at.elapsed().as_millis(),
            results: hits.len(),
            staging_orphans,
        },
        Err(_) => SearchIndexDiagnostic::Corrupt { staging_orphans },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchIndexDiagnostic {
    Unavailable {
        staging_orphans: Option<usize>,
    },
    Corrupt {
        staging_orphans: Option<usize>,
    },
    Available {
        elapsed_ms: u128,
        results: usize,
        staging_orphans: Option<usize>,
    },
}

impl SearchIndexDiagnostic {
    fn index_label(self) -> String {
        match self {
            Self::Unavailable { .. } => "unavailable".to_string(),
            Self::Corrupt { .. } => "corrupt".to_string(),
            Self::Available { .. } => "available (database Ready full-text snapshot)".to_string(),
        }
    }

    fn state_label(self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "unavailable",
            Self::Corrupt { .. } => "corrupt",
            Self::Available { .. } => "available",
        }
    }

    fn read_target_label(self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "none",
            Self::Corrupt { .. } | Self::Available { .. } => "database_ready_generation",
        }
    }

    fn snapshot_fallback_label(self) -> &'static str {
        "none"
    }

    fn staging_orphans(self) -> Option<usize> {
        match self {
            Self::Unavailable { staging_orphans }
            | Self::Corrupt {
                staging_orphans, ..
            }
            | Self::Available {
                staging_orphans, ..
            } => staging_orphans,
        }
    }

    fn query_smoke_label(self) -> String {
        match self {
            Self::Unavailable { .. } => "skipped (no full-text index)".to_string(),
            Self::Corrupt { .. } => "skipped (index unavailable)".to_string(),
            Self::Available {
                elapsed_ms,
                results,
                ..
            } => {
                format!("ok (elapsed_ms={elapsed_ms}, results={results})")
            }
        }
    }

    fn query_smoke_json_label(self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "skipped_no_fulltext_index",
            Self::Corrupt { .. } => "skipped_index_unavailable",
            Self::Available { .. } => "ok",
        }
    }
}

fn inspect_vector_index(data_dir: &Path) -> VectorIndexDiagnostic {
    let Ok(store) = ReadMetaStore::open_data_dir(data_dir) else {
        return VectorIndexDiagnostic::unavailable();
    };
    let Ok(state) = store.search_projection_state() else {
        return VectorIndexDiagnostic::corrupt();
    };
    let Some(vector) = state
        .publication
        .as_deref()
        .and_then(|publication| publication.vector.as_ref())
    else {
        return VectorIndexDiagnostic::unavailable();
    };
    VectorIndexDiagnostic {
        state: "available",
        backend: if matches!(vector.mode(), VectorSnapshotMode::Enabled { .. }) {
            "hnsw_ann"
        } else {
            "none"
        },
        vector_count: vector.vector_count(),
        deleted_count: 0,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct VectorIndexDiagnostic {
    state: &'static str,
    backend: &'static str,
    vector_count: u64,
    deleted_count: u64,
}

impl VectorIndexDiagnostic {
    fn unavailable() -> Self {
        Self {
            state: "unavailable",
            backend: "none",
            vector_count: 0,
            deleted_count: 0,
        }
    }

    fn corrupt() -> Self {
        Self {
            state: "corrupt",
            ..Self::unavailable()
        }
    }

    fn index_label(self) -> &'static str {
        match (self.state, self.backend) {
            ("available", "hnsw_ann") => "available (hnsw ann vector snapshot)",
            ("available", _) => "available (disabled vector snapshot)",
            (state, _) => state,
        }
    }

    fn state_label(self) -> &'static str {
        self.state
    }

    fn vector_count(self) -> u64 {
        self.vector_count
    }

    fn deleted_count(self) -> u64 {
        self.deleted_count
    }

    fn backend_json_label(self) -> &'static str {
        self.backend
    }
}

fn open_store(data_dir: &Path) -> Result<ReadMetaStore> {
    ReadMetaStore::open_data_dir(data_dir).map_err(CliError::store)
}

fn open_owned_store(owner: &import_pipeline::DataDirectoryOwnerLease) -> Result<OwnedMetaStore> {
    owner.open_store().map_err(CliError::store)
}

fn current_timestamp() -> Result<UnixTimestamp> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliError::user("system clock is before the Unix epoch"))?
        .as_secs();
    let seconds = i64::try_from(seconds).map_err(|_| CliError::user("system clock is invalid"))?;
    Ok(UnixTimestamp::from_unix_seconds(seconds))
}

fn new_import_task_id() -> Result<ImportTaskId> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliError::user("system clock is before the Unix epoch"))?;
    let nanos = duration.as_nanos().to_string();
    let pid = std::process::id().to_string();

    Ok(ImportTaskId::from_non_secret_parts(&[
        "s4-import-task",
        &nanos,
        &pid,
    ]))
}

fn index_health_label(status: IndexStateStatus) -> &'static str {
    match status {
        IndexStateStatus::Empty => "empty",
        IndexStateStatus::Building => "building",
        IndexStateStatus::Ready => "ready",
        IndexStateStatus::Stale => "stale",
    }
}

fn metadata_encryption_remediation(state: MetadataEncryptionState) -> &'static str {
    match state {
        MetadataEncryptionState::Plaintext => METADATA_ENCRYPTION_REMEDIATION,
        MetadataEncryptionState::SqlCipher => "",
    }
}

fn document_status_label(status: DocumentStatus) -> &'static str {
    match status {
        DocumentStatus::Discovered => "discovered",
        DocumentStatus::Fingerprinted => "fingerprinted",
        DocumentStatus::ParseQueued => "parse_queued",
        DocumentStatus::ParseRunning => "parse_running",
        DocumentStatus::TextExtracted => "text_extracted",
        DocumentStatus::OcrRequired => "ocr_required",
        DocumentStatus::OcrRunning => "ocr_running",
        DocumentStatus::OcrDone => "ocr_done",
        DocumentStatus::TextCleaned => "text_cleaned",
        DocumentStatus::FieldsExtracted => "fields_extracted",
        DocumentStatus::EmbeddingDone => "embedding_done",
        DocumentStatus::IndexedPartial => "indexed_partial",
        DocumentStatus::Searchable => "searchable",
        DocumentStatus::Excluded => "excluded",
        DocumentStatus::FailedRetryable => "failed_retryable",
        DocumentStatus::FailedPermanent => "failed_permanent",
        DocumentStatus::Deleted => "deleted",
    }
}

fn ingest_job_kind_label(kind: IngestJobKind) -> &'static str {
    match kind {
        IngestJobKind::DiscoverDocument => "discover_document",
        IngestJobKind::FingerprintDocument => "fingerprint_document",
        IngestJobKind::ParseDocument => "parse_document",
        IngestJobKind::OcrDocument => "ocr_document",
        IngestJobKind::CleanText => "clean_text",
        IngestJobKind::ExtractFields => "extract_fields",
        IngestJobKind::UpdateIndex => "update_index",
    }
}

fn ingest_job_status_label(status: IngestJobStatus) -> &'static str {
    match status {
        IngestJobStatus::Queued => "queued",
        IngestJobStatus::Running => "running",
        IngestJobStatus::Interrupted => "interrupted",
        IngestJobStatus::Completed => "completed",
        IngestJobStatus::FailedRetryable => "failed_retryable",
        IngestJobStatus::FailedPermanent => "failed_permanent",
    }
}

fn ingest_job_failure_kind_label(kind: IngestJobFailureKind) -> &'static str {
    match kind {
        IngestJobFailureKind::OcrPageBudgetExceeded => "ocr_page_budget_exceeded",
    }
}

fn entity_type_label(entity_type: &EntityType) -> String {
    match entity_type {
        EntityType::Name => "name".to_string(),
        EntityType::Email => "email".to_string(),
        EntityType::Phone => "phone".to_string(),
        EntityType::WeChat => "wechat".to_string(),
        EntityType::School => "school".to_string(),
        EntityType::SchoolTier => "school_tier".to_string(),
        EntityType::Degree => "degree".to_string(),
        EntityType::Major => "major".to_string(),
        EntityType::Company => "company".to_string(),
        EntityType::Title => "title".to_string(),
        EntityType::Education => "education".to_string(),
        EntityType::Skills => "skills".to_string(),
        EntityType::Skill => "skill".to_string(),
        EntityType::Certificate => "certificate".to_string(),
        EntityType::Date => "date".to_string(),
        EntityType::DateRange => "date_range".to_string(),
        EntityType::YearsExperience => "years_experience".to_string(),
        EntityType::Location => "location".to_string(),
        EntityType::Other(_) => "other".to_string(),
    }
}

type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug)]
struct CliError {
    message: String,
    exit_code: i32,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 2,
        }
    }

    fn user(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
        }
    }

    fn store(error: meta_store::MetaStoreError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn import(error: import_pipeline::ImportPipelineError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn privacy(error: privacy::PrivacyError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn ocr(error: ocr_client::OcrError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn embedding(error: embedder::EmbeddingError) -> Self {
        Self {
            message: error.to_string(),
            exit_code: 1,
        }
    }

    fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meta_store::{
        ClassificationStatus, ContentDigest, CurrentClassifierEpoch, ReasonCode,
        SearchRepairReason, SourceRevision, SourceRevisionTriage, CLASSIFIER_EPOCH,
    };

    #[test]
    fn import_parse_workers_argument_sets_direct_import_override() {
        let args = strings(&["--root", "/tmp/resumes", "--parse-workers", "2"]);

        let parsed = parse_import_args(&args).unwrap();

        assert_eq!(parsed.parse_workers.get(), 2);
        assert_eq!(parsed.profile, ScanProfile::Explicit);
        assert_eq!(parsed.max_files, None);
        assert_eq!(
            parsed.index_writer_heap_bytes,
            ImportResourcePolicy::detect().index_writer_heap_bytes
        );
    }

    #[test]
    fn import_parse_workers_argument_rejects_zero() {
        let args = strings(&["--root", "/tmp/resumes", "--parse-workers", "0"]);

        assert!(parse_import_args(&args).is_err());
    }

    #[test]
    fn launchctl_status_success_with_running_state_reports_running() {
        let status = service_runtime_state_from_launchctl_result(
            true,
            "service = com.resume-ir.daemon\nstate = running\npid = 123\n",
            "",
        );

        assert_eq!(status, ServiceRuntimeState::Running);
        assert_eq!(status.label(), "running");
    }

    #[test]
    fn launchctl_status_success_without_running_state_reports_loaded() {
        let status = service_runtime_state_from_launchctl_result(
            true,
            "service = com.resume-ir.daemon\nstate = waiting\n",
            "",
        );

        assert_eq!(status, ServiceRuntimeState::Loaded);
        assert_eq!(status.label(), "loaded");
    }

    #[test]
    fn launchctl_status_missing_service_reports_not_loaded_without_path_leak() {
        let status = service_runtime_state_from_launchctl_result(
            false,
            "",
            "Could not find service \"com.resume-ir.daemon\" in domain gui/501\n",
        );

        assert_eq!(status, ServiceRuntimeState::NotLoaded);
        assert_eq!(status.label(), "not_loaded");
    }

    #[test]
    fn launchctl_status_unexpected_error_reports_unknown_without_diagnostics() {
        let status = service_runtime_state_from_launchctl_result(
            false,
            "",
            "Input/output error while reading /Users/private/Library/LaunchAgents/service.plist\n",
        );

        assert_eq!(status, ServiceRuntimeState::Unknown);
        assert_eq!(status.label(), "unknown");
    }

    #[test]
    fn witness_ocr_repair_blocked_is_bounded_and_does_not_claim() {
        let temp_dirs = WitnessTempDirs::create().unwrap();
        let owner = import_processing::acquire_owner_for_mutation(
            &temp_dirs.data_dir,
            import_processing::OfflineImportProcessingMutation::SyntheticFaultProbe,
        )
        .unwrap();
        let store = owner.open_store().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_800_411_000);
        let digest = ContentDigest::from_bytes(b"synthetic-witness-ocr-gate");
        let document_id = DocumentId::from_non_secret_parts(&["synthetic-witness-ocr-gate"]);
        store
            .upsert_document(&Document {
                id: document_id.clone(),
                source_uri: "synthetic://witness-ocr-gate".to_string(),
                normalized_path: "/synthetic/witness-ocr-gate.pdf".to_string(),
                file_name: "synthetic-witness-ocr-gate.pdf".to_string(),
                extension: FileExtension::Pdf,
                byte_size: 32,
                mtime: now,
                content_hash: Some(digest.as_str().to_string()),
                text_hash: None,
                is_deleted: false,
                created_at: now,
                updated_at: now,
                status: DocumentStatus::OcrRequired,
            })
            .unwrap();
        let revision = SourceRevision::for_content(document_id, digest, 32);
        store.insert_source_revision(&revision).unwrap();
        store
            .insert_source_revision_triage(&SourceRevisionTriage {
                source_revision_id: revision.id.clone(),
                status: ClassificationStatus::OcrBacklog,
                triage_epoch: CLASSIFIER_EPOCH.to_string(),
                reason_codes: vec![ReasonCode::OcrRequired],
                triaged_at: now,
            })
            .unwrap();
        let queued = store
            .enqueue_ocr_job_for_source_triage(
                &revision.id,
                CurrentClassifierEpoch::parse(CLASSIFIER_EPOCH).unwrap(),
                now,
            )
            .unwrap();
        store
            .block_migration_rebuild(SearchRepairReason::RuntimeInvariant, now)
            .unwrap();

        let status = run_witness_ocr_jobs(
            &temp_dirs.data_dir,
            &store,
            &default_ocr_worker_args(),
            None,
            now,
        )
        .unwrap();

        match status {
            WitnessOcrStatus::Blocked {
                reason,
                documents_processed,
                documents_failed,
                cache_writes,
                cache_hits,
                budget_exhausted,
            } => {
                assert_eq!(reason, "search publication is not ready");
                assert_eq!(documents_processed, 0);
                assert_eq!(documents_failed, 0);
                assert_eq!(cache_writes, 0);
                assert_eq!(cache_hits, 0);
                assert!(!budget_exhausted);
            }
            _ => panic!("repair-blocked witness OCR must return the bounded blocked state"),
        }
        let job = store.ingest_job_by_id(&queued.job.id).unwrap().unwrap();
        assert_eq!(job.status, IngestJobStatus::Queued);
        assert_eq!(job.attempt_count, 0);
        assert_eq!(
            store
                .document_by_id(&job.document_id)
                .unwrap()
                .unwrap()
                .status,
            DocumentStatus::OcrRequired
        );
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }
}
