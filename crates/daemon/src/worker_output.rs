use std::io::{self, Write};

use import_pipeline::{ImportPipelineErrorClass, SearchArtifactRecoverySummary};
use meta_store::StoreStatusSummary;

use crate::daemon_error::{DaemonError, Result};
use crate::import_watcher::ImportWatcherSummary;
use crate::store_access::index_health_label;

#[derive(Clone, Copy)]
pub(crate) enum StartupMode {
    Once,
    Foreground,
}

pub(crate) fn print_startup_summary(mode: StartupMode, summary: &StoreStatusSummary) -> Result<()> {
    println!("resume-daemon foreground ready");
    println!(
        "mode: {}",
        match mode {
            StartupMode::Once => "once",
            StartupMode::Foreground => "foreground",
        }
    );
    println!("index health: {}", index_health_label(summary.index_health));
    println!("import tasks queued: {}", summary.import_tasks_queued);
    println!("import tasks cancelled: {}", summary.import_tasks_cancelled);
    flush_status()
}

#[derive(Default)]
pub(crate) struct ImportWorkerSummary {
    pub(crate) orphaned_recovered: usize,
    pub(crate) stale_recovered: usize,
    pub(crate) repair_requeued: usize,
    pub(crate) completed_requeued: usize,
    pub(crate) watcher_active_roots: Option<usize>,
    pub(crate) watcher_events: usize,
    pub(crate) watcher_requeued: usize,
    pub(crate) watcher_event_errors: usize,
    pub(crate) processed: usize,
    pub(crate) cancelled: usize,
    pub(crate) failed: usize,
    pub(crate) failure_class: Option<ImportPipelineErrorClass>,
    pub(crate) metadata_failure_class: Option<&'static str>,
    pub(crate) searchable_documents: usize,
    pub(crate) ocr_jobs_queued: usize,
}

impl ImportWorkerSummary {
    pub(crate) fn has_activity(&self) -> bool {
        self.orphaned_recovered > 0
            || self.stale_recovered > 0
            || self.repair_requeued > 0
            || self.completed_requeued > 0
            || self.watcher_active_roots.is_some()
            || self.watcher_events > 0
            || self.watcher_requeued > 0
            || self.watcher_event_errors > 0
            || self.processed > 0
            || self.cancelled > 0
            || self.failed > 0
            || self.searchable_documents > 0
            || self.ocr_jobs_queued > 0
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.orphaned_recovered += other.orphaned_recovered;
        self.stale_recovered += other.stale_recovered;
        self.repair_requeued += other.repair_requeued;
        self.completed_requeued += other.completed_requeued;
        if other.watcher_active_roots.is_some() {
            self.watcher_active_roots = other.watcher_active_roots;
        }
        self.watcher_events += other.watcher_events;
        self.watcher_requeued += other.watcher_requeued;
        self.watcher_event_errors += other.watcher_event_errors;
        self.processed += other.processed;
        self.cancelled += other.cancelled;
        self.failed += other.failed;
        if other.failure_class.is_some() {
            self.failure_class = other.failure_class;
        }
        if other.metadata_failure_class.is_some() {
            self.metadata_failure_class = other.metadata_failure_class;
        }
        self.searchable_documents += other.searchable_documents;
        self.ocr_jobs_queued += other.ocr_jobs_queued;
    }

    pub(crate) fn extend_watcher(&mut self, watcher_summary: ImportWatcherSummary) {
        if watcher_summary.active_roots.is_some() {
            self.watcher_active_roots = watcher_summary.active_roots;
        }
        self.watcher_events += watcher_summary.events;
        self.watcher_requeued += watcher_summary.requeued;
        self.watcher_event_errors += watcher_summary.event_errors;
    }
}

