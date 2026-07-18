use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use index_fulltext::publish_snapshot;
use index_vector::{
    VectorDocument, VectorDocumentIdentity, VectorModelContract, VectorSnapshotStore,
};
use meta_store::{
    ActiveSearchProjection, ClassificationStatus, ContentDigest, Document, DocumentId,
    DocumentStatus, EnabledVectorSnapshotDescriptor, EntityMention, EntityMentionId, EntityType,
    FileExtension, FullTextSnapshotDescriptor, MetaStore, ReasonCode, ResumeVersion,
    ResumeVersionClassification, ResumeVersionId, ReviewDisposition, SearchProjectionDigest,
    SearchPublicationCommit, SearchPublicationDraft, SearchPublicationOutcome,
    SearchPublicationValidation, SourceRevision, TerminalDocumentUpdate, UnixTimestamp,
    VectorSnapshotDescriptor, CLASSIFIER_EPOCH,
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use super::{synthetic_document, synthetic_query_workload, BenchmarkError, Result};

const SNAPSHOT_TOKEN: &str = "public-synthetic-query-hot-path-v1";
const MODEL_ID: &str = "resume-ir-hash-embedding-v1";
const VECTOR_DIMENSION: usize = 8;
const BYTES_PER_GIB: u64 = 1024 * 1024 * 1024;
const H0_MEMORY_CEILING_BYTES: u64 = 12 * BYTES_PER_GIB;
const H1_MEMORY_CEILING_BYTES: u64 = 20 * BYTES_PER_GIB;

pub(super) fn prepare_fixture(data_dir: &Path) -> Result<()> {
    if data_dir.exists()
        && fs::read_dir(data_dir)
            .map_err(BenchmarkError::io)?
            .next()
            .is_some()
    {
        return Err(BenchmarkError::invalid_config(
            "resident_query_data_dir_must_be_empty",
        ));
    }
    fs::create_dir_all(data_dir).map_err(BenchmarkError::io)?;
    let store = MetaStore::open_data_dir(data_dir)
        .and_then(|store| {
            store.run_migrations()?;
            Ok(store)
        })
        .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?;
    let now = UnixTimestamp::from_unix_seconds(1_800_100_000);
    let count = synthetic_query_workload::CANONICAL_DOCUMENT_COUNT;
    let mut index_documents = Vec::with_capacity(count);
    let mut vector_documents = Vec::with_capacity(count);
    let mut projections = Vec::with_capacity(count);
    let mut terminal_documents = Vec::with_capacity(count);
    for index in 0..count {
        let mut indexed = synthetic_document(index, count);
        let document_id =
            DocumentId::from_non_secret_parts(&["resident-public", &index.to_string()]);
        let source = indexed.clean_text.as_bytes();
        let revision = SourceRevision::for_content(
            document_id.clone(),
            ContentDigest::from_bytes(source),
            source.len() as u64,
        );
        let normalized_text_hash = ContentDigest::from_bytes(source);
        let version_id = ResumeVersionId::from_content_identity(
            &document_id,
            &revision.id,
            &normalized_text_hash,
            "public-synthetic-v27",
            "schema-v27",
        );
        indexed.doc_id = document_id.to_string();
        indexed.resume_version_id = version_id.to_string();
        let staged = Document {
            id: document_id.clone(),
            source_uri: format!("synthetic://{index}"),
            normalized_path: format!("synthetic/{index}"),
            file_name: indexed.file_name.clone(),
            extension: FileExtension::Pdf,
            byte_size: source.len() as u64,
            mtime: now,
            content_hash: Some(revision.content_hash.as_str().to_string()),
            text_hash: Some(normalized_text_hash.as_str().to_string()),
            is_deleted: false,
            created_at: now,
            updated_at: now,
            status: DocumentStatus::FieldsExtracted,
        };
        store
            .upsert_document(&staged)
            .and_then(|_| store.insert_source_revision(&revision).map(|_| ()))
            .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?;
        let version = ResumeVersion {
            id: version_id.clone(),
            document_id: document_id.clone(),
            source_revision_id: revision.id,
            normalized_text_hash,
            parse_version: "public-synthetic-v27".to_string(),
            schema_version: "schema-v27".to_string(),
            language_set: vec!["en".to_string()],
            page_count: Some(1),
            raw_text: None,
            clean_text: Some(indexed.clean_text.clone()),
            quality_score: Some(1.0),
        };
        store
            .insert_resume_version(&version)
            .and_then(|_| {
                store
                    .insert_resume_version_classification(&ResumeVersionClassification {
                        resume_version_id: version_id.clone(),
                        status: ClassificationStatus::ResumeCandidate,
                        classifier_epoch: CLASSIFIER_EPOCH.to_string(),
                        reason_codes: vec![ReasonCode::CorroboratedResumeSignals],
                        classified_at: now,
                        review_disposition: ReviewDisposition::NotRequired,
                    })
                    .map(|_| ())
            })
            .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?;
        if (325..400).contains(&(index % synthetic_query_workload::CYCLE_QUERY_COUNT)) {
            store
                .insert_entity_mentions(
                    &version_id,
                    &[EntityMention {
                        id: EntityMentionId::from_non_secret_parts(&[
                            "resident-public",
                            "location",
                            &index.to_string(),
                        ]),
                        resume_version_id: version_id.clone(),
                        section_id: None,
                        entity_type: EntityType::Location,
                        raw_value: "shanghai".to_string(),
                        normalized_value: Some("shanghai".to_string()),
                        span_start: Some(0),
                        span_end: Some(8),
                        confidence: 1.0,
                        extractor: "public-synthetic-v27".to_string(),
                    }],
                )
                .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?;
        }
        let mut values = vec![0.0_f32; VECTOR_DIMENSION];
        values[index % VECTOR_DIMENSION] = 1.0;
        vector_documents.push(
            VectorDocument::new(
                VectorDocumentIdentity::new(
                    format!("vec_{index:08}"),
                    document_id.to_string(),
                    version_id.to_string(),
                    MODEL_ID,
                )
                .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?,
                values,
            )
            .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?,
        );
        projections.push(ActiveSearchProjection {
            document_id: document_id.clone(),
            resume_version_id: version_id,
        });
        terminal_documents.push(TerminalDocumentUpdate {
            document_id,
            expected_status: DocumentStatus::FieldsExtracted,
            expected_is_deleted: false,
            expected_content_hash: revision.content_hash,
            terminal_status: DocumentStatus::Searchable,
            terminal_is_deleted: false,
        });
        index_documents.push(indexed);
    }
    projections.sort_by(|left, right| left.document_id.cmp(&right.document_id));
    let projection_digest = SearchProjectionDigest::from_pairs(
        projections
            .iter()
            .map(|item| (item.document_id.as_str(), item.resume_version_id.as_str())),
    )
    .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?;
    if store
        .begin_search_publication(&SearchPublicationDraft {
            generation: SNAPSHOT_TOKEN.to_string(),
            base_generation: None,
            expected_visible_epoch: 0,
            classifier_epoch: CLASSIFIER_EPOCH.to_string(),
            projection_digest: projection_digest.clone(),
            now,
        })
        .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?
        != SearchPublicationOutcome::Applied
    {
        return Err(BenchmarkError::invalid_config("resident_query_fixture"));
    }
    let fulltext = publish_snapshot(
        &data_dir.join("search-index"),
        SNAPSHOT_TOKEN,
        index_documents,
    )
    .map_err(BenchmarkError::fulltext)?;
    let vector_contract = VectorModelContract::enabled(MODEL_ID, VECTOR_DIMENSION)
        .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?;
    let vector = VectorSnapshotStore::new(data_dir.join("vector-index"), vector_contract)
        .and_then(|store| {
            store.publish_generation(
                SNAPSHOT_TOKEN,
                projections.iter().cloned(),
                vector_documents,
            )
        })
        .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?;
    let fulltext_descriptor = FullTextSnapshotDescriptor::new(
        SNAPSHOT_TOKEN.to_string(),
        fulltext.document_count() as u64,
        fulltext.projection_digest().clone(),
        fulltext.logical_content_digest().clone(),
    );
    let vector_descriptor = VectorSnapshotDescriptor::enabled(EnabledVectorSnapshotDescriptor {
        generation: SNAPSHOT_TOKEN.to_string(),
        model_id: MODEL_ID.to_string(),
        dimension: VECTOR_DIMENSION as u32,
        projection_count: vector.projection_count() as u64,
        projection_digest: vector.projection_digest().clone(),
        coverage_digest: vector.coverage_digest().clone(),
        vector_count: vector.vector_count() as u64,
        document_count: vector.vector_document_count() as u64,
        resume_version_count: vector.vector_document_count() as u64,
        logical_content_digest: vector.logical_content_digest().clone(),
    });
    store
        .validate_search_publication(&SearchPublicationValidation {
            generation: SNAPSHOT_TOKEN,
            fulltext: &fulltext_descriptor,
            vector: &vector_descriptor,
            now,
        })
        .and_then(|_| {
            store.commit_search_publication(&SearchPublicationCommit {
                generation: SNAPSHOT_TOKEN,
                terminal_documents: &terminal_documents,
                projections: &projections,
                vector_coverage: &projections,
                now,
            })
        })
        .map_err(|_| BenchmarkError::invalid_config("resident_query_fixture"))?;
    Ok(())
}

pub(super) struct ResidentDaemon {
    child: Child,
    endpoint: String,
    stdout_drain: Option<thread::JoinHandle<()>>,
}

impl ResidentDaemon {
    pub(super) fn start(
        data_dir: &Path,
        daemon_command: &Path,
        embedding_command: &Path,
    ) -> Result<Self> {
        let mut child = Command::new(daemon_command)
            .arg("--data-dir")
            .arg(data_dir)
            .arg("run")
            .arg("--foreground")
            .arg("--ipc-listen")
            .arg("127.0.0.1:0")
            .arg("--embedding-command")
            .arg(embedding_command)
            .arg("--embedding-model-id")
            .arg(MODEL_ID)
            .arg("--embedding-dimension")
            .arg(VECTOR_DIMENSION.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(BenchmarkError::io)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BenchmarkError::invalid_config("resident_query_daemon"))?;
        let mut reader = BufReader::new(stdout);
        let endpoint = match read_endpoint(&mut reader) {
            Ok(endpoint) => endpoint,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let stdout_drain = thread::spawn(move || {
            for line in reader.lines() {
                if line.is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            endpoint,
            stdout_drain: Some(stdout_drain),
        })
    }

    pub(super) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(super) fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for ResidentDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(worker) = self.stdout_drain.take() {
            let _ = worker.join();
        }
    }
}

fn read_endpoint(reader: &mut impl BufRead) -> Result<String> {
    for _ in 0..16 {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(BenchmarkError::io)? == 0 {
            break;
        }
        if let Some(status) = line.trim().strip_prefix("ipc status endpoint: ") {
            return Ok(status.replace("/status", "/search"));
        }
    }
    Err(BenchmarkError::invalid_config("resident_query_daemon"))
}

pub(super) struct RssSampler {
    peak: Arc<AtomicU64>,
    host_cpu_total_milli_pct: Arc<AtomicU64>,
    host_cpu_peak_milli_pct: Arc<AtomicU64>,
    daemon_cpu_peak_milli_pct: Arc<AtomicU64>,
    cpu_samples: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

pub(super) struct ResourceSample {
    pub(super) daemon_rss_peak_bytes: u64,
    pub(super) host_cpu_mean_pct: f64,
    pub(super) host_cpu_peak_pct: f64,
    pub(super) daemon_cpu_peak_pct: f64,
}

impl RssSampler {
    pub(super) fn start(pid: u32) -> Self {
        let peak = Arc::new(AtomicU64::new(0));
        let host_cpu_total_milli_pct = Arc::new(AtomicU64::new(0));
        let host_cpu_peak_milli_pct = Arc::new(AtomicU64::new(0));
        let daemon_cpu_peak_milli_pct = Arc::new(AtomicU64::new(0));
        let cpu_samples = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_peak = Arc::clone(&peak);
        let worker_host_cpu_total = Arc::clone(&host_cpu_total_milli_pct);
        let worker_host_cpu_peak = Arc::clone(&host_cpu_peak_milli_pct);
        let worker_daemon_cpu_peak = Arc::clone(&daemon_cpu_peak_milli_pct);
        let worker_cpu_samples = Arc::clone(&cpu_samples);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let pid = Pid::from_u32(pid);
            let mut system = System::new_all();
            while !worker_stop.load(Ordering::Relaxed) {
                system.refresh_cpu_usage();
                system.refresh_processes_specifics(
                    ProcessesToUpdate::Some(&[pid]),
                    true,
                    ProcessRefreshKind::nothing()
                        .with_cpu()
                        .with_memory()
                        .without_tasks(),
                );
                let host_cpu = (system.global_cpu_usage() as f64 * 1_000.0) as u64;
                worker_host_cpu_total.fetch_add(host_cpu, Ordering::Relaxed);
                worker_host_cpu_peak.fetch_max(host_cpu, Ordering::Relaxed);
                worker_cpu_samples.fetch_add(1, Ordering::Relaxed);
                if let Some(process) = system.process(pid) {
                    worker_peak.fetch_max(process.memory(), Ordering::Relaxed);
                    worker_daemon_cpu_peak.fetch_max(
                        (process.cpu_usage() as f64 * 1_000.0) as u64,
                        Ordering::Relaxed,
                    );
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        Self {
            peak,
            host_cpu_total_milli_pct,
            host_cpu_peak_milli_pct,
            daemon_cpu_peak_milli_pct,
            cpu_samples,
            stop,
            worker: Some(worker),
        }
    }

    pub(super) fn finish(mut self) -> ResourceSample {
        self.stop_and_join();
        let samples = self.cpu_samples.load(Ordering::Relaxed).max(1);
        ResourceSample {
            daemon_rss_peak_bytes: self.peak.load(Ordering::Relaxed),
            host_cpu_mean_pct: self.host_cpu_total_milli_pct.load(Ordering::Relaxed) as f64
                / samples as f64
                / 1_000.0,
            host_cpu_peak_pct: self.host_cpu_peak_milli_pct.load(Ordering::Relaxed) as f64
                / 1_000.0,
            daemon_cpu_peak_pct: self.daemon_cpu_peak_milli_pct.load(Ordering::Relaxed) as f64
                / 1_000.0,
        }
    }

    fn stop_and_join(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for RssSampler {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

pub(super) fn hardware_profile() -> (&'static str, u64) {
    let mut system = System::new();
    system.refresh_memory();
    hardware_profile_for_total_memory(system.total_memory())
}

fn hardware_profile_for_total_memory(total_memory_bytes: u64) -> (&'static str, u64) {
    match total_memory_bytes {
        bytes if bytes > H1_MEMORY_CEILING_BYTES => ("H2_Aggressive", 1536),
        bytes if bytes > H0_MEMORY_CEILING_BYTES => ("H1_Balanced", 1024),
        _ => ("H0_Eco", 512),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_profile_uses_inclusive_12_and_20_gib_boundaries() {
        for (total_memory_bytes, expected_profile) in [
            (0, ("H0_Eco", 512)),
            (12 * BYTES_PER_GIB, ("H0_Eco", 512)),
            (12 * BYTES_PER_GIB + 1, ("H1_Balanced", 1024)),
            (20 * BYTES_PER_GIB, ("H1_Balanced", 1024)),
            (20 * BYTES_PER_GIB + 1, ("H2_Aggressive", 1536)),
        ] {
            assert_eq!(
                hardware_profile_for_total_memory(total_memory_bytes),
                expected_profile
            );
        }
    }
}
