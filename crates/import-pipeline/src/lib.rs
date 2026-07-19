// Import orchestration passes stage state explicitly; split this before tightening
// these shape lints for the crate.
#![allow(clippy::too_many_arguments, clippy::large_enum_variant)]

mod classification;
mod data_directory_owner;
mod file_processing;
mod immutable_ingest;
mod import_run;
mod index_recovery;
mod migration_artifacts;
mod migration_rebuild;
mod ocr_publication;
mod processing_contract;
mod publication_coordinator;
mod purge_artifact;
mod search_artifact_cache;
mod search_artifacts;
mod search_publication;
mod search_publication_ocr;
mod search_publication_vector;
mod search_vectorizer;
mod source_digest;
mod source_dispositions;
mod timing;

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroUsize;
use std::thread;
use std::time::Duration;

pub use fs_crawler::ScanProfile;
use fs_crawler::{CrawlErrorKind, ScanBudgetKind};
use index_fulltext::SnapshotPublishPhase;
#[cfg(test)]
use index_fulltext::{IndexDocument, IndexSection};
use meta_store::{DocumentId, FileExtension};
use parser_common::{ParseInput, Parser, ParserErrorKind, ResourceBudget};
use parser_pdf::{PdfParser, PdfTextExtractionTimings};
pub use resume_classifier::LinearPromotionPolicy;
#[cfg(test)]
use sectionizer::Sectionizer;
use sysinfo::System;

pub(crate) use classification::AdmissionDecision;
pub use data_directory_owner::{
    DataDirectoryOwnerAcquireError, DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease,
    ImportProcessingOrphanNormalizationError,
};
#[cfg(test)]
use file_processing::classify_language_set;
#[cfg(test)]
use file_processing::persist_source_revision_failure;
pub(crate) use file_processing::{
    contact_hashes_from_mentions, entity_mentions_from_rules, language_set, sections_to_index,
};
#[cfg(test)]
use file_processing::{
    process_file, recv_parse_result_with_cancel_poll, ParseWorkOutcome, ParseWorkResult,
};
pub(crate) use file_processing::{PendingSearchableDocument, PendingSearchablePublicationKind};
#[cfg(test)]
use immutable_ingest::{resume_version, StagedDerivedData, StagedResume};
pub(crate) use import_run::current_timestamp_or;
#[cfg(test)]
use import_run::{
    document_path_is_deletion_candidate, finish_import_file, should_flush_searchable_documents,
    CancelCheckMetrics, ImportCancelPoller, ParseWorkerClock,
};
pub use import_run::{
    import_root, import_root_with_options, import_root_with_options_and_control, ImportRunControl,
};
pub use index_recovery::{
    finalize_migration_rebuild, reconcile_search_artifacts, SearchArtifactRecoverySummary,
};
pub use meta_store::{import_task_owner_lock_path, ImportTaskOwnerLock};
pub use migration_artifacts::{prepare_migration_rebuild_artifacts, MigrationArtifactRetirement};
pub use migration_rebuild::{ocr_preclaim_decision, OcrPreclaimDecision, OcrPreclaimNotReady};
pub use ocr_publication::{
    index_claimed_ocr_text, index_claimed_ocr_text_with_policy, OcrTextIndexOutcome,
    OcrTextIndexSummary,
};
pub use processing_contract::current_import_processing_contract;
#[cfg(test)]
use publication_coordinator::take_pending_searchable_documents;
#[cfg(test)]
use publication_coordinator::{flush_pending_searchable_documents, PendingProjectionRemovals};
pub use publication_coordinator::{publish_search_projection_removals, rebuild_search_artifacts};
pub use purge_artifact::{
    PurgeArtifactClass, PurgeArtifactClassificationError, PurgeArtifactClassifier,
};
#[cfg(test)]
use search_artifact_cache::{CurrentImportCacheMode, CurrentImportDocumentCache};
#[cfg(test)]
use search_artifacts::write_incremental_search_artifacts;
#[cfg(test)]
use search_publication::commit_prepared_search_publication;
pub use search_vectorizer::{
    SearchPublicationEmbeddingFailure, SearchPublicationEmbeddingInput,
    SearchPublicationEmbeddingOutput, SearchPublicationVectorization, SearchPublicationVectorizer,
};
#[cfg(test)]
use source_dispositions::{ImportDispositionBatches, ProcessedFile};
pub(crate) use timing::measure_result_stage;

pub(crate) const PARSE_VERSION: &str = "parser-v1";
pub(crate) const OCR_PARSE_VERSION: &str = "ocr-v1";
pub(crate) const SCHEMA_VERSION: &str = "resume-ir-s9-v2";
const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;
const MAX_IMPORT_PARSE_WORKERS: usize = 3;
const IMPORT_CANCEL_POLL_INTERVAL_MS: u64 = 25;
const PARSE_RESULT_CANCEL_POLL_INTERVAL_MS: u64 = 50;
const H0_MAX_PRIVATE_OR_ANONYMOUS_MB: u16 = 512;
const H1_MAX_PRIVATE_OR_ANONYMOUS_MB: u16 = 1024;
const H2_MAX_PRIVATE_OR_ANONYMOUS_MB: u16 = 1536;
const H0_INDEX_WRITER_HEAP_BYTES: usize = 64 * 1024 * 1024;
const H1_INDEX_WRITER_HEAP_BYTES: usize = 128 * 1024 * 1024;
const H2_INDEX_WRITER_HEAP_BYTES: usize = 256 * 1024 * 1024;
const H0_MEMORY_CEILING_BYTES: u64 = 12 * BYTES_PER_GIB;
const H1_MEMORY_CEILING_BYTES: u64 = 20 * BYTES_PER_GIB;

pub fn crate_name() -> &'static str {
    "import-pipeline"
}

pub type Result<T> = std::result::Result<T, ImportPipelineError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchProjectionRemovalReason {
    ConfirmedSourceDeletion,
    PermanentClassificationExclusion,
    PrivacyRevocation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchProjectionRemoval {
    pub document_id: DocumentId,
    pub reason: SearchProjectionRemovalReason,
}

#[derive(Clone, Debug)]
pub struct ImportOptions {
    pub scan_profile: ScanProfile,
    pub max_files: Option<usize>,
    pub parse_workers: ImportParseWorkers,
    pub index_writer_heap_bytes: usize,
    pub linear_promotion: LinearPromotionPolicy,
    pub search_vectorization: SearchPublicationVectorization,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self::for_resource_policy(ImportResourcePolicy::detect())
    }
}

impl ImportOptions {
    pub fn for_resource_policy(resource_policy: ImportResourcePolicy) -> Self {
        Self {
            scan_profile: ScanProfile::default(),
            max_files: None,
            parse_workers: resource_policy.parse_workers,
            index_writer_heap_bytes: resource_policy.index_writer_heap_bytes,
            linear_promotion: LinearPromotionPolicy::default(),
            search_vectorization: SearchPublicationVectorization::default(),
        }
    }

    pub fn for_hardware_profile(hardware_profile: ImportHardwareProfile) -> Self {
        Self::for_resource_policy(ImportResourcePolicy::for_hardware(hardware_profile))
    }