pub(crate) fn print_import_worker_summary(summary: &ImportWorkerSummary) -> Result<()> {
    println!(
        "import worker recovered orphaned running: {}",
        summary.orphaned_recovered
    );
    println!(
        "import worker recovered stale running: {}",
        summary.stale_recovered
    );
    println!(
        "import worker requeued completed imports: {}",
        summary.completed_requeued
    );
    println!(
        "import worker queued migration repairs: {}",
        summary.repair_requeued
    );
    if let Some(active_roots) = summary.watcher_active_roots {
        println!("import watcher active roots: {active_roots}");
    }
    println!("import watcher events: {}", summary.watcher_events);
    println!(
        "import watcher requeued imports: {}",
        summary.watcher_requeued
    );
    println!(
        "import watcher event errors: {}",
        summary.watcher_event_errors
    );
    println!("import worker processed: {}", summary.processed);
    println!("import worker cancelled: {}", summary.cancelled);
    println!("import worker failed: {}", summary.failed);
    if let Some(class) = summary.failure_class {
        println!("import worker failure class: {}", class.label());
    }
    if let Some(class) = summary.metadata_failure_class {
        println!("import worker metadata failure class: {class}");
    }
    println!(
        "import worker searchable documents: {}",
        summary.searchable_documents
    );
    println!("import worker ocr jobs queued: {}", summary.ocr_jobs_queued);
    flush_status()
}

#[derive(Default)]
pub(crate) struct OcrWorkerSummary {
    pub(crate) stale_recovered: usize,
    pub(crate) paused: bool,
    pub(crate) runtime_unavailable: Option<crate::ipc::OptionalRuntimeReason>,
    pub(crate) processed: usize,
    pub(crate) failed: usize,
    pub(crate) cache_writes: usize,
    pub(crate) cache_hits: usize,
}

impl OcrWorkerSummary {
    pub(crate) fn has_activity(&self) -> bool {
        self.stale_recovered > 0
            || self.paused
            || self.runtime_unavailable.is_some()
            || self.processed > 0
            || self.failed > 0
            || self.cache_writes > 0
            || self.cache_hits > 0
    }

    pub(crate) fn extend(&mut self, other: Self) {
        self.stale_recovered += other.stale_recovered;
        self.paused = self.paused || other.paused;
        if other.runtime_unavailable.is_some() {
            self.runtime_unavailable = other.runtime_unavailable;
        }
        self.processed += other.processed;
        self.failed += other.failed;
        self.cache_writes += other.cache_writes;
        self.cache_hits += other.cache_hits;
    }
}

pub(crate) fn print_ocr_worker_summary(summary: &OcrWorkerSummary) -> Result<()> {
    println!(
        "ingest jobs recovered stale running: {}",
        summary.stale_recovered
    );
    println!("ocr worker paused: {}", summary.paused);
    if let Some(reason) = summary.runtime_unavailable {
        println!("ocr worker runtime unavailable: {}", reason.label());
    }
    println!("ocr worker processed: {}", summary.processed);
    println!("ocr worker cache writes: {}", summary.cache_writes);
    println!("ocr worker cache hits: {}", summary.cache_hits);
    println!("ocr worker failed: {}", summary.failed);
    flush_status()
}

pub(crate) fn search_artifact_recovery_has_activity(
    summary: &SearchArtifactRecoverySummary,
) -> bool {
    summary.interrupted_publications_abandoned > 0
        || summary.fulltext_staging_directories_removed > 0
        || summary.vector_staging_directories_removed > 0
        || summary.fulltext_generations_removed > 0
        || summary.vector_generations_removed > 0
        || summary.active_generation_rebuilt
        || summary.gc_deferred
        || summary.gc_partial
        || summary.gc_failed
}

pub(crate) fn print_search_artifact_worker_summary(
    summary: &SearchArtifactRecoverySummary,
) -> Result<()> {
    println!(
        "search artifact worker active generation rebuilt: {}",
        if summary.active_generation_rebuilt {
            "yes"
        } else {
            "no"
        }
    );
    println!(
        "search artifact worker interrupted publications abandoned: {}",
        summary.interrupted_publications_abandoned
    );
    println!(
        "search artifact worker fulltext staging removed: {}",
        summary.fulltext_staging_directories_removed
    );
    println!(
        "search artifact worker vector staging removed: {}",
        summary.vector_staging_directories_removed
    );
    println!(
        "search artifact worker fulltext generations removed: {}",
        summary.fulltext_generations_removed
    );
    println!(
        "search artifact worker vector generations removed: {}",
        summary.vector_generations_removed
    );
    println!(
        "search artifact worker gc deferred: {}",
        summary.gc_deferred
    );
    println!("search artifact worker gc partial: {}", summary.gc_partial);
    println!("search artifact worker gc failed: {}", summary.gc_failed);
    flush_status()
}

fn flush_status() -> Result<()> {
    io::stdout()
        .flush()
        .map_err(|_| DaemonError::control_plane("unable to write daemon status"))
}