    pub fn low_memory_default_for_available_parallelism(available_parallelism: usize) -> Self {
        Self {
            scan_profile: ScanProfile::default(),
            max_files: None,
            parse_workers: ImportParseWorkers::low_memory_default_for_available_parallelism(
                available_parallelism,
            ),
            index_writer_heap_bytes: H0_INDEX_WRITER_HEAP_BYTES,
            linear_promotion: LinearPromotionPolicy::default(),
            search_vectorization: SearchPublicationVectorization::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportHardwareProfile {
    pub total_memory_bytes: Option<u64>,
    pub available_parallelism: usize,
}

impl ImportHardwareProfile {
    pub fn new(total_memory_bytes: Option<u64>, available_parallelism: usize) -> Self {
        Self {
            total_memory_bytes,
            available_parallelism: available_parallelism.max(1),
        }
    }

    pub fn detect() -> Self {
        let available_parallelism = thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(1);
        Self::new(detect_total_memory_bytes(), available_parallelism)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportHardwareTier {
    H0Eco,
    H1Balanced,
    H2Aggressive,
}

impl ImportHardwareTier {
    pub fn label(self) -> &'static str {
        match self {
            Self::H0Eco => "H0_Eco",
            Self::H1Balanced => "H1_Balanced",
            Self::H2Aggressive => "H2_Aggressive",
        }
    }

    fn default_parse_workers(self) -> usize {
        match self {
            Self::H0Eco => 1,
            Self::H1Balanced => 2,
            Self::H2Aggressive => MAX_IMPORT_PARSE_WORKERS,
        }
    }

    fn max_private_or_anonymous_mb(self) -> u16 {
        match self {
            Self::H0Eco => H0_MAX_PRIVATE_OR_ANONYMOUS_MB,
            Self::H1Balanced => H1_MAX_PRIVATE_OR_ANONYMOUS_MB,
            Self::H2Aggressive => H2_MAX_PRIVATE_OR_ANONYMOUS_MB,
        }
    }

    fn index_writer_heap_bytes(self) -> usize {
        match self {
            Self::H0Eco => H0_INDEX_WRITER_HEAP_BYTES,
            Self::H1Balanced => H1_INDEX_WRITER_HEAP_BYTES,
            Self::H2Aggressive => H2_INDEX_WRITER_HEAP_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportResourcePolicy {
    pub hardware_tier: ImportHardwareTier,
    pub parse_workers: ImportParseWorkers,
    pub index_writer_heap_bytes: usize,
    pub max_private_or_anonymous_mb: u16,
}

impl ImportResourcePolicy {
    pub fn detect() -> Self {
        Self::for_hardware(ImportHardwareProfile::detect())
    }

    pub fn for_hardware(hardware_profile: ImportHardwareProfile) -> Self {
        let hardware_tier = classify_import_hardware_tier(hardware_profile);
        let worker_limit = hardware_tier
            .default_parse_workers()
            .min(hardware_profile.available_parallelism);
        Self {
            hardware_tier,
            parse_workers: ImportParseWorkers::new(worker_limit),
            index_writer_heap_bytes: hardware_tier.index_writer_heap_bytes(),
            max_private_or_anonymous_mb: hardware_tier.max_private_or_anonymous_mb(),
        }
    }
}

fn classify_import_hardware_tier(hardware_profile: ImportHardwareProfile) -> ImportHardwareTier {
    match hardware_profile.total_memory_bytes {
        Some(memory_bytes) if memory_bytes > H1_MEMORY_CEILING_BYTES => {
            ImportHardwareTier::H2Aggressive
        }
        Some(memory_bytes) if memory_bytes > H0_MEMORY_CEILING_BYTES => {
            ImportHardwareTier::H1Balanced
        }
        _ => ImportHardwareTier::H0Eco,
    }
}

fn detect_total_memory_bytes() -> Option<u64> {
    let mut system = System::new();
    system.refresh_memory();
    let total_memory = system.total_memory();
    if total_memory == 0 {
        None
    } else {
        Some(total_memory)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportParseWorkers(NonZeroUsize);

impl ImportParseWorkers {
    pub fn new(count: usize) -> Self {
        let bounded = count.clamp(1, MAX_IMPORT_PARSE_WORKERS);
        Self(NonZeroUsize::new(bounded).expect("bounded worker count is non-zero"))
    }

    pub fn low_memory_default_for_available_parallelism(available_parallelism: usize) -> Self {
        Self::new(available_parallelism.clamp(1, MAX_IMPORT_PARSE_WORKERS))
    }

    pub fn sequential() -> Self {
        Self(NonZeroUsize::MIN)
    }

    pub fn get(self) -> usize {
        self.0.get()
    }
}

impl Default for ImportParseWorkers {
    fn default() -> Self {
        let available_parallelism = thread::available_parallelism()
            .map(NonZeroUsize::get)
            .unwrap_or(1);
        Self::low_memory_default_for_available_parallelism(available_parallelism)
    }
}

pub fn detect_ocr_page_count(extension: &FileExtension, bytes: &[u8]) -> Result<u32> {
    if !matches!(extension, FileExtension::Pdf) {
        return Ok(1);
    }

    let output = PdfParser
        .parse(
            ParseInput::from_bytes(Some("pdf"), bytes),
            ResourceBudget::default(),
        )
        .map_err(ImportPipelineError::parser)?;
    Ok(output
        .page_count()
        .and_then(|page_count| u32::try_from(page_count).ok())
        .filter(|page_count| *page_count > 0)
        .unwrap_or(1))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub files_discovered: usize,
    pub scan_errors: usize,
    pub ignored_entries: usize,
    pub content_bytes_read: u64,
    pub searchable_documents: usize,
    pub ocr_required_documents: usize,
    pub ocr_jobs_queued: usize,
    pub failed_documents: usize,
    pub failure_counts: ImportFailureCounts,
    pub deleted_documents: usize,
    pub scan_budget: Option<ImportScanBudget>,
    pub stage_timings: ImportStageTimings,
    pub milestone_timings: ImportMilestoneTimings,
    pub worker_metrics: ImportWorkerMetrics,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportStageTimings {
    pub scan: Duration,
    pub parse: Duration,
    pub db: Duration,
    pub index: Duration,
    pub ocr: Duration,
    pub embedding: Duration,
}

impl ImportStageTimings {
    pub fn add_assign(&mut self, next: &Self) {
        self.scan += next.scan;
        self.parse += next.parse;
        self.db += next.db;
        self.index += next.index;
        self.ocr += next.ocr;
        self.embedding += next.embedding;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportMilestoneTimings {
    pub first_searchable: Option<Duration>,
    pub ttf100_searchable: Option<Duration>,
    pub ttf1000_searchable: Option<Duration>,
    pub full_import_ready: Option<Duration>,
    pub full_index_ready: Option<Duration>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImportCancelCheckPhase {
    #[default]
    Unattributed,
    ImportSetup,
    Scan,
    SequentialParse,
    ParsePrepare,
    ParseQueueWait,
    ParseResultWait,
    WorkerResultCommit,
    DbWrite,
    IndexPublication,
    IndexPublicationSetup,
    IndexPublicationDocuments,
    IndexPublicationCommit,
    IndexPublicationPlaintextValidation,
    IndexPublicationEncryptedPublication,
    IndexPublicationEncryptedValidation,
    IndexPublicationAtomicPublication,
}

impl ImportCancelCheckPhase {
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Unattributed => "unattributed",
            Self::ImportSetup => "import_setup",
            Self::Scan => "scan",
            Self::SequentialParse => "sequential_parse",
            Self::ParsePrepare => "parse_prepare",
            Self::ParseQueueWait => "parse_queue_wait",
            Self::ParseResultWait => "parse_result_wait",
            Self::WorkerResultCommit => "worker_result_commit",
            Self::DbWrite => "db_write",
            Self::IndexPublication => "index_publication",
            Self::IndexPublicationSetup => "index_publication_setup",
            Self::IndexPublicationDocuments => "index_publication_documents",
            Self::IndexPublicationCommit => "index_publication_commit",
            Self::IndexPublicationPlaintextValidation => "index_publication_plaintext_validation",
            Self::IndexPublicationEncryptedPublication => "index_publication_encrypted_publication",
            Self::IndexPublicationEncryptedValidation => "index_publication_encrypted_validation",
            Self::IndexPublicationAtomicPublication => "index_publication_atomic_publication",
        }
    }

    fn from_snapshot_publish_phase(phase: SnapshotPublishPhase) -> Self {
        match phase {
            SnapshotPublishPhase::Setup => Self::IndexPublicationSetup,
            SnapshotPublishPhase::DocumentIndexing => Self::IndexPublicationDocuments,
            SnapshotPublishPhase::TantivyCommit => Self::IndexPublicationCommit,
            SnapshotPublishPhase::PlaintextValidation => Self::IndexPublicationPlaintextValidation,
            SnapshotPublishPhase::EncryptedPublication => {
                Self::IndexPublicationEncryptedPublication
            }
            SnapshotPublishPhase::EncryptedValidation => Self::IndexPublicationEncryptedValidation,
            SnapshotPublishPhase::AtomicPublication => Self::IndexPublicationAtomicPublication,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportWorkerMetrics {
    pub parse_worker_count: usize,
    pub parse_jobs_queued: usize,
    pub parse_prepare: Duration,
    pub parse_worker_wall: Duration,
    pub parse_worker_active: Duration,
    pub parse_queue_full_events: usize,
    pub parse_queue_wait: Duration,
    pub parse_result_wait: Duration,
    pub cancel_check_count: usize,
    pub cancel_check_max_gap: Duration,
    pub cancel_check_max_gap_phase: ImportCancelCheckPhase,
    pub index_publication_timings: ImportIndexPublicationTimings,
    pub pdf_parse_timings: PdfTextExtractionTimings,
    pub post_parser_timings: ImportPostParserTimings,
}

impl ImportWorkerMetrics {
    pub fn add_assign(&mut self, next: &Self) {
        self.parse_worker_count = self.parse_worker_count.max(next.parse_worker_count);
        self.parse_jobs_queued += next.parse_jobs_queued;
        self.parse_prepare += next.parse_prepare;
        self.parse_worker_wall += next.parse_worker_wall;
        self.parse_worker_active += next.parse_worker_active;
        self.parse_queue_full_events += next.parse_queue_full_events;
        self.parse_queue_wait += next.parse_queue_wait;
        self.parse_result_wait += next.parse_result_wait;
        self.cancel_check_count += next.cancel_check_count;
        if next.cancel_check_max_gap > self.cancel_check_max_gap {
            self.cancel_check_max_gap = next.cancel_check_max_gap;
            self.cancel_check_max_gap_phase = next.cancel_check_max_gap_phase;
        }
        self.index_publication_timings
            .add_assign(&next.index_publication_timings);
        self.pdf_parse_timings.add_assign(&next.pdf_parse_timings);
        self.post_parser_timings
            .add_assign(&next.post_parser_timings);
    }

    fn record_parse_worker_count(&mut self, count: usize) {
        self.parse_worker_count = self.parse_worker_count.max(count);
    }

    fn record_parse_worker_timing(&mut self, active: Duration, wall: Duration) {
        self.parse_worker_wall += wall;
        self.parse_worker_active += active;
    }

    fn record_cancel_checks(
        &mut self,
        count: usize,
        max_gap: Duration,
        max_gap_phase: ImportCancelCheckPhase,
    ) {
        self.cancel_check_count += count;
        if max_gap > self.cancel_check_max_gap {
            self.cancel_check_max_gap = max_gap;
            self.cancel_check_max_gap_phase = max_gap_phase;
        }
    }

    fn record_index_publication_phase_timing(
        &mut self,
        phase: SnapshotPublishPhase,
        elapsed: Duration,
    ) {
        self.index_publication_timings.record(phase, elapsed);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportPostParserTimings {
    pub normalization: Duration,
    pub sectionization: Duration,
}

impl ImportPostParserTimings {
    fn add_assign(&mut self, next: &Self) {
        self.normalization += next.normalization;
        self.sectionization += next.sectionization;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportIndexPublicationTimings {
    pub setup: Duration,
    pub documents: Duration,
    pub commit: Duration,
    pub plaintext_validation: Duration,
    pub encrypted_publication: Duration,
    pub encrypted_validation: Duration,
    pub atomic_publication: Duration,
}

impl ImportIndexPublicationTimings {
    fn record(&mut self, phase: SnapshotPublishPhase, elapsed: Duration) {
        match phase {
            SnapshotPublishPhase::Setup => self.setup += elapsed,
            SnapshotPublishPhase::DocumentIndexing => self.documents += elapsed,
            SnapshotPublishPhase::TantivyCommit => self.commit += elapsed,
            SnapshotPublishPhase::PlaintextValidation => self.plaintext_validation += elapsed,
            SnapshotPublishPhase::EncryptedPublication => self.encrypted_publication += elapsed,
            SnapshotPublishPhase::EncryptedValidation => self.encrypted_validation += elapsed,
            SnapshotPublishPhase::AtomicPublication => self.atomic_publication += elapsed,
        }
    }

    fn add_assign(&mut self, next: &Self) {
        self.setup += next.setup;
        self.documents += next.documents;
        self.commit += next.commit;
        self.plaintext_validation += next.plaintext_validation;
        self.encrypted_publication += next.encrypted_publication;
        self.encrypted_validation += next.encrypted_validation;
        self.atomic_publication += next.atomic_publication;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportFailureCounts {
    counts: BTreeMap<ImportFailureKind, usize>,
}

impl ImportFailureCounts {
    fn increment(&mut self, kind: ImportFailureKind) {
        *self.counts.entry(kind).or_default() += 1;
    }

    pub fn add(&mut self, kind: ImportFailureKind, count: usize) {
        *self.counts.entry(kind).or_default() += count;
    }

    pub fn get(&self, kind: ImportFailureKind) -> usize {
        self.counts.get(&kind).copied().unwrap_or(0)
    }

    pub fn entries(&self) -> impl Iterator<Item = (ImportFailureKind, usize)> + '_ {
        self.counts.iter().map(|(kind, count)| (*kind, *count))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportFailureKind {
    TextTooLarge,
    ReadError,
    UnsupportedExtension,
    ParserUnsupported,
    ParserCorrupted,
    ParserEncrypted,
    ParserTimeout,
    ParserResourceExhausted,
    ParserIo,
    ParserCancelled,
    ParserInternal,
    EmptyText,
}

impl ImportFailureKind {
    fn from_parser_error(kind: ParserErrorKind) -> Self {
        match kind {
            ParserErrorKind::Unsupported => Self::ParserUnsupported,
            ParserErrorKind::Corrupted => Self::ParserCorrupted,
            ParserErrorKind::Encrypted => Self::ParserEncrypted,
            ParserErrorKind::Timeout => Self::ParserTimeout,
            ParserErrorKind::ResourceExhausted => Self::ParserResourceExhausted,
            ParserErrorKind::Io => Self::ParserIo,
            ParserErrorKind::Cancelled => Self::ParserCancelled,
            ParserErrorKind::OcrRequired | ParserErrorKind::Internal => Self::ParserInternal,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::TextTooLarge => "text_too_large",
            Self::ReadError => "read_error",
            Self::UnsupportedExtension => "unsupported_extension",
            Self::ParserUnsupported => "parser_unsupported",
            Self::ParserCorrupted => "parser_corrupted",
            Self::ParserEncrypted => "parser_encrypted",
            Self::ParserTimeout => "parser_timeout",
            Self::ParserResourceExhausted => "parser_resource_exhausted",
            Self::ParserIo => "parser_io",
            Self::ParserCancelled => "parser_cancelled",
            Self::ParserInternal => "parser_internal",
            Self::EmptyText => "empty_text",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportScanBudget {
    pub kind: ImportScanBudgetKind,
    pub limit: usize,
    pub observed: usize,
    pub exhausted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportScanBudgetKind {
    Files,
}

impl From<fs_crawler::ScanBudgetExhausted> for ImportScanBudget {
    fn from(value: fs_crawler::ScanBudgetExhausted) -> Self {
        Self {
            kind: match value.kind {
                ScanBudgetKind::Files => ImportScanBudgetKind::Files,
            },
            limit: value.limit,
            observed: value.observed,
            exhausted: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchArtifactPublicationSummary {
    pub active_projection_count: usize,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ImportPipelineError {
    kind: ImportPipelineErrorKind,
    retryable: bool,
}

impl ImportPipelineError {
    fn store(error: meta_store::MetaStoreError) -> Self {
        let class = error.class();
        if class != meta_store::MetaStoreErrorClass::Storage {
            return Self {
                kind: ImportPipelineErrorKind::StoreInvariant(class),
                retryable: false,
            };
        }
        Self {
            kind: ImportPipelineErrorKind::Store,
            retryable: true,
        }
    }

    fn store_invariant() -> Self {
        Self {
            kind: ImportPipelineErrorKind::StoreInvariant(
                meta_store::MetaStoreErrorClass::StorageInvariant,
            ),
            retryable: false,
        }
    }

    fn crawl(error: fs_crawler::CrawlError) -> Self {
        if error.kind == CrawlErrorKind::Cancelled {
            return Self::cancelled();
        }

        Self {
            kind: ImportPipelineErrorKind::Crawl(error.kind),
            retryable: true,
        }
    }

    fn migration_scan_incomplete() -> Self {
        Self {
            kind: ImportPipelineErrorKind::Crawl(CrawlErrorKind::SourceUnavailable),
            retryable: true,
        }
    }

    fn cancelled() -> Self {
        Self {
            kind: ImportPipelineErrorKind::Cancelled,
            retryable: true,
        }
    }

    fn interrupted() -> Self {
        Self {
            kind: ImportPipelineErrorKind::Interrupted,
            retryable: true,
        }
    }

    fn repairing() -> Self {
        Self {
            kind: ImportPipelineErrorKind::Repairing,
            retryable: true,
        }
    }

    fn artifact_retirement() -> Self {
        Self {
            kind: ImportPipelineErrorKind::ArtifactRetirement,
            retryable: false,
        }
    }

    fn index(error: index_fulltext::FullTextError) -> Self {
        if matches!(error, index_fulltext::FullTextError::Cancelled) {
            return Self::cancelled();
        }

        Self {
            kind: ImportPipelineErrorKind::Index,
            retryable: !matches!(error, index_fulltext::FullTextError::Internal { .. }),
        }
    }

    fn index_io() -> Self {
        Self {
            kind: ImportPipelineErrorKind::Index,
            retryable: true,
        }
    }

    fn vector(error: index_vector::VectorIndexError) -> Self {
        let (kind, retryable) = match error {
            index_vector::VectorIndexError::InvalidDimension { .. }
            | index_vector::VectorIndexError::InvalidVectorValue
            | index_vector::VectorIndexError::InvalidModelId
            | index_vector::VectorIndexError::InvalidIdentity
            | index_vector::VectorIndexError::InvalidGeneration
            | index_vector::VectorIndexError::InvalidModelContract
            | index_vector::VectorIndexError::SemanticUnavailable
            | index_vector::VectorIndexError::PublicationProjectionMismatch
            | index_vector::VectorIndexError::DuplicateVectorId
            | index_vector::VectorIndexError::ConflictingDocumentVersion => {
                (ImportPipelineErrorKind::VectorContract, false)
            }
            index_vector::VectorIndexError::GenerationAlreadyExists
            | index_vector::VectorIndexError::GenerationNotFound
            | index_vector::VectorIndexError::LeaseRootMismatch
            | index_vector::VectorIndexError::SchemaMismatch
            | index_vector::VectorIndexError::CorruptSnapshot
            | index_vector::VectorIndexError::StorageLayoutInvalid => {
                (ImportPipelineErrorKind::VectorStorage, false)
            }
            index_vector::VectorIndexError::Storage => {
                (ImportPipelineErrorKind::VectorStorage, true)
            }
        };
        Self { kind, retryable }
    }

    fn vector_io() -> Self {
        Self {
            kind: ImportPipelineErrorKind::EmbeddingRuntime,
            retryable: true,
        }
    }

    fn privacy(_error: privacy::PrivacyError) -> Self {
        Self {
            kind: ImportPipelineErrorKind::Privacy,
            retryable: false,
        }
    }

    fn parser(_error: parser_common::ParserError) -> Self {
        Self {
            kind: ImportPipelineErrorKind::Parser,
            retryable: true,
        }
    }

    pub fn class(&self) -> ImportPipelineErrorClass {
        match self.kind {
            ImportPipelineErrorKind::Cancelled => ImportPipelineErrorClass::Cancelled,
            ImportPipelineErrorKind::Interrupted => ImportPipelineErrorClass::Interrupted,
            ImportPipelineErrorKind::Repairing => ImportPipelineErrorClass::Repairing,
            ImportPipelineErrorKind::ArtifactRetirement => {
                ImportPipelineErrorClass::ArtifactRetirement
            }
            ImportPipelineErrorKind::Store => ImportPipelineErrorClass::Metadata,
            ImportPipelineErrorKind::StoreInvariant(_) => {
                ImportPipelineErrorClass::MetadataInvariant
            }
            ImportPipelineErrorKind::Crawl(
                CrawlErrorKind::PermissionDenied
                | CrawlErrorKind::SourceUnavailable
                | CrawlErrorKind::LockedOrUnreadable,
            ) => ImportPipelineErrorClass::SourceUnavailable,
            ImportPipelineErrorKind::Crawl(CrawlErrorKind::Io) => ImportPipelineErrorClass::Scan,
            ImportPipelineErrorKind::Crawl(CrawlErrorKind::Cancelled) => {
                ImportPipelineErrorClass::Cancelled
            }
            ImportPipelineErrorKind::Index => ImportPipelineErrorClass::FullText,
            ImportPipelineErrorKind::VectorContract => ImportPipelineErrorClass::VectorContract,
            ImportPipelineErrorKind::VectorStorage => ImportPipelineErrorClass::VectorStorage,
            ImportPipelineErrorKind::EmbeddingRuntime => ImportPipelineErrorClass::EmbeddingRuntime,
            ImportPipelineErrorKind::Privacy => ImportPipelineErrorClass::Privacy,
            ImportPipelineErrorKind::Parser => ImportPipelineErrorClass::Parser,
        }
    }

    pub fn is_retryable(&self) -> bool {
        self.retryable
    }

    pub fn metadata_class_label(&self) -> Option<&'static str> {
        let ImportPipelineErrorKind::StoreInvariant(class) = self.kind else {
            return None;
        };
        Some(match class {
            meta_store::MetaStoreErrorClass::Storage => "storage",
            meta_store::MetaStoreErrorClass::Migration => "migration",
            meta_store::MetaStoreErrorClass::MigrationOwnershipRequired => {
                "migration_ownership_required"
            }
            meta_store::MetaStoreErrorClass::InvalidValue => "invalid_value",
            meta_store::MetaStoreErrorClass::NotFound => "not_found",
            meta_store::MetaStoreErrorClass::InvalidTransition => "invalid_transition",
            meta_store::MetaStoreErrorClass::ImmutableIdentityConflict => {
                "immutable_identity_conflict"
            }
            meta_store::MetaStoreErrorClass::StorageInvariant => "storage_invariant",
            meta_store::MetaStoreErrorClass::WeakPassphrase => "weak_passphrase",
            meta_store::MetaStoreErrorClass::InvalidBackup => "invalid_backup",
            meta_store::MetaStoreErrorClass::Crypto => "crypto",
            meta_store::MetaStoreErrorClass::KeyAlreadyExists => "key_already_exists",
        })
    }
}

impl fmt::Debug for ImportPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImportPipelineError")
            .field("kind", &self.kind)
            .field("retryable", &self.retryable)
            .finish()
    }
}

impl fmt::Display for ImportPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ImportPipelineErrorKind::Cancelled => formatter.write_str("import task was cancelled"),
            ImportPipelineErrorKind::Interrupted => {
                formatter.write_str("import task was interrupted by shutdown")
            }
            ImportPipelineErrorKind::Repairing => {
                formatter.write_str("search publication is repairing")
            }
            ImportPipelineErrorKind::ArtifactRetirement => {
                formatter.write_str("legacy search artifact retirement failed")
            }
            ImportPipelineErrorKind::Store => formatter.write_str("metadata update failed"),
            ImportPipelineErrorKind::StoreInvariant(_) => {
                formatter.write_str("metadata invariant failed")
            }
            ImportPipelineErrorKind::Crawl(_) => formatter.write_str("file scan failed"),
            ImportPipelineErrorKind::Index => formatter.write_str("search index update failed"),
            ImportPipelineErrorKind::VectorContract => {
                formatter.write_str("vector publication contract failed")
            }
            ImportPipelineErrorKind::VectorStorage => {
                formatter.write_str("vector index storage failed")
            }
            ImportPipelineErrorKind::EmbeddingRuntime => {
                formatter.write_str("document embedding failed")
            }
            ImportPipelineErrorKind::Privacy => {
                formatter.write_str("contact privacy boundary failed")
            }
            ImportPipelineErrorKind::Parser => formatter.write_str("document parser failed"),
        }
    }
}

impl std::error::Error for ImportPipelineError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImportPipelineErrorKind {
    Cancelled,
    Interrupted,
    Repairing,
    ArtifactRetirement,
    Store,
    StoreInvariant(meta_store::MetaStoreErrorClass),
    Crawl(CrawlErrorKind),
    Index,
    VectorContract,
    VectorStorage,
    EmbeddingRuntime,
    Privacy,
    Parser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportPipelineErrorClass {
    Cancelled,
    Interrupted,
    Repairing,
    ArtifactRetirement,
    Metadata,
    MetadataInvariant,
    SourceUnavailable,
    Scan,
    FullText,
    VectorContract,
    VectorStorage,
    EmbeddingRuntime,
    Privacy,
    Parser,
}

impl ImportPipelineErrorClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
            Self::Repairing => "repairing",
            Self::ArtifactRetirement => "artifact_retirement",
            Self::Metadata => "metadata",
            Self::MetadataInvariant => "metadata_invariant",
            Self::SourceUnavailable => "source_unavailable",
            Self::Scan => "scan",
            Self::FullText => "fulltext",
            Self::VectorContract => "vector_contract",
            Self::VectorStorage => "vector_storage",
            Self::EmbeddingRuntime => "embedding_runtime",
            Self::Privacy => "privacy",
            Self::Parser => "parser",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    };
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use fs_crawler::{crawl_directory, normalize_path, NormalizedPath, ScanProfile};
    use index_fulltext::{
        incremental_snapshot_documents, FullTextError, FullTextIndex, SearchQuery,
        SnapshotReadLease,
    };
    use index_vector::{QueryVector, VectorIndexError, VectorModelContract, VectorSnapshotRoot};
    use meta_store::{
        ActiveSearchProjection, ClassificationStatus, ContentDigest, CurrentClassifierEpoch,
        DataDirectoryOwnerAcquisition, DataDirectoryOwnerLease, Document, DocumentId,
        DocumentStatus, EntityMention, EntityMentionId, EntityType, FileExtension, ImportRootKind,
        ImportScanBudgetKind as StoreImportScanBudgetKind, ImportScanProfile, ImportScanScope,
        ImportTask, ImportTaskFailure, ImportTaskPurpose, ImportTaskStatus, IngestJobStatus,
        OcrAttemptFailure, OwnedMetaStore, ReadMetaStore, ReasonCode, ResumeVersion,
        ResumeVersionClassification, ResumeVersionId, ReviewDisposition, SearchMetadataHead,
        SearchProjectionServiceState, SearchProjectionTransitionOutcome, SearchPublicationState,
        SearchRepairReason, SearchSelection, SearchSelectionResolution, SourceRevision,
        UnixTimestamp, CLASSIFIER_EPOCH,
    };
    use resume_classifier::LinearPromotionPolicy;

    fn create_test_store(data_dir: &Path) -> OwnedMetaStore {
        let owner = match DataDirectoryOwnerLease::try_acquire(data_dir).unwrap() {
            DataDirectoryOwnerAcquisition::Acquired(owner) => owner,
            DataDirectoryOwnerAcquisition::Contended => panic!("test store owner contended"),
        };
        let store = owner.open_store().unwrap();
        store.run_migrations().unwrap();
        store
    }

    use super::search_artifact_cache::CachedSearchDocument;
    use super::{
        classify_language_set, commit_prepared_search_publication, current_timestamp_or,
        document_path_is_deletion_candidate, finish_import_file,
        flush_pending_searchable_documents, import_root_with_options,
        import_root_with_options_and_control, index_claimed_ocr_text, process_file,
        reconcile_search_artifacts, recv_parse_result_with_cancel_poll,
        should_flush_searchable_documents, take_pending_searchable_documents,
        write_incremental_search_artifacts, AdmissionDecision, CurrentImportCacheMode,
        CurrentImportDocumentCache, ImportCancelCheckPhase, ImportFailureKind,
        ImportHardwareProfile, ImportHardwareTier, ImportOptions, ImportParseWorkers,
        ImportPipelineError, ImportPipelineErrorClass, ImportPipelineErrorKind,
        ImportResourcePolicy, ImportRunControl, ImportStageTimings, ImportSummary,
        ImportWorkerMetrics, IndexDocument, IndexSection, OcrTextIndexOutcome, ParseWorkOutcome,
        ParseWorkResult, PendingProjectionRemovals, PendingSearchableDocument,
        PendingSearchablePublicationKind, ProcessedFile, SearchProjectionRemoval,
        SearchProjectionRemovalReason, SearchPublicationEmbeddingFailure,
        SearchPublicationEmbeddingInput, SearchPublicationEmbeddingOutput,
        SearchPublicationVectorization, SearchPublicationVectorizer, Sectionizer,
        SnapshotPublishPhase, BYTES_PER_GIB, H2_INDEX_WRITER_HEAP_BYTES,
    };

    #[path = "ocr_publication_tests.rs"]
    mod ocr_publication_tests;

    struct TestPublicationVectorizer {
        fail: bool,
    }

    #[test]
    fn fulltext_internal_failure_is_a_non_retryable_runtime_invariant() {
        let error = ImportPipelineError::index(FullTextError::Internal {
            diagnostic: "synthetic invariant".to_string(),
        });

        assert_eq!(error.class(), ImportPipelineErrorClass::FullText);
        assert!(!error.is_retryable());
    }

    #[test]
    fn unavailable_import_root_preserves_source_failure_class() {
        let temp = TestDir::new("import-pipeline-unavailable-root");
        let crawl_error = crawl_directory(temp.path().join("missing-root")).unwrap_err();
        let error = ImportPipelineError::crawl(crawl_error);

        assert_eq!(error.class(), ImportPipelineErrorClass::SourceUnavailable);
        assert!(error.is_retryable());
    }

    #[test]
    fn migration_ocr_staging_and_initial_base_preserve_an_inherited_visible_epoch() {
        let state = meta_store::SearchProjectionState {
            service_state: SearchProjectionServiceState::Repairing,
            generation: None,
            visible_epoch: 17,
            repair_reason: Some(SearchRepairReason::MigrationRebuild),
            publication: None,
            updated_at: UnixTimestamp::from_unix_seconds(1_700_000_020),
        };

        assert!(super::migration_rebuild::is_unpublished_migration_rebuild(
            &state
        ));
        let base = super::search_publication::migration_rebuild_publication_base_from_state(state)
            .unwrap();
        assert_eq!(base.generation, None);
        assert_eq!(base.visible_epoch, 17);
        assert!(base.projections.is_empty());
    }

    #[test]
    fn vector_contract_failures_are_non_retryable() {
        let failures = [
            VectorIndexError::InvalidDimension {
                expected: 2,
                actual: 3,
            },
            VectorIndexError::InvalidVectorValue,
            VectorIndexError::InvalidModelId,
            VectorIndexError::InvalidIdentity,
            VectorIndexError::InvalidGeneration,
            VectorIndexError::InvalidModelContract,
            VectorIndexError::SemanticUnavailable,
            VectorIndexError::PublicationProjectionMismatch,
            VectorIndexError::DuplicateVectorId,
            VectorIndexError::ConflictingDocumentVersion,
        ];

        for failure in failures {
            let error = ImportPipelineError::vector(failure);
            assert_eq!(error.class(), ImportPipelineErrorClass::VectorContract);
            assert!(!error.is_retryable(), "{failure:?} must fail closed");
        }
    }

    #[test]
    fn vector_storage_invariants_are_non_retryable() {
        let failures = [
            VectorIndexError::GenerationAlreadyExists,
            VectorIndexError::GenerationNotFound,
            VectorIndexError::LeaseRootMismatch,
            VectorIndexError::SchemaMismatch,
            VectorIndexError::CorruptSnapshot,
            VectorIndexError::StorageLayoutInvalid,
        ];

        for failure in failures {
            let error = ImportPipelineError::vector(failure);
            assert_eq!(error.class(), ImportPipelineErrorClass::VectorStorage);
            assert!(!error.is_retryable(), "{failure:?} must fail closed");
        }
    }

    #[test]
    fn vector_storage_failure_is_retryable() {
        let error = ImportPipelineError::vector(VectorIndexError::Storage);

        assert_eq!(error.class(), ImportPipelineErrorClass::VectorStorage);
        assert!(error.is_retryable());
    }

    impl SearchPublicationVectorizer for TestPublicationVectorizer {
        fn model_id(&self) -> &str {
            "synthetic-publication-v1"
        }

        fn dimension(&self) -> usize {
            2
        }

        fn max_batch_inputs(&self) -> usize {
            4
        }

        fn max_text_bytes(&self) -> usize {
            65_536
        }

        fn embed_batch(
            &self,
            inputs: &[SearchPublicationEmbeddingInput],
            _is_cancelled: &dyn Fn() -> bool,
        ) -> std::result::Result<
            Vec<SearchPublicationEmbeddingOutput>,
            SearchPublicationEmbeddingFailure,
        > {
            if self.fail {
                return Err(SearchPublicationEmbeddingFailure::RuntimeUnavailable);
            }
            Ok(inputs
                .iter()
                .map(|input| {
                    SearchPublicationEmbeddingOutput::new(
                        input.id(),
                        self.model_id(),
                        vec![1.0, input.text().len() as f32],
                    )
                })
                .collect())
        }
    }

    #[cfg(unix)]
    static DOC_CONVERTER_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn claim_ocr_document(
        store: &OwnedMetaStore,
        document: &Document,
        now: UnixTimestamp,
    ) -> meta_store::ClaimedOcrJob {
        let mut document = document.clone();
        document.status = DocumentStatus::OcrRequired;
        let content_hash = document
            .content_hash
            .as_deref()
            .and_then(|value| value.parse::<ContentDigest>().ok())
            .unwrap_or_else(|| ContentDigest::from_bytes(document.file_name.as_bytes()));
        document.content_hash = Some(content_hash.as_str().to_string());
        let source_revision =
            SourceRevision::for_content(document.id.clone(), content_hash, document.byte_size);
        store.upsert_document(&document).unwrap();
        store.insert_source_revision(&source_revision).unwrap();
        let triage = AdmissionDecision::ocr_backlog(&LinearPromotionPolicy::default())
            .into_source_triage(source_revision.id.clone(), now);
        store.insert_source_revision_triage(&triage).unwrap();
        let triage_epoch = CurrentClassifierEpoch::parse(&triage.triage_epoch).unwrap();
        store
            .enqueue_ocr_job_for_source_triage(&source_revision.id, triage_epoch, now)
            .unwrap();
        store.claim_next_ocr_job(now).unwrap().unwrap()
    }

    fn active_resume_version(store: &OwnedMetaStore, document: &Document) -> Option<ResumeVersion> {
        let projection = store
            .active_search_projection_for_document(&document.id)
            .unwrap()?;
        store
            .resume_version_by_id(&projection.resume_version_id)
            .unwrap()
    }

    fn ready_search_head(store: &OwnedMetaStore) -> SearchMetadataHead {
        store
            .with_search_metadata_snapshot(|snapshot| Ok::<_, ()>(snapshot.head().clone()))
            .unwrap()
    }

    fn ready_search_head_from_reader(store: &ReadMetaStore) -> SearchMetadataHead {
        store
            .with_search_metadata_snapshot(|snapshot| Ok::<_, ()>(snapshot.head().clone()))
            .unwrap()
    }

    fn initialize_ready_empty_search(_data_dir: &Path, store: &OwnedMetaStore, now: UnixTimestamp) {
        let contract =
            super::current_import_processing_contract(&ImportOptions::default()).unwrap();
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap();
        let summary = super::finalize_migration_rebuild(
            store,
            now,
            &contract,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        assert!(summary.active_generation_rebuilt);
    }

    fn open_fulltext_generation(data_dir: &Path, generation: &str) -> FullTextIndex {
        let index_root = data_dir.join("search-index");
        let lease = SnapshotReadLease::acquire(&index_root)
            .unwrap()
            .expect("ready publication must expose a full-text root lease");
        FullTextIndex::open_snapshot_with_lease(&index_root, generation, lease)
            .unwrap()
            .expect("ready publication must expose its exact full-text generation")
    }

    fn resolve_selection(
        store: &OwnedMetaStore,
        selection: &SearchSelection,
    ) -> SearchSelectionResolution {
        store
            .with_search_metadata_snapshot(|snapshot| snapshot.resolve_search_selection(selection))
            .unwrap()
    }

    #[test]
    fn invalid_publication_capability_blocks_repair_without_forging_an_attempt() {
        let temp = TestDir::new("import-pipeline-invalid-publication-capability");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        fs::remove_file(data_dir.join("search-publication.lock")).unwrap();
        fs::create_dir_all(data_dir.join("search-publication.lock")).unwrap();
        let contract =
            super::current_import_processing_contract(&ImportOptions::default()).unwrap();
        store
            .activate_migration_rebuild_contract(
                &contract,
                UnixTimestamp::from_unix_seconds(1_700_000_100),
            )
            .unwrap();

        let error = super::finalize_migration_rebuild(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_100),
            &contract,
            &SearchPublicationVectorization::default(),
        )
        .unwrap_err();
        assert_eq!(error.class(), ImportPipelineErrorClass::MetadataInvariant);
        assert_eq!(error.metadata_class_label(), Some("invalid_value"));
        assert!(store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .is_none());
        let state = store.search_projection_state().unwrap();
        assert_eq!(
            state.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::RuntimeInvariant)
        );
        assert_eq!(state.generation, None);
    }

    #[test]
    fn migration_rebuild_vector_failures_are_bounded_without_artifact_accumulation() {
        let temp = TestDir::new("import-pipeline-migration-vector-failure-budget");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            synthetic_resume_text("Vector Failure Candidate", "Rust Search"),
        )
        .unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_200);
        let vectorization =
            SearchPublicationVectorization::enabled(Arc::new(TestPublicationVectorizer {
                fail: true,
            }));
        let options = ImportOptions {
            search_vectorization: vectorization.clone(),
            ..ImportOptions::default()
        };
        let task = import_task("migration-vector-failure", root.to_str().unwrap(), now);
        let contract = insert_test_import_task(&store, &task, &options);

        let first_error =
            import_root_with_options(&data_dir, &store, &task, &root, now, options).unwrap_err();
        assert_eq!(
            first_error.class(),
            ImportPipelineErrorClass::EmbeddingRuntime
        );
        assert_eq!(
            store.import_task_by_id(&task.id).unwrap().unwrap().status,
            ImportTaskStatus::Completed
        );
        assert_no_search_artifact_candidates(&data_dir);

        for expected_attempt_count in 2..=5 {
            let retry_at = store
                .migration_rebuild_publication_attempt_state()
                .unwrap()
                .unwrap()
                .next_retry_at
                .expect("failed migration publication must be in retry_wait");
            let error =
                super::finalize_migration_rebuild(&store, retry_at, &contract, &vectorization)
                    .unwrap_err();
            assert_eq!(error.class(), ImportPipelineErrorClass::EmbeddingRuntime);
            assert_eq!(
                store
                    .migration_rebuild_publication_attempt_state()
                    .unwrap()
                    .unwrap()
                    .attempt_count,
                expected_attempt_count
            );
            assert_no_search_artifact_candidates(&data_dir);
            assert!(store
                .interrupted_search_publications(256)
                .unwrap()
                .is_empty());
        }

        let state = store.search_projection_state().unwrap();
        assert_eq!(
            state.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::RuntimeInvariant)
        );
    }

    #[test]
    fn migration_rebuild_fulltext_failures_are_bounded_before_vector_layout_exists() {
        let temp = TestDir::new("import-pipeline-migration-fulltext-failure-budget");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let contract =
            super::current_import_processing_contract(&ImportOptions::default()).unwrap();
        store
            .activate_migration_rebuild_contract(
                &contract,
                UnixTimestamp::from_unix_seconds(1_700_000_300),
            )
            .unwrap();
        for attempt_at in [
            1_700_000_300,
            1_700_000_301,
            1_700_000_305,
            1_700_000_320,
            1_700_000_350,
        ] {
            let error = super::index_recovery::finalize_migration_rebuild_with_fault(
                &store,
                UnixTimestamp::from_unix_seconds(attempt_at),
                &contract,
                &SearchPublicationVectorization::default(),
                super::index_recovery::MigrationPublicationFault::RetryableFullText,
            )
            .unwrap_err();
            assert_eq!(error.class(), ImportPipelineErrorClass::FullText);
            assert_no_search_artifact_candidates(&data_dir);
        }

        let attempt = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(attempt.attempt_count, 5);
        assert_eq!(
            attempt.last_error_class,
            Some(meta_store::MigrationRebuildPublicationErrorClass::FullText)
        );
        assert_eq!(
            store.search_projection_state().unwrap().service_state,
            SearchProjectionServiceState::RepairBlocked
        );
    }

    #[test]
    fn migration_rebuild_retry_deadline_is_relative_to_attempt_completion() {
        let temp = TestDir::new("import-pipeline-migration-failure-finished-at");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let contract =
            super::current_import_processing_contract(&ImportOptions::default()).unwrap();
        let started_at = UnixTimestamp::from_unix_seconds(1_700_000_600);
        let finished_at = UnixTimestamp::from_unix_seconds(1_700_000_660);
        store
            .activate_migration_rebuild_contract(&contract, started_at)
            .unwrap();

        super::index_recovery::finalize_migration_rebuild_with_fault(
            &store,
            started_at,
            &contract,
            &SearchPublicationVectorization::default(),
            super::index_recovery::MigrationPublicationFault::RetryableFullTextFinishedAt(
                finished_at,
            ),
        )
        .unwrap_err();

        let attempt = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(attempt.attempt_count, 1);
        assert_eq!(
            attempt.next_retry_at,
            Some(UnixTimestamp::from_unix_seconds(1_700_000_661))
        );
    }

    #[cfg(unix)]
    #[test]
    fn migration_rebuild_partial_cleanup_blocks_on_the_first_attempt() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TestDir::new("import-pipeline-migration-partial-cleanup");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let fulltext_root = data_dir.join("search-index");
        index_fulltext::publish_trusted_redacted_snapshot_with_control(
            &fulltext_root,
            "orphan-generation",
            Vec::<IndexDocument>::new(),
            index_fulltext::SnapshotPublishControl::disabled(),
        )
        .unwrap();
        let snapshots = fulltext_root.join("snapshots");
        fs::set_permissions(&snapshots, fs::Permissions::from_mode(0o500)).unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let contract =
            super::current_import_processing_contract(&ImportOptions::default()).unwrap();
        store
            .activate_migration_rebuild_contract(
                &contract,
                UnixTimestamp::from_unix_seconds(1_700_000_400),
            )
            .unwrap();
        let error = super::index_recovery::finalize_migration_rebuild_with_fault(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_400),
            &contract,
            &SearchPublicationVectorization::default(),
            super::index_recovery::MigrationPublicationFault::RetryableFullText,
        )
        .unwrap_err();
        fs::set_permissions(&snapshots, fs::Permissions::from_mode(0o700)).unwrap();

        assert_eq!(error.class(), ImportPipelineErrorClass::FullText);
        let attempt = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(attempt.attempt_count, 1);
        assert_eq!(
            attempt.last_error_class,
            Some(meta_store::MigrationRebuildPublicationErrorClass::Cleanup)
        );
        assert_eq!(
            store.search_projection_state().unwrap().service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert!(fulltext_root.join("snapshots/orphan-generation").exists());
    }

    #[test]
    fn migration_rebuild_cleanup_error_blocks_on_the_first_attempt() {
        let temp = TestDir::new("import-pipeline-migration-cleanup-error");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("search-index"), b"invalid artifact root").unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let contract =
            super::current_import_processing_contract(&ImportOptions::default()).unwrap();
        store
            .activate_migration_rebuild_contract(
                &contract,
                UnixTimestamp::from_unix_seconds(1_700_000_500),
            )
            .unwrap();

        let error = super::index_recovery::finalize_migration_rebuild_with_fault(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_500),
            &contract,
            &SearchPublicationVectorization::default(),
            super::index_recovery::MigrationPublicationFault::RetryableFullText,
        )
        .unwrap_err();

        assert_eq!(error.class(), ImportPipelineErrorClass::FullText);
        let attempt = store
            .migration_rebuild_publication_attempt_state()
            .unwrap()
            .unwrap();
        assert_eq!(attempt.attempt_count, 1);
        assert_eq!(
            attempt.last_error_class,
            Some(meta_store::MigrationRebuildPublicationErrorClass::Cleanup)
        );
        assert_eq!(
            store.search_projection_state().unwrap().service_state,
            SearchProjectionServiceState::RepairBlocked
        );
    }

    fn assert_no_search_artifact_candidates(data_dir: &Path) {
        for relative in [
            "search-index/snapshots",
            "search-index/staging",
            "search-index/generation-pins",
            "vector-index/snapshots",
            "vector-index/staging",
            "vector-index/generation-pins",
        ] {
            let path = data_dir.join(relative);
            let count = match fs::read_dir(&path) {
                Ok(entries) => entries.count(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(error) => panic!("failed to inspect {relative}: {error}"),
            };
            assert_eq!(count, 0, "artifact candidates remain under {relative}");
        }
    }

    fn test_source_revision(document: &Document) -> SourceRevision {
        SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(document.file_name.as_bytes()),
            document.byte_size,
        )
    }

    #[test]
    fn content_addressed_version_changes_when_source_revision_changes() {
        let mut document = test_document("content-identity", DocumentStatus::TextCleaned);
        let revision_a = SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(b"source-a"),
            8,
        );
        let revision_b = SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(b"source-b"),
            8,
        );
        let clean_text = "Synthetic Candidate\nEXPERIENCE\nRust Search";
        document.content_hash = Some(revision_a.content_hash.as_str().to_string());
        let version_a = super::resume_version(
            &document,
            &revision_a,
            clean_text.to_string(),
            "parser-v1",
            "schema-v27",
            vec!["en".to_string()],
            Some(1),
            Some(0.8),
        );
        document.content_hash = Some(revision_b.content_hash.as_str().to_string());
        let version_b = super::resume_version(
            &document,
            &revision_b,
            clean_text.to_string(),
            "parser-v1",
            "schema-v27",
            vec!["en".to_string()],
            Some(1),
            Some(0.8),
        );

        assert_ne!(revision_a.id, revision_b.id);
        assert_ne!(version_a.id, version_b.id);
        assert_eq!(
            version_a.normalized_text_hash,
            ContentDigest::from_bytes(clean_text.as_bytes())
        );
        assert_eq!(
            version_a.normalized_text_hash,
            version_b.normalized_text_hash
        );
    }

    #[test]
    fn staged_versions_are_invisible_and_serial_publications_advance_atomically() {
        let temp = TestDir::new("staged-publication-cas");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_199_999),
        );
        let first = test_pending_searchable_document("publication-first");
        let second = test_pending_searchable_document("publication-second");
        for pending in [&first, &second] {
            super::immutable_ingest::stage(
                &store,
                super::StagedResume {
                    document: &pending.document,
                    source_revision: &pending.source_revision,
                    derived: super::StagedDerivedData::ClassifiedVersion {
                        version: &pending.version,
                        classification: &pending.classification,
                        mentions: &pending.mentions,
                        email_hash: None,
                        phone_hash: None,
                    },
                },
            )
            .unwrap();
            assert_eq!(
                store
                    .active_search_projection_for_document(&pending.document.id)
                    .unwrap(),
                None
            );
        }

        let now = UnixTimestamp::from_unix_seconds(1_700_200_000);
        let first_session = store.wait_for_search_publication_session().unwrap();
        let first_publication = write_incremental_search_artifacts(
            &first_session,
            now,
            CLASSIFIER_EPOCH,
            vec![first.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let mut first_document = first.document.clone();
        first_document.status = DocumentStatus::Searchable;
        first_document.updated_at = now;
        let first_publication = commit_prepared_search_publication(
            now,
            first_publication,
            std::slice::from_ref(&first_document),
        )
        .unwrap()
        .release();
        drop(first_session);
        let second_now = UnixTimestamp::from_unix_seconds(now.as_unix_seconds() + 1);
        let second_session = store.wait_for_search_publication_session().unwrap();
        let second_publication = write_incremental_search_artifacts(
            &second_session,
            second_now,
            CLASSIFIER_EPOCH,
            vec![second.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let mut second_document = second.document.clone();
        second_document.status = DocumentStatus::Searchable;
        second_document.updated_at = second_now;
        let second_publication = commit_prepared_search_publication(
            second_now,
            second_publication,
            std::slice::from_ref(&second_document),
        )
        .unwrap()
        .release();
        drop(second_session);
        assert_eq!(first_publication.projections.len(), 1);
        assert_eq!(second_publication.projections.len(), 2);
        assert_eq!(
            store
                .active_search_projection_for_document(&first.document.id)
                .unwrap(),
            Some(ActiveSearchProjection {
                document_id: first.document.id.clone(),
                resume_version_id: first.version.id.clone(),
            })
        );
        assert_eq!(
            store
                .active_search_projection_for_document(&second.document.id)
                .unwrap(),
            Some(ActiveSearchProjection {
                document_id: second.document.id.clone(),
                resume_version_id: second.version.id.clone(),
            })
        );
    }

    #[test]
    fn vector_publication_is_exact_atomic_and_retained_across_removal() {
        let temp = TestDir::new("vector-publication-atomic");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_209_999),
        );
        let first = test_pending_searchable_document("vector-first");
        let second = test_pending_searchable_document("vector-second");
        for pending in [&first, &second] {
            super::immutable_ingest::stage(
                &store,
                super::StagedResume {
                    document: &pending.document,
                    source_revision: &pending.source_revision,
                    derived: super::StagedDerivedData::ClassifiedVersion {
                        version: &pending.version,
                        classification: &pending.classification,
                        mentions: &pending.mentions,
                        email_hash: None,
                        phone_hash: None,
                    },
                },
            )
            .unwrap();
        }

        let enabled =
            SearchPublicationVectorization::enabled(Arc::new(TestPublicationVectorizer {
                fail: false,
            }));
        let first_now = UnixTimestamp::from_unix_seconds(1_700_210_000);
        let first_session = store.wait_for_search_publication_session().unwrap();
        let first_publication = write_incremental_search_artifacts(
            &first_session,
            first_now,
            CLASSIFIER_EPOCH,
            vec![first.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &enabled,
        )
        .unwrap();
        let mut first_document = first.document.clone();
        first_document.status = DocumentStatus::Searchable;
        first_document.updated_at = first_now;
        commit_prepared_search_publication(
            first_now,
            first_publication,
            std::slice::from_ref(&first_document),
        )
        .unwrap()
        .release();
        drop(first_session);
        let first_head = ready_search_head(&store);

        let failing =
            SearchPublicationVectorization::enabled(Arc::new(TestPublicationVectorizer {
                fail: true,
            }));
        let failing_session = store.wait_for_search_publication_session().unwrap();
        let failed = write_incremental_search_artifacts(
            &failing_session,
            UnixTimestamp::from_unix_seconds(1_700_210_001),
            CLASSIFIER_EPOCH,
            vec![second.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &failing,
        );
        assert!(failed.is_err());
        drop(failing_session);
        assert_eq!(ready_search_head(&store).generation, first_head.generation);
        assert_eq!(
            store
                .active_search_projection_for_document(&second.document.id)
                .unwrap(),
            None
        );

        let second_now = UnixTimestamp::from_unix_seconds(1_700_210_002);
        let second_session = store.wait_for_search_publication_session().unwrap();
        let second_publication = write_incremental_search_artifacts(
            &second_session,
            second_now,
            CLASSIFIER_EPOCH,
            vec![second.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &enabled,
        )
        .unwrap();
        let mut second_document = second.document.clone();
        second_document.status = DocumentStatus::Searchable;
        second_document.updated_at = second_now;
        commit_prepared_search_publication(
            second_now,
            second_publication,
            std::slice::from_ref(&second_document),
        )
        .unwrap()
        .release();
        drop(second_session);
        assert_vector_generation(&data_dir, &store, 2);

        super::publish_search_projection_removals(
            &store,
            &[SearchProjectionRemoval {
                document_id: second.document.id.clone(),
                reason: SearchProjectionRemovalReason::ConfirmedSourceDeletion,
            }],
            UnixTimestamp::from_unix_seconds(1_700_210_003),
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        assert_vector_generation(&data_dir, &store, 1);
    }

    #[test]
    fn reconcile_promotes_a_usable_disabled_snapshot_to_the_configured_vector_contract() {
        let temp = TestDir::new("vector-publication-reconcile-contract");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_219_999),
        );
        let pending = test_pending_searchable_document("vector-reconcile");
        super::immutable_ingest::stage(
            &store,
            super::StagedResume {
                document: &pending.document,
                source_revision: &pending.source_revision,
                derived: super::StagedDerivedData::ClassifiedVersion {
                    version: &pending.version,
                    classification: &pending.classification,
                    mentions: &pending.mentions,
                    email_hash: None,
                    phone_hash: None,
                },
            },
        )
        .unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_220_000);
        let publication_session = store.wait_for_search_publication_session().unwrap();
        let publication = write_incremental_search_artifacts(
            &publication_session,
            now,
            CLASSIFIER_EPOCH,
            vec![pending.index_document.clone()],
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let mut document = pending.document.clone();
        document.status = DocumentStatus::Searchable;
        document.updated_at = now;
        commit_prepared_search_publication(now, publication, std::slice::from_ref(&document))
            .unwrap()
            .release();
        drop(publication_session);

        let enabled =
            SearchPublicationVectorization::enabled(Arc::new(TestPublicationVectorizer {
                fail: false,
            }));
        let summary = reconcile_search_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_220_001),
            &enabled,
        )
        .unwrap();

        assert!(summary.active_generation_rebuilt);
        assert_vector_generation(&data_dir, &store, 1);
    }

    #[test]
    fn reconcile_resumes_an_interrupted_artifact_repair() {
        let temp = TestDir::new("artifact-repair-resume");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_220_100),
        );
        let observed = store.search_projection_state().unwrap();
        assert_eq!(
            store
                .begin_artifact_repair(
                    observed.generation.as_deref().unwrap(),
                    observed.visible_epoch,
                    UnixTimestamp::from_unix_seconds(1_700_220_101),
                )
                .unwrap(),
            SearchProjectionTransitionOutcome::Applied
        );

        let summary = reconcile_search_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_220_102),
            &SearchPublicationVectorization::default(),
        )
        .unwrap();

        assert!(summary.active_generation_rebuilt);
        let ready = store.search_projection_state().unwrap();
        assert_eq!(ready.service_state, SearchProjectionServiceState::Ready);
        assert_eq!(ready.repair_reason, None);
        assert_eq!(ready.visible_epoch, observed.visible_epoch + 1);
    }

    fn assert_vector_generation(
        data_dir: &Path,
        store: &OwnedMetaStore,
        expected_documents: usize,
    ) {
        let head = ready_search_head(store);
        let vector = head.publication.vector.as_ref().unwrap();
        let contract = VectorModelContract::enabled("synthetic-publication-v1", 2).unwrap();
        let root = VectorSnapshotRoot::new(data_dir.join("vector-index")).unwrap();
        let reader = root
            .open_generation_with_lease(
                vector.generation(),
                &contract,
                root.acquire_read_lease().unwrap(),
            )
            .unwrap();
        assert_eq!(reader.summary().vector_document_count(), expected_documents);
        assert_eq!(reader.exact_projection().len(), expected_documents);
        assert_eq!(
            reader
                .knn(QueryVector::new(vec![1.0, 1.0]).unwrap(), 10)
                .unwrap()
                .len(),
            expected_documents
        );
    }

    #[test]
    fn discovery_deletion_requires_direct_parent_directory_to_be_scanned() {
        let root = Path::new("/fixture");
        let scanned_directories = vec![normalized_path("/fixture")];

        assert!(document_path_is_deletion_candidate(
            "/fixture/root-resume.pdf",
            root,
            ScanProfile::Discovery,
            &scanned_directories,
            &[],
        ));
        assert!(!document_path_is_deletion_candidate(
            "/fixture/unreadable/resume.pdf",
            root,
            ScanProfile::Discovery,
            &scanned_directories,
            &[],
        ));
    }

    #[test]
    fn discovery_deletion_excludes_skipped_subtrees_even_when_parent_was_seen() {
        let root = Path::new("/fixture");
        let scanned_directories = vec![
            normalized_path("/fixture"),
            normalized_path("/fixture/Documents"),
        ];
        let skipped_directories = vec![normalized_path("/fixture/node_modules")];

        assert!(document_path_is_deletion_candidate(
            "/fixture/Documents/resume.pdf",
            root,
            ScanProfile::Discovery,
            &scanned_directories,
            &skipped_directories,
        ));
        assert!(!document_path_is_deletion_candidate(
            "/fixture/node_modules/resume.pdf",
            root,
            ScanProfile::Discovery,
            &scanned_directories,
            &skipped_directories,
        ));
    }

    #[test]
    fn deletion_candidate_matches_windows_normalized_paths() {
        let root = Path::new(r"C:\fixture");
        let scanned_directories = vec![normalized_path(r"C:\fixture")];

        assert!(document_path_is_deletion_candidate(
            "c:/fixture/resume.pdf",
            root,
            ScanProfile::Explicit,
            &scanned_directories,
            &[],
        ));
        assert!(!document_path_is_deletion_candidate(
            "c:/fixture-neighbor/resume.pdf",
            root,
            ScanProfile::Explicit,
            &scanned_directories,
            &[],
        ));
    }

    #[test]
    fn current_timestamp_or_never_returns_before_default_timestamp() {
        let future_default = UnixTimestamp::from_unix_seconds(4_000_000_000);

        assert_eq!(current_timestamp_or(future_default), future_default);
    }

    #[test]
    fn searchable_flush_policy_publishes_first_match_then_batches_followups() {
        assert!(should_flush_searchable_documents(0, 100, 1, 0));
        assert!(!should_flush_searchable_documents(8, 100, 8, 1));
        assert!(!should_flush_searchable_documents(31, 1000, 32, 1));
        assert!(should_flush_searchable_documents(99, 1000, 99, 1));
        assert!(!should_flush_searchable_documents(100, 1000, 1, 100));
        assert!(!should_flush_searchable_documents(126, 1000, 27, 100));
        assert!(!should_flush_searchable_documents(127, 1000, 28, 100));
        assert!(!should_flush_searchable_documents(510, 1000, 411, 100));
        assert!(!should_flush_searchable_documents(511, 1000, 412, 100));
        assert!(should_flush_searchable_documents(998, 2000, 900, 100));
        assert!(!should_flush_searchable_documents(999, 2000, 1, 1000));
        assert!(!should_flush_searchable_documents(1022, 2000, 1023, 1000));
        assert!(should_flush_searchable_documents(1023, 2000, 1024, 1000));
        assert!(!should_flush_searchable_documents(999, 1000, 1, 1));
    }

    #[test]
    fn current_import_index_cache_refreshes_after_intervening_snapshot_publication() {
        let temp = TestDir::new("import-pipeline-current-import-index-cache-refresh");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_049),
        );
        let empty_exclusions = BTreeSet::new();
        let mut current_import_index_documents = CurrentImportDocumentCache::default();

        let first_session = store.wait_for_search_publication_session().unwrap();
        let first_publication = write_incremental_search_artifacts(
            &first_session,
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-1")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let first_publication = commit_prepared_search_publication(
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            first_publication,
            &[terminal_searchable_document(
                &store,
                "doc-1",
                UnixTimestamp::from_unix_seconds(1_700_000_050),
            )],
        )
        .unwrap()
        .release();
        drop(first_session);
        assert_eq!(first_publication.fulltext.document_count(), 1);
        assert_eq!(current_import_index_documents.documents.len(), 1);
        assert_eq!(
            retained_section_text_bytes(&current_import_index_documents),
            0
        );

        let intervening_index_document = stage_test_index_document(&store, "doc-2");
        let intervening_doc_id = intervening_index_document.doc_id.clone();
        let intervening_session = store.wait_for_search_publication_session().unwrap();
        let intervening_publication = write_incremental_search_artifacts(
            &intervening_session,
            UnixTimestamp::from_unix_seconds(1_700_000_051),
            CLASSIFIER_EPOCH,
            vec![intervening_index_document],
            &empty_exclusions,
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let intervening_publication = commit_prepared_search_publication(
            UnixTimestamp::from_unix_seconds(1_700_000_051),
            intervening_publication,
            &[terminal_searchable_document(
                &store,
                "doc-2",
                UnixTimestamp::from_unix_seconds(1_700_000_051),
            )],
        )
        .unwrap()
        .release();
        drop(intervening_session);
        assert_eq!(intervening_publication.fulltext.document_count(), 2);

        let second_session = store.wait_for_search_publication_session().unwrap();
        let second_publication = write_incremental_search_artifacts(
            &second_session,
            UnixTimestamp::from_unix_seconds(1_700_000_052),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-3")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();

        assert_eq!(second_publication.fulltext.document_count(), 3);
        let cached_doc_ids = current_import_index_documents
            .documents
            .iter()
            .map(|document| document.doc_id.clone())
            .collect::<Vec<_>>();
        let mut expected_doc_ids = vec![
            DocumentId::from_non_secret_parts(&["doc-1"]).to_string(),
            intervening_doc_id.clone(),
            DocumentId::from_non_secret_parts(&["doc-3"]).to_string(),
        ];
        expected_doc_ids.sort();
        assert_eq!(cached_doc_ids, expected_doc_ids);
        let active_doc_ids = incremental_snapshot_documents(
            &data_dir.join("search-index"),
            Some(second_publication.fulltext.generation()),
            Vec::new(),
            &BTreeSet::new(),
        )
        .unwrap()
        .into_iter()
        .map(|document| document.doc_id)
        .collect::<Vec<_>>();
        assert_eq!(active_doc_ids, expected_doc_ids);
        assert_eq!(
            retained_section_text_bytes(&current_import_index_documents),
            0
        );
    }
    #[test]
    fn current_import_cache_ignores_uncommitted_generations_and_recovery_abandons_them() {
        let temp = TestDir::new("import-pipeline-uncommitted-generation");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_059),
        );
        let empty_exclusions = BTreeSet::new();
        let mut current_import_documents = CurrentImportDocumentCache::default();
        let ready_now = UnixTimestamp::from_unix_seconds(1_700_000_060);

        let ready_session = store.wait_for_search_publication_session().unwrap();
        let ready = write_incremental_search_artifacts(
            &ready_session,
            ready_now,
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-1")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let ready = commit_prepared_search_publication(
            ready_now,
            ready,
            &[terminal_searchable_document(&store, "doc-1", ready_now)],
        )
        .unwrap()
        .release();
        drop(ready_session);

        let uncommitted_session = store.wait_for_search_publication_session().unwrap();
        let uncommitted = write_incremental_search_artifacts(
            &uncommitted_session,
            UnixTimestamp::from_unix_seconds(1_700_000_061),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-2")],
            &empty_exclusions,
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let uncommitted_generation = uncommitted.fulltext.generation().to_string();
        drop(uncommitted);
        drop(uncommitted_session);
        let next_session = store.wait_for_search_publication_session().unwrap();
        let next = write_incremental_search_artifacts(
            &next_session,
            UnixTimestamp::from_unix_seconds(1_700_000_062),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-3")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let next_generation = next.fulltext.generation().to_string();

        let indexed_doc_ids = incremental_snapshot_documents(
            &data_dir.join("search-index"),
            Some(&next_generation),
            Vec::new(),
            &BTreeSet::new(),
        )
        .unwrap()
        .into_iter()
        .map(|document| document.doc_id)
        .collect::<Vec<_>>();
        assert_eq!(
            indexed_doc_ids,
            vec![
                DocumentId::from_non_secret_parts(&["doc-1"]).to_string(),
                DocumentId::from_non_secret_parts(&["doc-3"]).to_string(),
            ]
        );
        drop(next);
        drop(next_session);

        let recovery = reconcile_search_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_063),
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        assert_eq!(recovery.interrupted_publications_abandoned, 2);
        assert!(!recovery.active_generation_rebuilt);
        assert_eq!(
            ready_search_head(&store).generation,
            ready.fulltext.generation()
        );
        for generation in [&uncommitted_generation, &next_generation] {
            assert_eq!(
                store.search_publication(generation).unwrap().unwrap().state,
                SearchPublicationState::Abandoned
            );
        }
        let ready_reader = open_fulltext_generation(&data_dir, ready.fulltext.generation());
        assert_eq!(
            ready_reader
                .search(SearchQuery::new("synthetic").with_limit(5))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn recovery_rebuilds_an_exact_fulltext_vector_pair_from_metadata() {
        let temp = TestDir::new("import-pipeline-search-artifact-recovery");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("candidate.txt"),
            synthetic_resume_text("Synthetic Recovery Candidate", "Rust recovery"),
        )
        .unwrap();
        let store = create_test_store(&data_dir);
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_070);
        let task = import_task(
            "search-artifact-recovery",
            root.to_str().unwrap(),
            first_now,
        );
        insert_test_import_task(&store, &task, &ImportOptions::default());
        import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();

        let first_head = ready_search_head(&store);
        let first_projection = store
            .active_search_projection_for_document(&store.visible_documents().unwrap().remove(0).id)
            .unwrap()
            .unwrap();
        let first_selection = SearchSelection {
            document_id: first_projection.document_id.clone(),
            resume_version_id: first_projection.resume_version_id.clone(),
            visible_epoch: first_head.visible_epoch,
        };
        fs::remove_file(
            data_dir
                .join("search-index")
                .join("snapshots")
                .join(&first_head.generation)
                .join("snapshot-manifest.json"),
        )
        .unwrap();

        let recovery = reconcile_search_artifacts(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_071),
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        assert!(recovery.active_generation_rebuilt);
        let recovered_head = ready_search_head(&store);
        assert_ne!(recovered_head.generation, first_head.generation);
        let fulltext = recovered_head.publication.fulltext.as_ref().unwrap();
        let vector = recovered_head.publication.vector.as_ref().unwrap();
        assert_eq!(fulltext.generation(), recovered_head.generation);
        assert_eq!(vector.generation(), recovered_head.generation);
        assert_eq!(fulltext.projection_digest(), vector.projection_digest());
        assert_eq!(fulltext.document_count(), vector.projection_count());

        let recovered = open_fulltext_generation(&data_dir, &recovered_head.generation);
        assert_eq!(
            recovered
                .search(SearchQuery::new("Rust recovery").with_limit(5))
                .unwrap()
                .len(),
            1
        );
        let vector_root = VectorSnapshotRoot::new(data_dir.join("vector-index")).unwrap();
        let vector_reader = vector_root
            .open_generation_with_lease(
                &recovered_head.generation,
                &VectorModelContract::Disabled,
                vector_root.acquire_read_lease().unwrap(),
            )
            .unwrap();
        assert_eq!(
            vector_reader.summary().projection_digest(),
            fulltext.projection_digest()
        );
        assert_eq!(
            resolve_selection(&store, &first_selection),
            SearchSelectionResolution::Current {
                selection: first_selection
            }
        );
    }

    #[test]
    fn current_import_index_cache_consumes_final_flush_documents() {
        let temp = TestDir::new("import-pipeline-current-import-index-cache-final");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_049),
        );
        let empty_exclusions = BTreeSet::new();
        let mut current_import_index_documents = CurrentImportDocumentCache::default();

        let ready_session = store.wait_for_search_publication_session().unwrap();
        let ready_publication = write_incremental_search_artifacts(
            &ready_session,
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-1")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        commit_prepared_search_publication(
            UnixTimestamp::from_unix_seconds(1_700_000_050),
            ready_publication,
            &[terminal_searchable_document(
                &store,
                "doc-1",
                UnixTimestamp::from_unix_seconds(1_700_000_050),
            )],
        )
        .unwrap()
        .release();
        drop(ready_session);
        assert_eq!(current_import_index_documents.documents.len(), 1);

        let final_session = store.wait_for_search_publication_session().unwrap();
        let final_publication = write_incremental_search_artifacts(
            &final_session,
            UnixTimestamp::from_unix_seconds(1_700_000_051),
            CLASSIFIER_EPOCH,
            vec![stage_test_index_document(&store, "doc-2")],
            &empty_exclusions,
            0,
            0,
            Some(&mut current_import_index_documents),
            CurrentImportCacheMode::Consume,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();

        assert_eq!(final_publication.fulltext.document_count(), 2);
        assert!(current_import_index_documents.documents.is_empty());
    }

    #[test]
    fn current_import_index_cache_redacts_contact_text_before_retaining() {
        let cached = CachedSearchDocument::from_index_document(IndexDocument {
            doc_id: "doc-contact".to_string(),
            resume_version_id: "ver-contact".to_string(),
            file_name: "person@example.test resume.pdf".to_string(),
            clean_text:
                "Email person@example.test phone +1 650-555-1234 file /Users/private/resume.pdf"
                    .to_string(),
            sections: Vec::new(),
        });

        assert!(cached.file_name.contains("<redacted-email>"));
        assert!(cached.clean_text.contains("<redacted-email>"));
        assert!(cached.clean_text.contains("<redacted-phone>"));
        assert!(cached.clean_text.contains("<redacted-path>"));
        assert!(!cached.file_name.contains("person@example.test"));
        assert!(!cached.clean_text.contains("person@example.test"));
        assert!(!cached.clean_text.contains("650-555-1234"));
        assert!(!cached.clean_text.contains("/Users/private"));
    }

    #[test]
    fn pending_searchable_documents_are_moved_into_flush_inputs() {
        let mut pending = vec![
            test_pending_searchable_document("doc-2"),
            test_pending_searchable_document("doc-1"),
        ];

        let (documents, replacements) = take_pending_searchable_documents(&mut pending);

        assert!(pending.is_empty());
        assert_eq!(
            documents
                .iter()
                .map(|document| document.file_name.as_str())
                .collect::<Vec<_>>(),
            vec!["doc-2.txt", "doc-1.txt"]
        );
        assert_eq!(
            replacements
                .iter()
                .map(|document| document.doc_id.clone())
                .collect::<Vec<_>>(),
            vec![
                DocumentId::from_non_secret_parts(&["doc-2"]).to_string(),
                DocumentId::from_non_secret_parts(&["doc-1"]).to_string(),
            ]
        );
    }

    #[test]
    fn failed_staging_batch_does_not_publish_projection_or_index() {
        let temp = TestDir::new("searchable-metadata-batch-rollback");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);

        let mut first = test_pending_searchable_document("batch-first");
        let mut second = test_pending_searchable_document("batch-second");
        let duplicate_id = EntityMentionId::from_non_secret_parts(&["test", "duplicate"]);
        first.mentions = vec![test_entity_mention(
            duplicate_id.clone(),
            first.version.id.clone(),
        )];
        second.mentions = vec![test_entity_mention(duplicate_id, second.version.id.clone())];
        let documents = [
            (first.document.id.clone(), first.version.id.clone()),
            (second.document.id.clone(), second.version.id.clone()),
        ];
        let mut pending = vec![first, second];
        let mut excluded = PendingProjectionRemovals::default();
        let mut summary = ImportSummary::default();

        let error = flush_pending_searchable_documents(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_000),
            &mut summary,
            &mut pending,
            &mut excluded,
            None,
            CurrentImportCacheMode::Retain,
            &|| Ok(()),
            &|_| {},
            Instant::now(),
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        )
        .unwrap_err();

        assert_eq!(
            error.kind,
            ImportPipelineErrorKind::StoreInvariant(
                meta_store::MetaStoreErrorClass::ImmutableIdentityConflict
            )
        );
        assert_eq!(pending.len(), 2);
        for (document_id, _) in documents {
            assert_eq!(
                store
                    .active_search_projection_for_document(&document_id)
                    .unwrap(),
                None
            );
        }
        assert!(!data_dir.join("search-index").exists());
    }

    #[test]
    fn language_set_classifier_preserves_order_and_unknown_fallback() {
        assert_eq!(classify_language_set("Rust 中文简历"), vec!["en", "zh"]);
        assert_eq!(classify_language_set("中文简历"), vec!["zh"]);
        assert_eq!(classify_language_set("Rust resume"), vec!["en"]);
        assert_eq!(classify_language_set("  123 !!!  "), vec!["unknown"]);
    }

    #[test]
    fn import_options_low_memory_default_caps_parse_workers() {
        assert_eq!(
            ImportOptions::low_memory_default_for_available_parallelism(8)
                .parse_workers
                .get(),
            3
        );
        assert_eq!(
            ImportOptions::low_memory_default_for_available_parallelism(2)
                .parse_workers
                .get(),
            2
        );
        assert_eq!(
            ImportOptions::low_memory_default_for_available_parallelism(1)
                .parse_workers
                .get(),
            1
        );
        assert_eq!(ImportParseWorkers::new(99).get(), 3);
    }

    #[test]
    fn import_resource_policy_classifies_ram_and_cpu_tiers() {
        let h0 = ImportResourcePolicy::for_hardware(ImportHardwareProfile::new(
            Some(8 * BYTES_PER_GIB),
            10,
        ));
        assert_eq!(h0.hardware_tier, ImportHardwareTier::H0Eco);
        assert_eq!(h0.parse_workers.get(), 1);
        assert_eq!(h0.index_writer_heap_bytes, 64 * 1024 * 1024);
        assert_eq!(h0.max_private_or_anonymous_mb, 512);

        let h1 = ImportResourcePolicy::for_hardware(ImportHardwareProfile::new(
            Some(16 * BYTES_PER_GIB),
            10,
        ));
        assert_eq!(h1.hardware_tier, ImportHardwareTier::H1Balanced);
        assert_eq!(h1.parse_workers.get(), 2);
        assert_eq!(h1.index_writer_heap_bytes, 128 * 1024 * 1024);
        assert_eq!(h1.max_private_or_anonymous_mb, 1024);

        let h2 = ImportResourcePolicy::for_hardware(ImportHardwareProfile::new(
            Some(32 * BYTES_PER_GIB),
            10,
        ));
        assert_eq!(h2.hardware_tier, ImportHardwareTier::H2Aggressive);
        assert_eq!(h2.parse_workers.get(), 3);
        assert_eq!(h2.index_writer_heap_bytes, 256 * 1024 * 1024);
        assert_eq!(h2.max_private_or_anonymous_mb, 1536);

        let high_memory_single_core = ImportResourcePolicy::for_hardware(
            ImportHardwareProfile::new(Some(32 * BYTES_PER_GIB), 1),
        );
        assert_eq!(
            high_memory_single_core.hardware_tier,
            ImportHardwareTier::H2Aggressive
        );
        assert_eq!(high_memory_single_core.parse_workers.get(), 1);

        let h2_options = ImportOptions::for_resource_policy(h2);
        assert_eq!(h2_options.index_writer_heap_bytes, 256 * 1024 * 1024);
    }

    #[test]
    fn import_resource_policy_uses_inclusive_12_and_20_gib_boundaries() {
        for (total_memory_bytes, expected_tier) in [
            (None, ImportHardwareTier::H0Eco),
            (Some(12 * BYTES_PER_GIB), ImportHardwareTier::H0Eco),
            (Some(12 * BYTES_PER_GIB + 1), ImportHardwareTier::H1Balanced),
            (Some(20 * BYTES_PER_GIB), ImportHardwareTier::H1Balanced),
            (
                Some(20 * BYTES_PER_GIB + 1),
                ImportHardwareTier::H2Aggressive,
            ),
        ] {
            let policy = ImportResourcePolicy::for_hardware(ImportHardwareProfile::new(
                total_memory_bytes,
                10,
            ));
            assert_eq!(policy.hardware_tier, expected_tier);
        }
    }

    #[test]
    fn snapshot_publish_phases_map_to_import_cancel_subphase_labels() {
        for (phase, expected_label) in [
            (SnapshotPublishPhase::Setup, "index_publication_setup"),
            (
                SnapshotPublishPhase::DocumentIndexing,
                "index_publication_documents",
            ),
            (
                SnapshotPublishPhase::TantivyCommit,
                "index_publication_commit",
            ),
            (
                SnapshotPublishPhase::PlaintextValidation,
                "index_publication_plaintext_validation",
            ),
            (
                SnapshotPublishPhase::EncryptedPublication,
                "index_publication_encrypted_publication",
            ),
            (
                SnapshotPublishPhase::EncryptedValidation,
                "index_publication_encrypted_validation",
            ),
        ] {
            assert_eq!(
                ImportCancelCheckPhase::from_snapshot_publish_phase(phase).as_label(),
                expected_label
            );
        }
    }

    #[test]
    fn import_root_persists_clean_text_without_duplicate_raw_text_body() {
        let temp = TestDir::new("import-pipeline-no-duplicate-raw-text");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust Search"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_075);
        let task = import_task("no-duplicate-raw-text-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.searchable_documents, 1);
        let document = store.visible_documents().unwrap().remove(0);
        let version = active_resume_version(&store, &document).unwrap();
        assert!(version
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Rust Search"));
        assert_eq!(version.raw_text, None);
    }

    #[test]
    fn import_root_sanitizes_embedded_control_bytes_before_version_identity() {
        let temp = TestDir::new("import-pipeline-control-byte-normalization");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        let source = synthetic_resume_text("Synthetic Candidate", "Rust\0Search");
        fs::write(root.join("synthetic-resume.txt"), source).unwrap();

        let store = create_test_store(&data_dir);
        let now = UnixTimestamp::from_unix_seconds(1_700_000_076);
        let task = import_task("control-byte-normalization", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.searchable_documents, 1);
        let document = store.visible_documents().unwrap().remove(0);
        let version = active_resume_version(&store, &document).unwrap();
        let clean_text = version.clean_text.as_deref().unwrap();
        assert!(clean_text.contains("Rust Search"));
        assert!(clean_text
            .chars()
            .all(|character| !character.is_control() || character == '\n'));
        assert_eq!(
            version.normalized_text_hash,
            ContentDigest::from_bytes(clean_text.as_bytes())
        );
    }

    #[test]
    fn classifier_gate_persists_all_five_states_before_search_admission() {
        let temp = TestDir::new("classifier-gate-five-states");
        let root = temp.path().join("mixed");
        fs::create_dir_all(&root).unwrap();
        for (name, body) in [
            (
                "resume.txt",
                synthetic_resume_text("Synthetic Candidate", "Rust Search"),
            ),
            (
                "invoice.txt",
                "INVOICE\nInvoice number 7\nSubtotal 10\nPayment terms net 30".to_string(),
            ),
            (
                "review.txt",
                "Project notes\nUnstructured material".to_string(),
            ),
            ("empty.txt", String::new()),
        ] {
            fs::write(root.join(name), body).unwrap();
        }
        fs::write(root.join("scan.pdf"), scanned_pdf_bytes()).unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_078);
        for workers in [1, 2] {
            let data_dir = temp.path().join(format!("data-{workers}"));
            fs::create_dir_all(&data_dir).unwrap();
            let store = create_test_store(&data_dir);
            store.run_migrations().unwrap();
            let task = import_task(&format!("gate-{workers}"), root.to_str().unwrap(), now);
            insert_test_import_task(&store, &task, &ImportOptions::default());
            let options = ImportOptions::low_memory_default_for_available_parallelism(workers);
            import_root_with_options(&data_dir, &store, &task, &root, now, options).unwrap();
            let counts = store.classification_counts(CLASSIFIER_EPOCH).unwrap();
            assert_eq!(
                (
                    counts.resume_candidate,
                    counts.non_resume,
                    counts.needs_review,
                    counts.ocr_backlog,
                    counts.failed
                ),
                (1, 1, 1, 1, 1)
            );
            assert_eq!(store.searchable_document_ids().unwrap().len(), 1);
        }
    }

    #[test]
    fn failed_reparse_stages_failure_without_withdrawing_active_projection() {
        let temp = TestDir::new("failed-reparse-retains-active-projection");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        let path = root.join("candidate.txt");
        fs::write(
            &path,
            synthetic_resume_text("Stable Candidate", "Rust Search"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_210_000);
        let first_task = import_task("failure-retention-first", root.to_str().unwrap(), first_now);
        insert_test_import_task(&store, &first_task, &ImportOptions::default());
        import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let document = store.visible_documents().unwrap().remove(0);
        let active_before = store
            .active_search_projection_for_document(&document.id)
            .unwrap()
            .unwrap();
        let generation_before = store.search_projection_state().unwrap().generation;

        fs::write(&path, []).unwrap();
        let second_now = UnixTimestamp::from_unix_seconds(1_700_210_001);
        let second_task = import_task(
            "failure-retention-second",
            root.to_str().unwrap(),
            second_now,
        );
        insert_test_import_task(&store, &second_task, &ImportOptions::default());
        let summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.failed_documents, 1);
        assert_eq!(
            store
                .active_search_projection_for_document(&document.id)
                .unwrap(),
            Some(active_before.clone())
        );
        assert_eq!(
            store.search_projection_state().unwrap().generation,
            generation_before
        );
        assert!(store
            .resume_version_by_id(&active_before.resume_version_id)
            .unwrap()
            .unwrap()
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Stable Candidate"));
        let failed_revision =
            SourceRevision::for_content(document.id, ContentDigest::from_bytes(&[]), 0);
        assert_eq!(
            store
                .source_revision_triage(&failed_revision.id, CLASSIFIER_EPOCH)
                .unwrap()
                .unwrap()
                .status,
            ClassificationStatus::Failed
        );
    }

    #[test]
    fn parallel_parse_workers_preserve_searchable_and_ocr_counts() {
        let temp = TestDir::new("import-pipeline-parallel-parse-counts");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("00-alpha.txt"),
            synthetic_resume_text("Alpha Candidate", "Rust Search"),
        )
        .unwrap();
        fs::write(root.join("01-scanned.pdf"), scanned_pdf_bytes()).unwrap();
        fs::write(
            root.join("02-beta.txt"),
            synthetic_resume_text("Beta Candidate", "PDF Search"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_078);
        let task = import_task("parallel-parse-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions {
                parse_workers: ImportParseWorkers::new(2),
                ..ImportOptions::default()
            },
        )
        .unwrap();

        assert_eq!(summary.files_discovered, 3);
        assert_eq!(summary.searchable_documents, 2);
        assert_eq!(summary.ocr_required_documents, 1);
        assert_eq!(summary.ocr_jobs_queued, 1);
        assert_eq!(summary.failed_documents, 0);
        assert!(summary.milestone_timings.first_searchable.is_some());
        assert!(summary.milestone_timings.full_import_ready.is_some());

        let status = store.status_summary().unwrap();
        assert_eq!(status.searchable_documents, 2);
        assert_eq!(status.ocr_queue_depth, 1);
        let documents = store.visible_documents().unwrap();
        let mut visible_text = String::new();
        for document in documents {
            if let Some(version) = active_resume_version(&store, &document) {
                visible_text.push_str(version.clean_text.as_deref().unwrap_or_default());
            }
        }
        assert!(visible_text.contains("Alpha Candidate"));
        assert!(visible_text.contains("Beta Candidate"));
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn parallel_parse_workers_record_queue_and_cancel_evidence() {
        let temp = TestDir::new("import-pipeline-parallel-parse-evidence");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("00-alpha.txt"),
            synthetic_resume_text("Alpha Candidate", "Rust Search"),
        )
        .unwrap();
        fs::write(root.join("01-scanned.pdf"), scanned_pdf_bytes()).unwrap();
        fs::write(
            root.join("02-beta.txt"),
            synthetic_resume_text("Beta Candidate", "PDF Search"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_079);
        let task = import_task(
            "parallel-parse-evidence-import",
            root.to_str().unwrap(),
            now,
        );
        insert_test_import_task(&store, &task, &ImportOptions::default());

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions {
                parse_workers: ImportParseWorkers::new(2),
                ..ImportOptions::default()
            },
        )
        .unwrap();

        assert_eq!(summary.worker_metrics.parse_worker_count, 2);
        assert_eq!(summary.worker_metrics.parse_jobs_queued, 2);
        assert!(summary.worker_metrics.parse_prepare > Duration::ZERO);
        assert!(summary.worker_metrics.parse_worker_wall > Duration::ZERO);
        assert!(summary.worker_metrics.parse_worker_active > Duration::ZERO);
        assert!(summary.stage_timings.parse >= summary.worker_metrics.parse_worker_wall);
        assert!(summary.worker_metrics.cancel_check_count > 0);
        assert!(summary.worker_metrics.cancel_check_max_gap >= Duration::ZERO);
        assert_ne!(
            summary.worker_metrics.cancel_check_max_gap_phase,
            ImportCancelCheckPhase::Unattributed
        );
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn parallel_parse_workers_record_pdf_and_post_parser_phase_timings() {
        let temp = TestDir::new("import-pipeline-parse-phase-evidence");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("00-scanned.pdf"), scanned_pdf_bytes()).unwrap();
        fs::write(root.join("01-alpha.pdf"), tounicode_cmap_pdf_bytes()).unwrap();
        fs::write(
            root.join("02-beta.txt"),
            synthetic_resume_text("Beta Candidate", "Rust Search"),
        )
        .unwrap();
        fs::write(
            root.join("03-gamma.txt"),
            synthetic_resume_text("Gamma Candidate", "PDF Search"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_080);
        let task = import_task("parse-phase-evidence-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions {
                parse_workers: ImportParseWorkers::new(2),
                ..ImportOptions::default()
            },
        )
        .unwrap();

        assert_eq!(summary.worker_metrics.parse_worker_count, 2);
        assert_eq!(summary.searchable_documents, 3);
        for (label, elapsed) in [
            (
                "document_load",
                summary.worker_metrics.pdf_parse_timings.document_load,
            ),
            (
                "page_content_fetch",
                summary.worker_metrics.pdf_parse_timings.page_content_fetch,
            ),
            (
                "text_operator_prefilter",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .text_operator_prefilter,
            ),
            (
                "font_encoding",
                summary.worker_metrics.pdf_parse_timings.font_encoding,
            ),
            (
                "content_decode",
                summary.worker_metrics.pdf_parse_timings.content_decode,
            ),
            (
                "content_string_parse",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .content_string_parse,
            ),
            (
                "text_collection",
                summary.worker_metrics.pdf_parse_timings.text_collection,
            ),
            (
                "text_byte_decode",
                summary.worker_metrics.pdf_parse_timings.text_byte_decode,
            ),
            (
                "text_accumulation",
                summary.worker_metrics.pdf_parse_timings.text_accumulation,
            ),
            (
                "normalization",
                summary.worker_metrics.post_parser_timings.normalization,
            ),
            (
                "sectionization",
                summary.worker_metrics.post_parser_timings.sectionization,
            ),
        ] {
            assert!(
                elapsed > Duration::ZERO,
                "{label} timing should be recorded: {summary:?}"
            );
        }
        for (label, count) in [
            (
                "content_string_operands",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .content_string_operands,
            ),
            (
                "content_string_bytes",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .content_string_bytes,
            ),
            (
                "text_decode_runs",
                summary.worker_metrics.pdf_parse_timings.text_decode_runs,
            ),
            (
                "text_decode_input_bytes",
                summary
                    .worker_metrics
                    .pdf_parse_timings
                    .text_decode_input_bytes,
            ),
        ] {
            assert!(count > 0, "{label} counter should be recorded: {summary:?}");
        }
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn parse_worker_clock_reports_wall_clock_separate_from_active_sum() {
        let temp = TestDir::new("import-pipeline-parse-worker-clock");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("clock.txt"),
            synthetic_resume_text("Clock Candidate", "Rust Search"),
        )
        .unwrap();
        let file = crawl_directory(&root).unwrap().files.remove(0);
        let document = test_document("clock", DocumentStatus::Searchable);
        let source_revision = test_source_revision(&document);
        let started = Instant::now();
        let mut clock = super::ParseWorkerClock::default();

        clock.record_result(&ParseWorkResult {
            index: 0,
            file: file.clone(),
            document: document.clone(),
            source_revision: source_revision.clone(),
            parse_elapsed: Duration::from_millis(100),
            parse_started: started,
            parse_finished: started + Duration::from_millis(100),
            pdf_parse_timings: parser_pdf::PdfTextExtractionTimings::default(),
            post_parser_timings: crate::ImportPostParserTimings::default(),
            outcome: ParseWorkOutcome::OcrRequired,
        });
        clock.record_result(&ParseWorkResult {
            index: 1,
            file,
            document,
            source_revision,
            parse_elapsed: Duration::from_millis(100),
            parse_started: started + Duration::from_millis(10),
            parse_finished: started + Duration::from_millis(110),
            pdf_parse_timings: parser_pdf::PdfTextExtractionTimings::default(),
            post_parser_timings: crate::ImportPostParserTimings::default(),
            outcome: ParseWorkOutcome::OcrRequired,
        });

        assert_eq!(clock.active_elapsed(), Duration::from_millis(200));
        assert_eq!(clock.worker_wall_elapsed(), Duration::from_millis(110));
    }

    #[test]
    fn cancel_check_max_gap_is_attributed_to_previous_phase() {
        let mut metrics = super::CancelCheckMetrics::default();

        metrics.record_check(ImportCancelCheckPhase::SequentialParse);
        thread::sleep(Duration::from_millis(2));
        metrics.record_check(ImportCancelCheckPhase::DbWrite);

        assert_eq!(metrics.count, 2);
        assert_eq!(
            metrics.max_gap_phase,
            ImportCancelCheckPhase::SequentialParse
        );
    }

    #[test]
    fn cancel_poller_reuses_cached_state_within_interval() {
        let started = Instant::now();
        let mut poller = super::ImportCancelPoller::new(Duration::from_millis(25));
        let probes = AtomicUsize::new(0);

        assert!(!poller
            .poll(started, || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(false)
            })
            .unwrap());
        assert!(!poller
            .poll(started + Duration::from_millis(10), || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            })
            .unwrap());
        assert!(poller
            .poll(started + Duration::from_millis(30), || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            })
            .unwrap());

        assert_eq!(probes.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn cancel_poller_keeps_cancelled_state_without_requery() {
        let started = Instant::now();
        let mut poller = super::ImportCancelPoller::new(Duration::from_millis(25));
        let probes = AtomicUsize::new(0);

        assert!(poller
            .poll(started, || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            })
            .unwrap());
        assert!(poller
            .poll(started + Duration::from_millis(30), || {
                probes.fetch_add(1, Ordering::Relaxed);
                Ok(false)
            })
            .unwrap());

        assert_eq!(probes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn recv_parse_result_polls_cancel_while_waiting() {
        let temp = TestDir::new("import-pipeline-parse-result-cancel-poll");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("wait.txt"),
            synthetic_resume_text("Wait Candidate", "Rust Search"),
        )
        .unwrap();
        let file = crawl_directory(&root).unwrap().files.remove(0);
        let document = test_document("wait", DocumentStatus::Searchable);
        let source_revision = test_source_revision(&document);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let cancel_polls = Arc::new(AtomicUsize::new(0));
        let observed_cancel_polls = Arc::clone(&cancel_polls);
        let sender = thread::spawn(move || {
            release_rx.recv().unwrap();
            let parse_started = Instant::now();
            result_tx
                .send(ParseWorkResult {
                    index: 7,
                    file,
                    document,
                    source_revision,
                    parse_elapsed: Duration::from_millis(1),
                    parse_started,
                    parse_finished: parse_started + Duration::from_millis(1),
                    pdf_parse_timings: parser_pdf::PdfTextExtractionTimings::default(),
                    post_parser_timings: crate::ImportPostParserTimings::default(),
                    outcome: ParseWorkOutcome::OcrRequired,
                })
                .unwrap();
        });

        let result = recv_parse_result_with_cancel_poll(&result_rx, &|| {
            let poll = observed_cancel_polls.fetch_add(1, Ordering::SeqCst) + 1;
            if poll == 2 {
                release_tx.send(()).unwrap();
            }
            Ok(())
        })
        .unwrap();
        sender.join().unwrap();

        assert_eq!(result.index, 7);
        assert!(
            cancel_polls.load(Ordering::SeqCst) >= 2,
            "expected repeated cancellation checks while waiting for parse result"
        );
    }

    #[test]
    fn index_ocr_text_persists_clean_text_without_duplicate_raw_text_body() {
        let temp = TestDir::new("import-pipeline-ocr-no-duplicate-raw-text");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_074),
        );
        let document = test_document("ocr-doc", DocumentStatus::OcrRequired);
        let stale = claim_ocr_document(
            &store,
            &document,
            UnixTimestamp::from_unix_seconds(1_700_000_075),
        );
        store
            .finish_ocr_attempt_failure(
                &stale,
                OcrAttemptFailure::Retryable,
                UnixTimestamp::from_unix_seconds(1_700_000_075),
            )
            .unwrap();
        let current = store
            .claim_next_ocr_job(UnixTimestamp::from_unix_seconds(1_700_000_076))
            .unwrap()
            .unwrap();
        let vectorization =
            SearchPublicationVectorization::enabled(Arc::new(TestPublicationVectorizer {
                fail: false,
            }));
        assert_eq!(
            index_claimed_ocr_text(
                &data_dir,
                &store,
                &stale,
                "stale OCR output",
                Some(0.99),
                Some(1),
                UnixTimestamp::from_unix_seconds(1_700_000_076),
                &vectorization,
            )
            .unwrap(),
            OcrTextIndexOutcome::Superseded
        );

        let OcrTextIndexOutcome::Committed(summary) = index_claimed_ocr_text(
            &data_dir,
            &store,
            &current,
            &synthetic_resume_text("OCR Candidate", "Rust Search"),
            Some(0.91),
            Some(1),
            UnixTimestamp::from_unix_seconds(1_700_000_076),
            &vectorization,
        )
        .unwrap() else {
            panic!("current OCR attempt was superseded");
        };

        assert!(summary.searchable);
        let version = active_resume_version(&store, &document).unwrap();
        assert!(version
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Rust Search"));
        assert_eq!(version.raw_text, None);
        assert_vector_generation(&data_dir, &store, 1);
    }

    #[test]
    fn migration_rebuild_defers_ocr_publication_until_exact_root_barrier_is_ready() {
        let temp = TestDir::new("import-pipeline-migration-ocr-barrier");
        let data_dir = temp.path().join("data");
        let root_a = temp.path().join("root-a");
        let root_b = temp.path().join("root-b");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root_a).unwrap();
        fs::create_dir_all(&root_b).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_500);
        let task_a = import_task("migration-ocr-root-a", root_a.to_str().unwrap(), now);
        let task_b = import_task("migration-ocr-root-b", root_b.to_str().unwrap(), now);
        let scope_a = import_scan_scope(&task_a, None);
        let scope_b = import_scan_scope(&task_b, None);
        let contract = insert_test_import_task_with_scope(
            &store,
            &task_a,
            &scope_a,
            &ImportOptions::default(),
        );
        insert_test_import_task_with_scope(&store, &task_b, &scope_b, &ImportOptions::default());
        store
            .complete_import_task(
                &task_a.id,
                contract.id(),
                &scope_a,
                UnixTimestamp::from_unix_seconds(1_700_000_501),
            )
            .unwrap();

        let document = test_document("migration-ocr-doc", DocumentStatus::OcrRequired);
        let claimed = claim_ocr_document(
            &store,
            &document,
            UnixTimestamp::from_unix_seconds(1_700_000_502),
        );
        let error = index_claimed_ocr_text(
            &data_dir,
            &store,
            &claimed,
            &synthetic_resume_text("Migration OCR Candidate", "Rust Search"),
            Some(0.94),
            Some(1),
            UnixTimestamp::from_unix_seconds(1_700_000_503),
            &SearchPublicationVectorization::default(),
        )
        .unwrap_err();

        assert_eq!(error.class(), ImportPipelineErrorClass::Repairing);
        assert!(error.is_retryable());
        let deferred_document = store.document_by_id(&document.id).unwrap().unwrap();
        assert_eq!(deferred_document.status, DocumentStatus::OcrRequired);
        assert!(active_resume_version(&store, &document).is_none());
        assert_eq!(
            store
                .ingest_job_by_id(&claimed.job.id)
                .unwrap()
                .unwrap()
                .status,
            IngestJobStatus::Running
        );
        let staging_state = store.search_projection_state().unwrap();
        assert_eq!(
            staging_state.service_state,
            SearchProjectionServiceState::Repairing
        );
        assert_eq!(
            staging_state.repair_reason,
            Some(SearchRepairReason::MigrationRebuild)
        );
        assert_eq!(staging_state.generation, None);
        let inherited_epoch = staging_state.visible_epoch;
        assert!(store.searchable_document_ids().unwrap().is_empty());

        store
            .complete_import_task(
                &task_b.id,
                contract.id(),
                &scope_b,
                UnixTimestamp::from_unix_seconds(1_700_000_504),
            )
            .unwrap();
        let finalized = super::finalize_migration_rebuild(
            &store,
            UnixTimestamp::from_unix_seconds(1_700_000_505),
            &contract,
            &SearchPublicationVectorization::default(),
        )
        .unwrap();

        assert!(finalized.active_generation_rebuilt);
        let ready_state = store.search_projection_state().unwrap();
        assert_eq!(
            ready_state.service_state,
            SearchProjectionServiceState::Ready
        );
        assert_eq!(ready_state.repair_reason, None);
        assert!(ready_state.generation.is_some());
        assert_eq!(
            ready_state.visible_epoch,
            inherited_epoch.checked_add(1).unwrap()
        );
        assert!(active_resume_version(&store, &document).is_none());

        let OcrTextIndexOutcome::Committed(summary) = index_claimed_ocr_text(
            &data_dir,
            &store,
            &claimed,
            &synthetic_resume_text("Migration OCR Candidate", "Rust Search"),
            Some(0.94),
            Some(1),
            UnixTimestamp::from_unix_seconds(1_700_000_506),
            &SearchPublicationVectorization::default(),
        )
        .unwrap() else {
            panic!("deferred OCR must publish after the exact root barrier is ready");
        };
        assert!(summary.searchable);
        assert!(active_resume_version(&store, &document).is_some());
        assert_eq!(
            store.document_by_id(&document.id).unwrap().unwrap().status,
            DocumentStatus::Searchable
        );
    }

    #[test]
    fn migration_publication_session_rejects_concurrent_ocr_then_retry_preserves_the_version() {
        let temp = TestDir::new("import-pipeline-migration-ocr-finalizer-race");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("root");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        let store = create_test_store(&data_dir);
        let now = UnixTimestamp::from_unix_seconds(1_700_000_520);
        let task = import_task("migration-ocr-race-root", root.to_str().unwrap(), now);
        let scope = import_scan_scope(&task, None);
        let contract =
            insert_test_import_task_with_scope(&store, &task, &scope, &ImportOptions::default());
        store
            .complete_import_task(
                &task.id,
                contract.id(),
                &scope,
                UnixTimestamp::from_unix_seconds(1_700_000_521),
            )
            .unwrap();
        let document = test_document("migration-ocr-race-doc", DocumentStatus::OcrRequired);
        let claimed = claim_ocr_document(
            &store,
            &document,
            UnixTimestamp::from_unix_seconds(1_700_000_522),
        );
        let publisher_store = store.open_sibling().unwrap();
        let publisher_contract_id = contract.id().clone();
        let (publisher_ready_sender, publisher_ready_receiver) = mpsc::sync_channel(1);
        let (publisher_release_sender, publisher_release_receiver) = mpsc::sync_channel(1);
        let publisher = thread::spawn(move || {
            let publication_session = publisher_store
                .wait_for_search_publication_session()
                .unwrap();
            publisher_ready_sender.send(()).unwrap();
            publisher_release_receiver.recv().unwrap();
            let barrier = publisher_store
                .acquire_migration_rebuild_barrier_token(&publisher_contract_id)
                .unwrap()
                .unwrap();
            let staged = super::search_artifacts::migration_index_documents_from_exact_projection(
                publisher_store
                    .migration_rebuild_projection_rows(&barrier)
                    .unwrap(),
            )
            .unwrap();
            assert!(staged.is_empty());
            let publication = super::search_artifacts::write_migration_rebuild_search_artifacts(
                &publication_session,
                UnixTimestamp::from_unix_seconds(1_700_000_524),
                CLASSIFIER_EPOCH,
                &BTreeSet::new(),
                Vec::new(),
                &SearchPublicationVectorization::default(),
            )
            .unwrap();
            super::search_publication::commit_migration_rebuild_search_publication(
                UnixTimestamp::from_unix_seconds(1_700_000_524),
                publication,
                &[],
                &barrier,
            )
            .unwrap()
            .expect("unchanged migration barrier must commit")
            .release();
        });
        publisher_ready_receiver.recv().unwrap();
        let concurrent_error = index_claimed_ocr_text(
            &data_dir,
            &store,
            &claimed,
            &synthetic_resume_text("Concurrent OCR Candidate", "Rust Search"),
            Some(0.95),
            Some(1),
            UnixTimestamp::from_unix_seconds(1_700_000_523),
            &SearchPublicationVectorization::default(),
        )
        .unwrap_err();
        assert_eq!(
            concurrent_error.metadata_class_label(),
            Some("migration_ownership_required")
        );
        publisher_release_sender.send(()).unwrap();
        publisher.join().unwrap();

        let outcome = index_claimed_ocr_text(
            &data_dir,
            &store,
            &claimed,
            &synthetic_resume_text("Concurrent OCR Candidate", "Rust Search"),
            Some(0.95),
            Some(1),
            UnixTimestamp::from_unix_seconds(1_700_000_525),
            &SearchPublicationVectorization::default(),
        )
        .unwrap();
        let OcrTextIndexOutcome::Committed(summary) = outcome else {
            panic!("OCR retried after migration publication must commit against the ready head");
        };
        assert!(summary.searchable);
        assert_eq!(summary.indexed_documents, 1);
        let state = store.search_projection_state().unwrap();
        assert_eq!(state.service_state, SearchProjectionServiceState::Ready);
        assert_eq!(state.visible_epoch, 2);
        assert!(active_resume_version(&store, &document).is_some());
    }

    fn test_pending_searchable_document(doc_id: &str) -> PendingSearchableDocument {
        let mut document = test_document(doc_id, DocumentStatus::TextCleaned);
        let source_bytes = format!("source bytes for {doc_id}");
        let source_revision = SourceRevision::for_content(
            document.id.clone(),
            ContentDigest::from_bytes(source_bytes.as_bytes()),
            source_bytes.len() as u64,
        );
        document.content_hash = Some(source_revision.content_hash.as_str().to_string());
        document.byte_size = source_revision.byte_size;
        let clean_text = format!("Synthetic Candidate {doc_id}\\nSkills: Rust Search");
        let version = super::resume_version(
            &document,
            &source_revision,
            clean_text,
            "parser-v1",
            super::SCHEMA_VERSION,
            vec!["en".to_string()],
            Some(1),
            Some(0.8),
        );
        let classification = ResumeVersionClassification {
            resume_version_id: version.id.clone(),
            status: ClassificationStatus::ResumeCandidate,
            classifier_epoch: CLASSIFIER_EPOCH.to_string(),
            reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
            classified_at: document.updated_at,
            review_disposition: ReviewDisposition::NotRequired,
        };
        let index_document = IndexDocument {
            doc_id: document.id.to_string(),
            resume_version_id: version.id.to_string(),
            file_name: format!("{doc_id}.txt"),
            clean_text: version.clean_text.clone().unwrap(),
            sections: Vec::new(),
        };
        PendingSearchableDocument {
            document,
            source_revision,
            classification,
            version,
            mentions: Vec::new(),
            email_hash: None,
            phone_hash: None,
            index_document,
            publication_kind: PendingSearchablePublicationKind::Replacement,
        }
    }

    fn test_entity_mention(
        id: EntityMentionId,
        resume_version_id: ResumeVersionId,
    ) -> EntityMention {
        EntityMention {
            id,
            resume_version_id,
            section_id: None,
            entity_type: EntityType::Skill,
            raw_value: "Rust".to_string(),
            normalized_value: Some("Rust".to_string()),
            span_start: Some(0),
            span_end: Some(4),
            confidence: 0.9,
            extractor: "rules-v1".to_string(),
        }
    }

    fn stage_test_index_document(store: &OwnedMetaStore, doc_id: &str) -> IndexDocument {
        let pending = test_pending_searchable_document(doc_id);
        super::immutable_ingest::stage(
            store,
            super::StagedResume {
                document: &pending.document,
                source_revision: &pending.source_revision,
                derived: super::StagedDerivedData::ClassifiedVersion {
                    version: &pending.version,
                    classification: &pending.classification,
                    mentions: &pending.mentions,
                    email_hash: None,
                    phone_hash: None,
                },
            },
        )
        .unwrap();
        let mut index_document = pending.index_document;
        index_document.sections = vec![IndexSection {
            section_type: "skills".to_string(),
            text: format!("Rust Search section for {doc_id}"),
        }];
        index_document
    }

    fn terminal_searchable_document(
        store: &OwnedMetaStore,
        doc_id: &str,
        now: UnixTimestamp,
    ) -> Document {
        let document_id = DocumentId::from_non_secret_parts(&[doc_id]);
        let mut document = store.document_by_id(&document_id).unwrap().unwrap();
        document.status = DocumentStatus::Searchable;
        document.updated_at = now;
        document
    }

    fn retained_section_text_bytes(documents: &CurrentImportDocumentCache) -> usize {
        documents
            .documents
            .iter()
            .flat_map(|document| document.sections.iter())
            .map(|section| section.text.len())
            .sum()
    }

    fn test_document(doc_id: &str, status: DocumentStatus) -> Document {
        let content_hash = ContentDigest::from_bytes(doc_id.as_bytes());
        Document {
            id: DocumentId::from_non_secret_parts(&[doc_id]),
            source_uri: format!("file:///fixture/{doc_id}.txt"),
            normalized_path: format!("/fixture/{doc_id}.txt"),
            file_name: format!("{doc_id}.txt"),
            extension: FileExtension::Txt,
            byte_size: 128,
            mtime: UnixTimestamp::from_unix_seconds(1_700_000_001),
            content_hash: Some(content_hash.as_str().to_string()),
            text_hash: None,
            is_deleted: false,
            created_at: UnixTimestamp::from_unix_seconds(1_700_000_000),
            updated_at: UnixTimestamp::from_unix_seconds(1_700_000_000),
            status,
        }
    }

    fn synthetic_resume_text(candidate: &str, skills: &str) -> String {
        format!("SUMMARY\n{candidate}\nEXPERIENCE\nBuilt {skills} systems\nSKILLS\n{skills}")
    }

    fn normalized_path(path: &str) -> NormalizedPath {
        normalize_path(path).unwrap()
    }

    fn import_scan_scope(task: &ImportTask, max_files: Option<u64>) -> ImportScanScope {
        ImportScanScope {
            import_task_id: task.id.clone(),
            root_kind: ImportRootKind::Explicit,
            root_preset: None,
            scan_profile: ImportScanProfile::Explicit,
            requested_root_path: task.root_path.clone(),
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
            scan_budget_limit: max_files,
            scan_budget_observed: max_files.map(|_| 0),
            scan_budget_exhausted: false,
            updated_at: task.updated_at,
        }
    }

    #[test]
    fn migration_rebuild_budget_exhaustion_blocks_without_partial_publication() {
        let temp = TestDir::new("import-pipeline-migration-budget-block");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("a-resume.txt"),
            synthetic_resume_text("Budget Candidate A", "Rust Search"),
        )
        .unwrap();
        fs::write(
            root.join("b-resume.txt"),
            synthetic_resume_text("Budget Candidate B", "Rust Search"),
        )
        .unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_600);
        let task = import_task("migration-budget-block", root.to_str().unwrap(), now);
        let scope = import_scan_scope(&task, Some(1));
        insert_test_import_task_with_scope(
            &store,
            &task,
            &scope,
            &ImportOptions {
                max_files: Some(1),
                ..ImportOptions::default()
            },
        );

        let error = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions {
                max_files: Some(1),
                ..ImportOptions::default()
            },
        )
        .unwrap_err();

        assert_eq!(error.class(), ImportPipelineErrorClass::SourceUnavailable);
        assert!(error.is_retryable());
        assert_eq!(
            store.import_task_by_id(&task.id).unwrap().unwrap().status,
            ImportTaskStatus::FailedRetryable
        );
        let scope = store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        assert!(scope.scan_budget_exhausted);
        let state = store.search_projection_state().unwrap();
        assert_eq!(
            state.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::SourceUnavailable)
        );
        assert_eq!(state.generation, None);
        assert_eq!(state.visible_epoch, 0);
        assert!(store.searchable_document_ids().unwrap().is_empty());
        assert!(!data_dir.join("search-index").join("active").exists());
    }

    #[test]
    fn ready_search_keeps_existing_partial_scan_semantics() {
        let temp = TestDir::new("import-pipeline-ready-budget-retained-semantics");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("a-resume.txt"),
            synthetic_resume_text("Ready Candidate A", "Rust Search"),
        )
        .unwrap();
        fs::write(
            root.join("b-resume.txt"),
            synthetic_resume_text("Ready Candidate B", "Rust Search"),
        )
        .unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_605);
        initialize_ready_empty_search(&data_dir, &store, now);
        let task = import_task("ready-budget-import", root.to_str().unwrap(), now);
        let scope = import_scan_scope(&task, Some(1));
        insert_test_import_task_with_scope(
            &store,
            &task,
            &scope,
            &ImportOptions {
                max_files: Some(1),
                ..ImportOptions::default()
            },
        );

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions {
                max_files: Some(1),
                ..ImportOptions::default()
            },
        )
        .unwrap();

        assert!(summary.scan_budget.unwrap().exhausted);
        assert_eq!(summary.searchable_documents, 1);
        assert_eq!(
            store.import_task_by_id(&task.id).unwrap().unwrap().status,
            ImportTaskStatus::Completed
        );
        let state = store.search_projection_state().unwrap();
        assert_eq!(state.service_state, SearchProjectionServiceState::Ready);
        assert_eq!(state.repair_reason, None);
        assert_eq!(store.searchable_document_ids().unwrap().len(), 1);
    }

    #[test]
    fn migration_empty_base_is_unavailable_to_normal_incremental_and_rebuild_paths() {
        let temp = TestDir::new("import-pipeline-migration-base-hard-cut");
        let data_dir = temp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_607);
        let publication_session = store.wait_for_search_publication_session().unwrap();

        let incremental_result = write_incremental_search_artifacts(
            &publication_session,
            now,
            CLASSIFIER_EPOCH,
            Vec::new(),
            &BTreeSet::new(),
            0,
            0,
            None,
            CurrentImportCacheMode::Retain,
            None,
            None,
            None,
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
        );
        let Err(incremental_error) = incremental_result else {
            panic!("normal incremental publication must reject the migration empty base");
        };
        assert_eq!(
            incremental_error.class(),
            ImportPipelineErrorClass::MetadataInvariant
        );

        let rebuild_result = super::search_artifacts::write_rebuilt_search_artifacts(
            &publication_session,
            now,
            CLASSIFIER_EPOCH,
            &BTreeSet::new(),
            Vec::new(),
            &SearchPublicationVectorization::default(),
        );
        let Err(rebuild_error) = rebuild_result else {
            panic!("normal rebuild publication must reject the migration empty base");
        };
        assert_eq!(
            rebuild_error.class(),
            ImportPipelineErrorClass::MetadataInvariant
        );
        let state = store.search_projection_state().unwrap();
        assert_eq!(state.service_state, SearchProjectionServiceState::Repairing);
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::MigrationRebuild)
        );
        assert_eq!(state.generation, None);
        assert_eq!(state.visible_epoch, 0);
    }

    #[cfg(unix)]
    #[test]
    fn migration_rebuild_partial_scan_error_blocks_without_staging_readable_subset() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TestDir::new("import-pipeline-migration-scan-error-block");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        let unreadable = root.join("unreadable");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&unreadable).unwrap();
        fs::write(
            root.join("readable-resume.txt"),
            synthetic_resume_text("Readable Candidate", "Rust Search"),
        )
        .unwrap();
        fs::write(
            unreadable.join("hidden-resume.txt"),
            synthetic_resume_text("Hidden Candidate", "Rust Search"),
        )
        .unwrap();
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_610);
        let task = import_task("migration-scan-error-block", root.to_str().unwrap(), now);
        let scope = import_scan_scope(&task, None);
        insert_test_import_task_with_scope(&store, &task, &scope, &ImportOptions::default());

        let result = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        );
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o700)).unwrap();
        let error = result.unwrap_err();

        assert_eq!(error.class(), ImportPipelineErrorClass::SourceUnavailable);
        assert!(error.is_retryable());
        assert_eq!(
            store.import_task_by_id(&task.id).unwrap().unwrap().status,
            ImportTaskStatus::FailedRetryable
        );
        let scope = store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        assert_eq!(scope.files_discovered, 1);
        assert_eq!(scope.scan_errors, 1);
        let state = store.search_projection_state().unwrap();
        assert_eq!(
            state.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::SourceUnavailable)
        );
        assert_eq!(state.generation, None);
        assert_eq!(state.visible_epoch, 0);
        assert_eq!(store.visible_document_count().unwrap(), 0);
        assert!(store.searchable_document_ids().unwrap().is_empty());
    }

    #[test]
    fn migration_rebuild_blocks_when_a_discovered_source_disappears_before_read() {
        let temp = TestDir::new("import-pipeline-migration-post-scan-read-error");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        let source = root.join("candidate.txt");
        fs::write(
            &source,
            synthetic_resume_text("Post Scan Candidate", "Rust Search"),
        )
        .unwrap();
        let mut report = crawl_directory(&root).unwrap();
        assert!(report.errors.is_empty());
        assert_eq!(report.files.len(), 1);
        let discovered = report.files.remove(0);
        fs::remove_file(&source).unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_611);
        let task = import_task(
            "migration-post-scan-read-error",
            root.to_str().unwrap(),
            now,
        );
        let contract = insert_test_import_task(&store, &task, &ImportOptions::default());

        let mut stage_timings = ImportStageTimings::default();
        let mut worker_metrics = ImportWorkerMetrics::default();
        let mut content_bytes_read = 0;
        let processed = process_file(
            &data_dir,
            &store,
            &discovered,
            &Sectionizer::default(),
            now,
            &|| Ok(()),
            &mut stage_timings,
            &mut worker_metrics,
            &mut content_bytes_read,
            &LinearPromotionPolicy::default(),
        )
        .unwrap();
        assert!(matches!(
            &processed,
            ProcessedFile::Failed {
                kind: ImportFailureKind::ReadError,
                ..
            }
        ));

        let mut summary = ImportSummary {
            files_discovered: 1,
            ..ImportSummary::default()
        };
        let mut disposition_batches =
            super::ImportDispositionBatches::new(task.id.clone(), contract.id().clone());
        let error = finish_import_file(
            &store,
            &task.id,
            now,
            &|| Ok(()),
            &mut summary,
            &mut Vec::new(),
            &mut PendingProjectionRemovals::default(),
            &mut disposition_batches,
            &mut CurrentImportDocumentCache::default(),
            &|_| {},
            Instant::now(),
            H2_INDEX_WRITER_HEAP_BYTES,
            &SearchPublicationVectorization::default(),
            0,
            1,
            &discovered,
            processed,
        )
        .unwrap_err();

        assert_eq!(error.class(), ImportPipelineErrorClass::SourceUnavailable);
        assert!(error.is_retryable());
        assert_eq!(
            store
                .import_scan_scope_by_task_id(&task.id)
                .unwrap()
                .unwrap()
                .failed_documents,
            1
        );
        let pre_failure_state = store.search_projection_state().unwrap();
        assert_eq!(
            pre_failure_state.service_state,
            SearchProjectionServiceState::Repairing
        );
        assert_eq!(
            pre_failure_state.repair_reason,
            Some(SearchRepairReason::MigrationRebuild)
        );
        store
            .fail_observed_import_task(
                &task,
                ImportTaskFailure::Retryable,
                Some(SearchRepairReason::SourceUnavailable),
                UnixTimestamp::from_unix_seconds(1_700_000_612),
            )
            .unwrap();
        let state = store.search_projection_state().unwrap();
        assert_eq!(
            state.service_state,
            SearchProjectionServiceState::RepairBlocked
        );
        assert_eq!(
            state.repair_reason,
            Some(SearchRepairReason::SourceUnavailable)
        );
        assert_eq!(
            store.import_task_by_id(&task.id).unwrap().unwrap().status,
            ImportTaskStatus::FailedRetryable
        );
        assert!(state.generation.is_none());
        assert!(store.searchable_document_ids().unwrap().is_empty());
    }

    #[test]
    fn import_root_reports_shutdown_as_retryable_interruption() {
        let temp = TestDir::new("import-pipeline-shutdown-running");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_000);
        let task = import_task("shutdown-interrupted-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());
        let control = ImportRunControl::default();
        control.request_shutdown();

        let error = import_root_with_options_and_control(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
            control,
        )
        .unwrap_err();

        assert_eq!(error.class(), ImportPipelineErrorClass::Interrupted);
        assert!(error.retryable);
        let stored_task = store.import_task_by_id(&task.id).unwrap().unwrap();
        assert_eq!(stored_task.status, ImportTaskStatus::FailedRetryable);
        assert_eq!(store.status_summary().unwrap().searchable_documents, 0);
        assert!(!data_dir.join("search-index").join("active").exists());
    }

    #[test]
    fn import_root_requires_the_exact_persisted_running_claim() {
        let temp = TestDir::new("import-pipeline-exact-running-claim");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_000);
        let queued = ImportTask {
            status: ImportTaskStatus::Queued,
            started_at: None,
            ..import_task("unclaimed-import", root.to_str().unwrap(), now)
        };
        let options = ImportOptions::default();
        let contract = super::current_import_processing_contract(&options).unwrap();
        store
            .activate_migration_rebuild_contract(&contract, now)
            .unwrap();
        store
            .insert_import_task_with_scan_scope(
                &queued,
                &import_scan_scope(&queued, None),
                &contract,
            )
            .unwrap();
        assert_eq!(
            store.import_task_purpose(&queued.id).unwrap(),
            ImportTaskPurpose::ConfiguredCatchUp
        );

        let queued_error =
            import_root_with_options(&data_dir, &store, &queued, &root, now, options.clone())
                .unwrap_err();
        assert_eq!(
            queued_error.class(),
            ImportPipelineErrorClass::MetadataInvariant
        );
        assert!(!queued_error.is_retryable());
        assert_eq!(
            store.import_task_by_id(&queued.id).unwrap(),
            Some(queued.clone())
        );

        let fabricated_running = ImportTask {
            status: ImportTaskStatus::Running,
            started_at: Some(now),
            ..queued.clone()
        };
        let fabricated_error =
            import_root_with_options(&data_dir, &store, &fabricated_running, &root, now, options)
                .unwrap_err();
        assert_eq!(
            fabricated_error.class(),
            ImportPipelineErrorClass::MetadataInvariant
        );
        assert!(!fabricated_error.is_retryable());
        assert_eq!(store.import_task_by_id(&queued.id).unwrap(), Some(queued));
        assert!(!data_dir.join("search-index").join("active").exists());
    }

    #[test]
    fn import_root_stops_running_task_when_cancellation_marker_exists() {
        let temp = TestDir::new("import-pipeline-cancel-running");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_000);
        let cancel_at = UnixTimestamp::from_unix_seconds(1_700_000_010);
        let task = import_task("running-cancelled-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());
        store.cancel_import_task(&task.id, cancel_at).unwrap();

        let error = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap_err();

        assert_eq!(error.kind, ImportPipelineErrorKind::Cancelled);
        assert_eq!(error.class(), ImportPipelineErrorClass::Cancelled);
        let stored_task = store.import_task_by_id(&task.id).unwrap().unwrap();
        assert_eq!(stored_task.status, ImportTaskStatus::Running);
        assert_eq!(stored_task.updated_at, cancel_at);
        assert!(store.is_import_task_cancelled(&task.id).unwrap());
        assert_eq!(store.status_summary().unwrap().searchable_documents, 0);
        assert!(!data_dir.join("search-index").join("active").exists());
    }

    #[test]
    fn import_root_updates_existing_scan_scope_progress_without_daemon_postprocessing() {
        let temp = TestDir::new("import-pipeline-live-progress");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_100);
        let task = import_task("live-progress-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());
        store
            .upsert_import_scan_scope(&ImportScanScope {
                import_task_id: task.id.clone(),
                root_kind: ImportRootKind::Explicit,
                root_preset: None,
                scan_profile: ImportScanProfile::Explicit,
                requested_root_path: root.to_str().unwrap().to_string(),
                canonical_root_path: root.to_str().unwrap().to_string(),
                files_discovered: 0,
                ignored_entries: 0,
                scan_errors: 0,
                searchable_documents: 0,
                ocr_required_documents: 0,
                ocr_jobs_queued: 0,
                failed_documents: 0,
                deleted_documents: 0,
                scan_budget_kind: None,
                scan_budget_limit: None,
                scan_budget_observed: None,
                scan_budget_exhausted: false,
                updated_at: now,
            })
            .unwrap();

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.files_discovered, 1);
        assert_eq!(summary.searchable_documents, 1);
        let scope = store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        assert_eq!(scope.files_discovered, 1);
        assert_eq!(scope.searchable_documents, 1);
        assert_eq!(scope.scan_budget_observed, None);
        assert!(!format!("{scope:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn import_root_keeps_utf16be_literal_pdf_text_layer_searchable_without_ocr() {
        let temp = TestDir::new("import-pipeline-utf16be-literal-pdf");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("utf16-literal-resume.pdf"),
            utf16be_literal_text_layer_pdf_bytes(),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_150);
        let task = import_task("utf16be-literal-pdf-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        let expected = "\u{4E2D}\u{6587}\u{7B80}\u{5386}";
        assert_eq!(summary.files_discovered, 1);
        assert_eq!(summary.searchable_documents, 1);
        assert_eq!(summary.ocr_required_documents, 0);
        assert_eq!(summary.failed_documents, 0);
        let document = store.visible_documents().unwrap().remove(0);
        let versions = store.resume_versions_for_document(&document.id).unwrap();
        assert_eq!(versions.len(), 1);
        assert!(versions[0]
            .clean_text
            .as_deref()
            .unwrap()
            .contains(expected));
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn import_root_keeps_tounicode_cmap_pdf_text_layer_searchable_without_ocr() {
        let temp = TestDir::new("import-pipeline-tounicode-cmap-pdf");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("tounicode-cmap-resume.pdf"),
            tounicode_cmap_pdf_bytes(),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_175);
        let task = import_task("tounicode-cmap-pdf-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.files_discovered, 1);
        assert_eq!(summary.searchable_documents, 1);
        assert_eq!(summary.ocr_required_documents, 0);
        assert_eq!(summary.failed_documents, 0);
        let document = store.visible_documents().unwrap().remove(0);
        let versions = store.resume_versions_for_document(&document.id).unwrap();
        assert_eq!(versions.len(), 1);
        assert!(versions[0]
            .clean_text
            .as_deref()
            .unwrap()
            .contains("中文简历"));
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn import_root_rerun_with_unchanged_searchable_file_keeps_publication_stable() {
        let temp = TestDir::new("import-pipeline-zero-change-rerun");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("synthetic-resume.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_190);
        let first_task = import_task(
            "zero-change-first-import",
            root.to_str().unwrap(),
            first_now,
        );
        insert_test_import_task(&store, &first_task, &ImportOptions::default());

        let first_summary = import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let first_head = ready_search_head(&store);
        let first_status = store.status_summary().unwrap();

        assert_eq!(first_summary.files_discovered, 1);
        assert_eq!(first_summary.searchable_documents, 1);
        assert_eq!(first_status.searchable_documents, 1);

        let second_now = UnixTimestamp::from_unix_seconds(1_700_000_191);
        let second_task = import_task(
            "zero-change-second-import",
            root.to_str().unwrap(),
            second_now,
        );
        insert_test_import_task(&store, &second_task, &ImportOptions::default());

        let second_summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();
        let second_head = ready_search_head(&store);
        let second_status = store.status_summary().unwrap();
        let documents = store.visible_documents().unwrap();

        assert_eq!(second_summary.files_discovered, 1);
        assert_eq!(second_summary.searchable_documents, 1);
        assert_eq!(second_summary.ocr_required_documents, 0);
        assert_eq!(second_summary.ocr_jobs_queued, 0);
        assert_eq!(second_summary.failed_documents, 0);
        assert_eq!(second_summary.deleted_documents, 0);
        assert_eq!(second_status.searchable_documents, 1);
        assert_eq!(second_status.ocr_jobs_queued, 0);
        assert_eq!(documents.len(), 1);
        assert_eq!(
            store
                .resume_versions_for_document(&documents[0].id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(second_head.visible_epoch, first_head.visible_epoch);
        assert_eq!(
            second_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            first_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count()
        );
        assert_eq!(second_head.generation, first_head.generation);
        assert!(!format!("{second_head:?}").contains(root.to_str().unwrap()));
    }

    #[test]
    fn import_root_rename_publishes_same_version_with_new_metadata_and_artifacts() {
        let temp = TestDir::new("import-pipeline-rename-rerun");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(root.join("before")).unwrap();
        fs::write(
            root.join("before/synthetic-resume.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust"),
        )
        .unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_192);
        let first_task = import_task("rename-first-import", root.to_str().unwrap(), first_now);
        insert_test_import_task(&store, &first_task, &ImportOptions::default());
        import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let first_document = store.visible_documents().unwrap().remove(0);
        let first_head = ready_search_head(&store);
        let first_projection = store
            .active_search_projection_for_document(&first_document.id)
            .unwrap()
            .unwrap();
        let first_projected_document = store
            .active_search_document(&first_projection)
            .unwrap()
            .unwrap();

        fs::create_dir_all(root.join("after")).unwrap();
        fs::rename(
            root.join("before/synthetic-resume.txt"),
            root.join("after/renamed-resume.txt"),
        )
        .unwrap();
        assert_eq!(
            store
                .active_search_document(&first_projection)
                .unwrap()
                .unwrap(),
            first_projected_document
        );
        let second_now = UnixTimestamp::from_unix_seconds(1_700_000_193);
        let second_task = import_task("rename-second-import", root.to_str().unwrap(), second_now);
        insert_test_import_task(&store, &second_task, &ImportOptions::default());
        let summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();
        let second_document = store.visible_documents().unwrap().remove(0);
        let second_head = ready_search_head(&store);
        let second_projection = store
            .active_search_projection_for_document(&second_document.id)
            .unwrap()
            .unwrap();
        let second_projected_document = store
            .active_search_document(&second_projection)
            .unwrap()
            .unwrap();

        assert_eq!(summary.deleted_documents, 0);
        assert_eq!(first_document.id, second_document.id);
        assert_eq!(first_projection, second_projection);
        assert!(second_document
            .normalized_path
            .ends_with("after/renamed-resume.txt"));
        assert!(second_projected_document
            .normalized_path
            .ends_with("after/renamed-resume.txt"));
        assert_eq!(second_projected_document.file_name, "renamed-resume.txt");
        assert_eq!(
            store
                .resume_versions_for_document(&second_document.id)
                .unwrap()
                .len(),
            1
        );
        assert_ne!(second_head.generation, first_head.generation);
        assert_eq!(
            second_head.visible_epoch,
            first_head.visible_epoch.checked_add(1).unwrap()
        );
        let indexed_documents = incremental_snapshot_documents(
            &data_dir.join("search-index"),
            Some(&second_head.generation),
            Vec::new(),
            &BTreeSet::new(),
        )
        .unwrap();
        assert_eq!(indexed_documents.len(), 1);
        assert_eq!(indexed_documents[0].file_name, "renamed-resume.txt");
    }

    #[test]
    fn strong_content_hash_matches_sha256_known_vector() {
        assert_eq!(
            ContentDigest::from_bytes(b"abc").as_str(),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn import_root_strong_hash_detects_middle_only_change_hidden_from_quick_fingerprint() {
        let temp = TestDir::new("import-pipeline-strong-content-hash");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        let path = root.join("synthetic-resume.txt");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        let content = synthetic_large_resume_with_middle_skill("Rust");
        fs::write(&path, &content).unwrap();
        let original_mtime = fs::metadata(&path).unwrap().modified().unwrap();
        let first_quick_fingerprint = fs_crawler::crawl_directory(&root)
            .unwrap()
            .files
            .remove(0)
            .fingerprint;

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_194);
        let first_task = import_task(
            "strong-hash-first-import",
            root.to_str().unwrap(),
            first_now,
        );
        insert_test_import_task(&store, &first_task, &ImportOptions::default());
        import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let first_document = store.visible_documents().unwrap().remove(0);
        let first_content_hash = first_document.content_hash.clone().unwrap();
        let first_head = ready_search_head(&store);
        let first_projection = store
            .active_search_projection_for_document(&first_document.id)
            .unwrap()
            .unwrap();
        let first_selection = SearchSelection {
            document_id: first_document.id.clone(),
            resume_version_id: first_projection.resume_version_id.clone(),
            visible_epoch: first_head.visible_epoch,
        };

        fs::write(&path, synthetic_large_resume_with_middle_skill("Java")).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_mtime))
            .unwrap();
        let second_quick_fingerprint = fs_crawler::crawl_directory(&root)
            .unwrap()
            .files
            .remove(0)
            .fingerprint;
        assert_eq!(
            first_quick_fingerprint.as_str(),
            second_quick_fingerprint.as_str()
        );

        let second_now = UnixTimestamp::from_unix_seconds(1_700_000_195);
        let second_task = import_task(
            "strong-hash-second-import",
            root.to_str().unwrap(),
            second_now,
        );
        insert_test_import_task(&store, &second_task, &ImportOptions::default());
        let summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();
        let second_document = store.visible_documents().unwrap().remove(0);
        let second_head = ready_search_head(&store);
        let second_projection = store
            .active_search_projection_for_document(&second_document.id)
            .unwrap()
            .unwrap();
        let first_version = store
            .resume_version_by_id(&first_projection.resume_version_id)
            .unwrap()
            .unwrap();
        let second_version = store
            .resume_version_by_id(&second_projection.resume_version_id)
            .unwrap()
            .unwrap();

        assert_eq!(summary.searchable_documents, 1);
        assert_eq!(summary.deleted_documents, 0);
        assert_eq!(first_document.id, second_document.id);
        assert_ne!(first_content_hash, second_document.content_hash.unwrap());
        assert_ne!(
            first_projection.resume_version_id,
            second_projection.resume_version_id
        );
        assert_ne!(first_head.generation, second_head.generation);
        assert!(first_version.clean_text.unwrap().contains("Rust"));
        assert!(second_version.clean_text.unwrap().contains("Java"));
        assert_eq!(
            resolve_selection(&store, &first_selection),
            SearchSelectionResolution::Stale
        );
    }

    fn synthetic_large_resume_with_middle_skill(skill: &str) -> Vec<u8> {
        let mut content = String::from(
            "Synthetic Candidate\nSummary\nEngineer\nExperience\nBuilt reliable systems\n",
        );
        content.push_str(&"a".repeat(5_000));
        content.push_str(skill);
        content.push_str(&"b".repeat(5_000));
        content.push_str("\nEducation\nSynthetic University\nSkills\nDatabases\n");
        content.into_bytes()
    }

    #[test]
    fn import_root_rerun_with_unchanged_ocr_required_file_requeues_only_terminal_job() {
        let temp = TestDir::new("import-pipeline-zero-change-ocr-rerun");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("scanned-resume.pdf"), scanned_pdf_bytes()).unwrap();

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let first_now = UnixTimestamp::from_unix_seconds(1_700_000_195);
        let first_task = import_task(
            "zero-change-ocr-first-import",
            root.to_str().unwrap(),
            first_now,
        );
        insert_test_import_task(&store, &first_task, &ImportOptions::default());

        let first_summary = import_root_with_options(
            &data_dir,
            &store,
            &first_task,
            &root,
            first_now,
            ImportOptions::default(),
        )
        .unwrap();
        let first_head = ready_search_head(&store);
        let first_status = store.status_summary().unwrap();

        assert_eq!(first_summary.files_discovered, 1);
        assert_eq!(first_summary.searchable_documents, 0);
        assert_eq!(first_summary.ocr_required_documents, 1);
        assert_eq!(first_summary.ocr_jobs_queued, 1);
        assert_eq!(first_status.searchable_documents, 0);
        assert_eq!(first_status.ocr_queue_depth, 1);
        assert_eq!(first_status.ocr_jobs_queued, 1);

        let second_now = UnixTimestamp::from_unix_seconds(1_700_000_196);
        let second_task = import_task(
            "zero-change-ocr-second-import",
            root.to_str().unwrap(),
            second_now,
        );
        insert_test_import_task(&store, &second_task, &ImportOptions::default());

        let second_summary = import_root_with_options(
            &data_dir,
            &store,
            &second_task,
            &root,
            second_now,
            ImportOptions::default(),
        )
        .unwrap();
        let second_head = ready_search_head(&store);
        let second_status = store.status_summary().unwrap();
        let documents = store.visible_documents().unwrap();

        assert_eq!(second_summary.files_discovered, 1);
        assert_eq!(second_summary.searchable_documents, 0);
        assert_eq!(second_summary.ocr_required_documents, 0);
        assert_eq!(second_summary.ocr_jobs_queued, 0);
        assert_eq!(second_summary.failed_documents, 0);
        assert_eq!(second_summary.deleted_documents, 0);
        assert_eq!(second_status.searchable_documents, 0);
        assert_eq!(second_status.ocr_queue_depth, 1);
        assert_eq!(second_status.ocr_jobs_queued, 1);
        assert_eq!(documents.len(), 1);
        assert_eq!(documents[0].status, meta_store::DocumentStatus::OcrRequired);
        assert_eq!(
            store
                .resume_versions_for_document(&documents[0].id)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(second_head.visible_epoch, first_head.visible_epoch);
        assert_eq!(
            second_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            first_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count()
        );
        assert_eq!(second_head.generation, first_head.generation);
        assert!(!format!("{second_head:?}").contains(root.to_str().unwrap()));

        let claimed_at = UnixTimestamp::from_unix_seconds(1_700_000_197);
        let claimed = store.claim_next_ocr_job(claimed_at).unwrap().unwrap();
        assert_eq!(claimed.job.status, IngestJobStatus::Running);
        assert_eq!(claimed.job.attempt_count, 1);
        store
            .finish_ocr_attempt_failure(
                &claimed,
                OcrAttemptFailure::Permanent,
                UnixTimestamp::from_unix_seconds(1_700_000_198),
            )
            .unwrap();

        let third_now = UnixTimestamp::from_unix_seconds(1_700_000_199);
        let third_task = import_task(
            "zero-change-ocr-third-import",
            root.to_str().unwrap(),
            third_now,
        );
        insert_test_import_task(&store, &third_task, &ImportOptions::default());
        let third_summary = import_root_with_options(
            &data_dir,
            &store,
            &third_task,
            &root,
            third_now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(third_summary.ocr_required_documents, 1);
        assert_eq!(third_summary.ocr_jobs_queued, 1);
        let requeued = store.ingest_job_by_id(&claimed.job.id).unwrap().unwrap();
        assert_eq!(requeued.status, IngestJobStatus::Queued);
        assert_eq!(requeued.attempt_count, 1);
        assert_eq!(requeued.queued_at, third_now);
        let reclaimed = store
            .claim_next_ocr_job(UnixTimestamp::from_unix_seconds(1_700_000_200))
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.job.attempt_count, 2);
    }

    #[cfg(unix)]
    #[test]
    fn import_root_parses_legacy_doc_with_local_converter_without_path_leak() {
        let _env_lock = DOC_CONVERTER_ENV_LOCK.lock().unwrap();
        let temp = TestDir::new("import-pipeline-doc-converter");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("legacy-word.doc"), synthetic_ole_doc()).unwrap();
        let converter = write_doc_converter(temp.path());
        let _env = EnvVarGuard::set(
            "RESUME_IR_DOC_TEXT_COMMAND",
            converter.to_str().unwrap().to_string(),
        );

        let store = create_test_store(&data_dir);
        store.run_migrations().unwrap();
        let now = UnixTimestamp::from_unix_seconds(1_700_000_200);
        let task = import_task("legacy-doc-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());

        let summary = import_root_with_options(
            &data_dir,
            &store,
            &task,
            &root,
            now,
            ImportOptions::default(),
        )
        .unwrap();

        assert_eq!(summary.files_discovered, 1);
        assert_eq!(summary.searchable_documents, 1);
        assert_eq!(summary.failed_documents, 0);
        let status = store.status_summary().unwrap();
        assert_eq!(status.searchable_documents, 1);
        let document = store.visible_documents().unwrap().remove(0);
        let versions = store.resume_versions_for_document(&document.id).unwrap();
        assert_eq!(versions.len(), 1);
        assert!(versions[0]
            .clean_text
            .as_deref()
            .unwrap()
            .contains("Synthetic Legacy Candidate"));
        assert!(!format!("{summary:?}").contains(root.to_str().unwrap()));
        assert!(!format!("{summary:?}").contains(converter.to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn import_root_publishes_searchable_progress_before_full_import_completion() {
        let _env_lock = DOC_CONVERTER_ENV_LOCK.lock().unwrap();
        let temp = TestDir::new("import-pipeline-first-searchable-progress");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();

        for index in 0..32 {
            fs::write(
                root.join(format!("{index:02}-fast.txt")),
                format!(
                    "SUMMARY\nSynthetic Candidate {index}\nEXPERIENCE\nBuilt Rust systems\nSKILLS\nRust"
                ),
            )
            .unwrap();
        }
        fs::write(root.join("zz-slow.doc"), synthetic_ole_doc()).unwrap();

        let converter = write_blocking_doc_converter(temp.path());
        let started_marker = converter.with_extension("started");
        let release_marker = converter.with_extension("release");
        let _env = EnvVarGuard::set(
            "RESUME_IR_DOC_TEXT_COMMAND",
            converter.to_str().unwrap().to_string(),
        );

        let store = create_test_store(&data_dir);
        let now = UnixTimestamp::from_unix_seconds(1_700_000_225);
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(now.as_unix_seconds() - 1),
        );
        let task = import_task(
            "first-searchable-progress-import",
            root.to_str().unwrap(),
            now,
        );
        insert_test_import_task(&store, &task, &ImportOptions::default());
        store
            .upsert_import_scan_scope(&ImportScanScope {
                import_task_id: task.id.clone(),
                root_kind: ImportRootKind::Explicit,
                root_preset: None,
                scan_profile: ImportScanProfile::Explicit,
                requested_root_path: root.to_str().unwrap().to_string(),
                canonical_root_path: root.to_str().unwrap().to_string(),
                files_discovered: 0,
                ignored_entries: 0,
                scan_errors: 0,
                searchable_documents: 0,
                ocr_required_documents: 0,
                ocr_jobs_queued: 0,
                failed_documents: 0,
                deleted_documents: 0,
                scan_budget_kind: None,
                scan_budget_limit: None,
                scan_budget_observed: None,
                scan_budget_exhausted: false,
                updated_at: now,
            })
            .unwrap();

        let data_dir_for_worker = data_dir.clone();
        let root_for_worker = root.clone();
        let task_for_worker = task.clone();
        let worker_store = store.open_sibling().unwrap();
        let worker = thread::spawn(move || {
            import_root_with_options(
                &data_dir_for_worker,
                &worker_store,
                &task_for_worker,
                &root_for_worker,
                now,
                ImportOptions::default(),
            )
            .unwrap()
        });

        wait_for_path(&started_marker);
        let observed_store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let scope = observed_store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        let status = observed_store.status_summary().unwrap();
        let observed_head = ready_search_head_from_reader(&observed_store);
        let _ready_reader = open_fulltext_generation(&data_dir, &observed_head.generation);

        fs::write(&release_marker, b"release").unwrap();
        let summary = worker.join().unwrap();
        let final_store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let final_head = ready_search_head_from_reader(&final_store);

        assert_eq!(scope.files_discovered, 33);
        assert!(
            scope.searchable_documents > 0,
            "expected mid-run searchable progress before full import completion, got scope: {scope:?}"
        );
        assert!(
            status.searchable_documents > 0,
            "expected searchable documents to be visible before the final file completed, got status: {status:?}"
        );
        assert_eq!(observed_head.visible_epoch, 2);
        assert_eq!(
            observed_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            1
        );
        assert_eq!(summary.searchable_documents, 33);
        assert_eq!(final_head.visible_epoch, 3);
        assert_eq!(
            final_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            33
        );
    }

    #[cfg(unix)]
    #[test]
    fn import_root_publishes_first_searchable_before_batch_threshold() {
        let _env_lock = DOC_CONVERTER_ENV_LOCK.lock().unwrap();
        let temp = TestDir::new("import-pipeline-first-searchable-early");
        let data_dir = temp.path().join("data");
        let root = temp.path().join("resumes");
        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("00-fast.txt"),
            synthetic_resume_text("Synthetic Candidate", "Rust"),
        )
        .unwrap();
        fs::write(root.join("zz-slow.doc"), synthetic_ole_doc()).unwrap();

        let converter = write_blocking_doc_converter(temp.path());
        let started_marker = converter.with_extension("started");
        let release_marker = converter.with_extension("release");
        let _env = EnvVarGuard::set(
            "RESUME_IR_DOC_TEXT_COMMAND",
            converter.to_str().unwrap().to_string(),
        );

        let store = create_test_store(&data_dir);
        let now = UnixTimestamp::from_unix_seconds(1_700_000_230);
        initialize_ready_empty_search(
            &data_dir,
            &store,
            UnixTimestamp::from_unix_seconds(now.as_unix_seconds() - 1),
        );
        let task = import_task("first-searchable-early-import", root.to_str().unwrap(), now);
        insert_test_import_task(&store, &task, &ImportOptions::default());
        store
            .upsert_import_scan_scope(&ImportScanScope {
                import_task_id: task.id.clone(),
                root_kind: ImportRootKind::Explicit,
                root_preset: None,
                scan_profile: ImportScanProfile::Explicit,
                requested_root_path: root.to_str().unwrap().to_string(),
                canonical_root_path: root.to_str().unwrap().to_string(),
                files_discovered: 0,
                ignored_entries: 0,
                scan_errors: 0,
                searchable_documents: 0,
                ocr_required_documents: 0,
                ocr_jobs_queued: 0,
                failed_documents: 0,
                deleted_documents: 0,
                scan_budget_kind: None,
                scan_budget_limit: None,
                scan_budget_observed: None,
                scan_budget_exhausted: false,
                updated_at: now,
            })
            .unwrap();

        let data_dir_for_worker = data_dir.clone();
        let root_for_worker = root.clone();
        let task_for_worker = task.clone();
        let worker_store = store.open_sibling().unwrap();
        let worker = thread::spawn(move || {
            import_root_with_options(
                &data_dir_for_worker,
                &worker_store,
                &task_for_worker,
                &root_for_worker,
                now,
                ImportOptions::default(),
            )
            .unwrap()
        });

        wait_for_path(&started_marker);
        let observed_store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let scope = observed_store
            .import_scan_scope_by_task_id(&task.id)
            .unwrap()
            .unwrap();
        let status = observed_store.status_summary().unwrap();
        let observed_head = ready_search_head_from_reader(&observed_store);
        let _ready_reader = open_fulltext_generation(&data_dir, &observed_head.generation);

        fs::write(&release_marker, b"release").unwrap();
        let summary = worker.join().unwrap();
        let final_store = ReadMetaStore::open_data_dir(&data_dir).unwrap();
        let final_head = ready_search_head_from_reader(&final_store);

        assert_eq!(scope.files_discovered, 2);
        assert!(
            scope.searchable_documents > 0,
            "expected first searchable document to publish before batch threshold, got scope: {scope:?}"
        );
        assert!(
            status.searchable_documents > 0,
            "expected searchable status before the slow file completed, got status: {status:?}"
        );
        assert_eq!(observed_head.visible_epoch, 2);
        assert_eq!(
            observed_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            1
        );
        assert_eq!(summary.searchable_documents, 2);
        assert!(summary.milestone_timings.first_searchable.is_some());
        assert!(summary.milestone_timings.full_import_ready.is_some());
        assert!(summary.milestone_timings.full_index_ready.is_some());
        assert_eq!(final_head.visible_epoch, 3);
        assert_eq!(
            final_head
                .publication
                .fulltext
                .as_ref()
                .unwrap()
                .document_count(),
            2
        );
    }

    fn import_task(label: &str, root_path: &str, now: UnixTimestamp) -> ImportTask {
        ImportTask {
            id: meta_store::ImportTaskId::from_non_secret_parts(&[label]),
            root_path: root_path.to_string(),
            status: ImportTaskStatus::Running,
            queued_at: now,
            started_at: Some(now),
            finished_at: None,
            updated_at: now,
        }
    }

    fn insert_test_import_task(
        store: &OwnedMetaStore,
        task: &ImportTask,
        options: &ImportOptions,
    ) -> meta_store::ImportProcessingContract {
        let scope = import_scan_scope(
            task,
            options.max_files.map(|limit| u64::try_from(limit).unwrap()),
        );
        insert_test_import_task_with_scope(store, task, &scope, options)
    }

    fn insert_test_import_task_with_scope(
        store: &OwnedMetaStore,
        task: &ImportTask,
        scope: &ImportScanScope,
        options: &ImportOptions,
    ) -> meta_store::ImportProcessingContract {
        let contract = super::current_import_processing_contract(options).unwrap();
        store
            .activate_migration_rebuild_contract(&contract, task.updated_at)
            .unwrap();
        let queued_task = ImportTask {
            status: ImportTaskStatus::Queued,
            started_at: None,
            finished_at: None,
            ..task.clone()
        };
        let projection = store.search_projection_state().unwrap();
        if projection.service_state == SearchProjectionServiceState::Repairing
            && projection.repair_reason == Some(SearchRepairReason::MigrationRebuild)
            && projection.generation.is_none()
        {
            let seed_id = meta_store::ImportTaskId::from_non_secret_parts(&[
                "import-pipeline-test-root-seed",
                task.id.as_str(),
            ]);
            let seed = ImportTask {
                id: seed_id.clone(),
                ..queued_task.clone()
            };
            let mut seed_scope = scope.clone();
            seed_scope.import_task_id = seed_id.clone();
            store
                .insert_import_task_with_scan_scope(&seed, &seed_scope, &contract)
                .unwrap();
            store.cancel_import_task(&seed_id, task.updated_at).unwrap();
            assert!(matches!(
                store
                    .enqueue_full_corpus_migration_rebuild_root(
                        &task.root_path,
                        &task.id,
                        &contract,
                        task.updated_at,
                    )
                    .unwrap(),
                meta_store::ImportRootTaskHeadOutcome::HeadInserted { .. }
            ));
            store.upsert_import_scan_scope(scope).unwrap();
        } else {
            store
                .insert_import_task_with_scan_scope(&queued_task, scope, &contract)
                .unwrap();
        }
        let claimed = store
            .claim_observed_import_task_for_worker(&queued_task, task.updated_at)
            .unwrap()
            .expect("new test import task must be claimable");
        assert_eq!(claimed.id, task.id);
        assert_eq!(claimed.status, ImportTaskStatus::Running);
        contract
    }

    fn synthetic_ole_doc() -> Vec<u8> {
        let mut bytes = b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1".to_vec();
        bytes.extend_from_slice(b"SYNTHETIC PRIVATE LEGACY DOC BODY");
        bytes
    }

    fn utf16be_literal_text_layer_pdf_bytes() -> Vec<u8> {
        let mut content = b"BT /F1 12 Tf 72 720 Td (SUMMARY) Tj T* (EXPERIENCE) Tj T* (Built systems) Tj T* (SKILLS) Tj T* (".to_vec();
        content.extend_from_slice(b"\xFE\xFF\x4E\x2D\x65\x87\x7B\x80\x53\x86");
        content.extend_from_slice(b") Tj ET\n");

        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
                content,
                b"endstream".to_vec(),
            ]
            .concat(),
        ])
    }

    fn tounicode_cmap_pdf_bytes() -> Vec<u8> {
        let cmap = br"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def
/CMapName /Adobe-Identity-UCS def
/CMapType 2 def
1 begincodespacerange
<0001> <0004>
endcodespacerange
4 beginbfchar
<0001> <4E2D>
<0002> <6587>
<0003> <7B80>
<0004> <5386>
endbfchar
endcmap
CMapName currentdict /CMap defineresource pop
end
end
";
        let content = b"BT /F2 12 Tf 72 720 Td (SUMMARY) Tj T* (EXPERIENCE) Tj T* (Built systems) Tj T* (SKILLS) Tj T* /F1 12 Tf <0001000200030004> Tj ET\n";

        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R /F2 9 0 R >> >> /MediaBox [0 0 612 792] /Contents 7 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /Type0 /BaseFont /TestFont /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 6 0 R >>".to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /TestFont /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 8 0 R /DW 1000 /W [1 [1000 1000]] >>".to_vec(),
            [
                format!("<< /Length {} >>\nstream\n", cmap.len()).into_bytes(),
                cmap.to_vec(),
                b"endstream".to_vec(),
            ]
            .concat(),
            [
                format!("<< /Length {} >>\nstream\n", content.len()).into_bytes(),
                content.to_vec(),
                b"endstream".to_vec(),
            ]
            .concat(),
            b"<< /Type /FontDescriptor /FontName /TestFont /Flags 4 /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 80 >>".to_vec(),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ])
    }

    fn scanned_pdf_bytes() -> Vec<u8> {
        build_valid_pdf(vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /Resources << /XObject << /Im1 4 0 R >> >> /MediaBox [0 0 612 792] /Contents 5 0 R >>".to_vec(),
            b"<< /Type /XObject /Subtype /Image /Width 100 /Height 100 /ColorSpace /DeviceGray /BitsPerComponent 8 /Length 11 >>\nstream\nimage bytes\nendstream".to_vec(),
            b"<< /Length 24 >>\nstream\nq 100 0 0 100 0 0 cm /Im1 Do Q\nendstream".to_vec(),
        ])
    }

    fn build_valid_pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::with_capacity(objects.len());

        for (index, object) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(object);
            if !object.ends_with(b"\n") {
                pdf.push(b'\n');
            }
            pdf.extend_from_slice(b"endobj\n");
        }

        let xref_offset = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Root 1 0 R /Size {} >>\nstartxref\n{}\n%%EOF",
                objects.len() + 1,
                xref_offset
            )
            .as_bytes(),
        );

        pdf
    }

    #[cfg(unix)]
    fn write_doc_converter(directory: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join("fixture-doc-converter");
        fs::write(
            &path,
            r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-output" ]; then
    shift
    out="$1"
  fi
  shift
done
if [ -z "$out" ]; then
  exit 9
fi
printf 'SUMMARY\nSynthetic Legacy Candidate\nEXPERIENCE\nBuilt Rust Search systems\nSKILLS\nRust Search\n' > "$out"
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[cfg(unix)]
    fn write_blocking_doc_converter(directory: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = directory.join("fixture-blocking-doc-converter");
        fs::write(
            &path,
            r#"#!/bin/sh
self="$0"
started="${self%.*}.started"
release="${self%.*}.release"
out=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-output" ]; then
    shift
    out="$1"
  fi
  shift
done
if [ -z "$out" ]; then
  exit 9
fi
: > "$started"
while [ ! -f "$release" ]; do
  sleep 0.01
done
printf 'SUMMARY\nSlow Synthetic Legacy Candidate\nEXPERIENCE\nBuilt Rust Search systems\nSKILLS\nRust Search\n' > "$out"
"#,
        )
        .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: String) -> Self {
            let previous = env::var(key).ok();
            env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                env::set_var(self.key, previous);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    fn wait_for_path(path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if path.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {}", path.display());
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = format!(
                "{}-{}-{}",
                label,
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
