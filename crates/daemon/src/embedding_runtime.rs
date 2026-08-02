use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "resident-role-isolation-experiment")]
use std::fs::{self, OpenOptions};
#[cfg(feature = "resident-role-isolation-experiment")]
use std::io::Write as _;
#[cfg(feature = "resident-role-isolation-experiment")]
use std::os::unix::fs::OpenOptionsExt as _;
#[cfg(feature = "resident-role-isolation-experiment")]
use std::path::PathBuf;
#[cfg(feature = "resident-role-isolation-experiment")]
use std::sync::Mutex;

use embedder::{
    EmbeddingBudget, EmbeddingError, EmbeddingInput, EmbeddingPriority, LocalEmbeddingCommandSpec,
    ResidentEmbeddingClient, ResidentEmbeddingOwner, ResidentEmbeddingSpec,
    ResidentEmbeddingStatus,
};
use import_pipeline::{
    ImportResourcePolicy, SearchPublicationEmbeddingFailure, SearchPublicationEmbeddingInput,
    SearchPublicationEmbeddingOutput, SearchPublicationTelemetrySnapshot,
    SearchPublicationVectorization, SearchPublicationVectorizer,
};

use crate::daemon_error::{DaemonError, Result};
#[cfg(feature = "resident-role-isolation-experiment")]
use crate::run_options::ResidentRoleIsolationArm;
use crate::run_options::{usage, RunOptions};

#[cfg(feature = "resident-role-isolation-experiment")]
const EXPERIMENT_OBSERVER_ENV: &str = "RESUME_IR_RESIDENT_ROLE_ISOLATION_OBSERVER";

pub(crate) enum ResidentEmbeddingTopologyOwner {
    Shared(ResidentEmbeddingOwner),
    #[cfg(feature = "resident-role-isolation-experiment")]
    Split(SplitResidentEmbeddingOwners),
}

impl ResidentEmbeddingTopologyOwner {
    pub(crate) fn statuses(&self) -> [ResidentEmbeddingStatus; 2] {
        match self {
            Self::Shared(owner) => [owner.client().status(); 2],
            #[cfg(feature = "resident-role-isolation-experiment")]
            Self::Split(owners) => [
                owners.interactive.client().status(),
                owners.bulk.client().status(),
            ],
        }
    }
}

#[cfg(feature = "resident-role-isolation-experiment")]
pub(crate) struct SplitResidentEmbeddingOwners {
    interactive: ResidentEmbeddingOwner,
    bulk: ResidentEmbeddingOwner,
}

#[cfg(feature = "resident-role-isolation-experiment")]
impl Drop for SplitResidentEmbeddingOwners {
    fn drop(&mut self) {
        ResidentEmbeddingOwner::shutdown_role_isolation_pair(&mut self.interactive, &mut self.bulk);
    }
}

struct StartedResidentEmbeddingTopology {
    owner: ResidentEmbeddingTopologyOwner,
    interactive_client: ResidentEmbeddingClient,
    publication_client: ResidentEmbeddingClient,
}

pub(crate) fn start(options: &mut RunOptions) -> Result<Option<ResidentEmbeddingTopologyOwner>> {
    if options.embedding_command.is_none() {
        return Ok(None);
    }
    let command = options
        .embedding_command
        .clone()
        .ok_or_else(|| DaemonError::usage(usage()))?;
    let command = crate::runtime_pack::validated_embedding_command(&command)
        .map_err(|_| {
            DaemonError::configuration_invalid(
                "embedding runtime executable attestation failed before spawn",
            )
        })?
        .into_path();
    let model_id = options
        .embedding_model_id
        .as_deref()
        .ok_or_else(|| DaemonError::usage(usage()))?;
    let dimension = options
        .embedding_dimension
        .ok_or_else(|| DaemonError::usage(usage()))?;
    let command =
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), model_id, dimension)
            .map_err(DaemonError::embedding)?
            .with_timeout_ms(options.embedding_timeout_ms)
            .map_err(DaemonError::embedding)?;
    let inference_threads = ImportResourcePolicy::detect().parse_workers.get();
    #[cfg(feature = "resident-role-isolation-experiment")]
    let started = match options.resident_role_isolation_arm {
        Some(ResidentRoleIsolationArm::SharedI3B4) => start_shared(command, 3),
        Some(ResidentRoleIsolationArm::SplitI3Bulk3B4) => start_split(command, 3),
        Some(ResidentRoleIsolationArm::SplitI3Bulk4B4) => start_split(command, 4),
        None => start_shared(command, inference_threads),
    }?;
    #[cfg(not(feature = "resident-role-isolation-experiment"))]
    let started = start_shared(command, inference_threads)?;

    let publication_client = started.publication_client;
    #[cfg(feature = "resident-role-isolation-experiment")]
    let experiment_observer =
        ExperimentObserver::from_environment(options.resident_role_isolation_arm.is_some())?;
    options.search_vectorization =
        SearchPublicationVectorization::enabled(Arc::new(ResidentPublicationVectorizer {
            client: publication_client.clone(),
            timeout_ms: options.embedding_timeout_ms,
            telemetry: std::array::from_fn(|_| AtomicU64::new(0)),
            #[cfg(feature = "resident-role-isolation-experiment")]
            experiment_observer,
        }));
    options.resident_embedding = Some(started.interactive_client);
    options.publication_resident_embedding = Some(publication_client);
    Ok(Some(started.owner))
}

fn start_shared(
    command: LocalEmbeddingCommandSpec,
    intra_threads: usize,
) -> Result<StartedResidentEmbeddingTopology> {
    let owner = ResidentEmbeddingOwner::start(
        ResidentEmbeddingSpec::new(command)
            .with_intra_threads(intra_threads)
            .map_err(DaemonError::embedding)?,
    )
    .map_err(DaemonError::embedding)?;
    let client = owner.client();
    Ok(StartedResidentEmbeddingTopology {
        owner: ResidentEmbeddingTopologyOwner::Shared(owner),
        interactive_client: client.clone(),
        publication_client: client,
    })
}

#[cfg(feature = "resident-role-isolation-experiment")]
fn start_split(
    command: LocalEmbeddingCommandSpec,
    bulk_threads: usize,
) -> Result<StartedResidentEmbeddingTopology> {
    let interactive = ResidentEmbeddingOwner::start(
        ResidentEmbeddingSpec::for_role_isolation_experiment(command.clone(), 3)
            .map_err(DaemonError::embedding)?,
    )
    .map_err(DaemonError::embedding)?;
    let bulk = ResidentEmbeddingOwner::start(
        ResidentEmbeddingSpec::for_role_isolation_experiment(command, bulk_threads)
            .map_err(DaemonError::embedding)?,
    )
    .map_err(DaemonError::embedding)?;
    let interactive_client = interactive.client();
    let publication_client = bulk.client();
    Ok(StartedResidentEmbeddingTopology {
        owner: ResidentEmbeddingTopologyOwner::Split(SplitResidentEmbeddingOwners {
            interactive,
            bulk,
        }),
        interactive_client,
        publication_client,
    })
}

struct ResidentPublicationVectorizer {
    client: ResidentEmbeddingClient,
    timeout_ms: u64,
    telemetry: [AtomicU64; Metric::Count as usize],
    #[cfg(feature = "resident-role-isolation-experiment")]
    experiment_observer: Option<ExperimentObserver>,
}

#[repr(usize)]
enum Metric {
    Calls,
    Inputs,
    ActiveTokens,
    PaddedTokens,
    QueueWait,
    IpcWall,
    ChildTotal,
    Tokenize,
    Tensor,
    Onnx,
    Pool,
    Normalize,
    VectorWall,
    Count,
}

impl SearchPublicationVectorizer for ResidentPublicationVectorizer {
    fn model_id(&self) -> &str {
        self.client.model_id()
    }

    fn dimension(&self) -> usize {
        self.client.dimension()
    }

    fn max_batch_inputs(&self) -> usize {
        embedding_protocol::MAX_INPUTS
    }

    fn max_text_bytes(&self) -> usize {
        embedding_protocol::MAX_TEXT_BYTES
    }

    fn embed_batch(
        &self,
        inputs: &[SearchPublicationEmbeddingInput],
        is_cancelled: &dyn Fn() -> bool,
    ) -> std::result::Result<Vec<SearchPublicationEmbeddingOutput>, SearchPublicationEmbeddingFailure>
    {
        let resident_inputs = inputs
            .iter()
            .map(|input| EmbeddingInput::new(input.id(), input.text()))
            .collect::<Vec<_>>();
        let (outputs, telemetry) = self
            .client
            .embed_batch_with_telemetry(
                EmbeddingPriority::Background,
                &resident_inputs,
                EmbeddingBudget::new(resident_inputs.len(), embedding_protocol::MAX_TEXT_BYTES),
                self.timeout_ms,
                is_cancelled,
            )
            .map_err(|error| match error {
                EmbeddingError::Cancelled => SearchPublicationEmbeddingFailure::Cancelled,
                EmbeddingError::InvalidDimension
                | EmbeddingError::InvalidRequest
                | EmbeddingError::BudgetExceeded { .. }
                | EmbeddingError::TextBudgetExceeded { .. } => {
                    SearchPublicationEmbeddingFailure::InvalidOutput
                }
                EmbeddingError::WorkerUnavailable
                | EmbeddingError::EngineFailed
                | EmbeddingError::Overloaded
                | EmbeddingError::Timeout => SearchPublicationEmbeddingFailure::RuntimeUnavailable,
            })?;
        for (metric, value) in [
            (Metric::Calls, 1),
            (Metric::Inputs, telemetry.input_count),
            (Metric::ActiveTokens, telemetry.active_token_count),
            (Metric::PaddedTokens, telemetry.padded_token_count),
            (Metric::QueueWait, telemetry.queue_wait_us),
            (Metric::IpcWall, telemetry.ipc_wall_us),
            (Metric::ChildTotal, telemetry.child_total_us),
            (Metric::Tokenize, telemetry.tokenize_us),
            (Metric::Tensor, telemetry.tensor_us),
            (Metric::Onnx, telemetry.onnx_us),
            (Metric::Pool, telemetry.pool_us),
            (Metric::Normalize, telemetry.normalize_us),
        ] {
            self.add(metric, value);
        }
        #[cfg(feature = "resident-role-isolation-experiment")]
        if let Some(observer) = &self.experiment_observer {
            observer
                .record(telemetry.input_count, telemetry.active_token_count)
                .map_err(|_| SearchPublicationEmbeddingFailure::RuntimeUnavailable)?;
        }
        Ok(outputs
            .into_iter()
            .map(|output| {
                SearchPublicationEmbeddingOutput::new(
                    output.id(),
                    output.model_id(),
                    output.values().to_vec(),
                )
            })
            .collect())
    }

    fn telemetry_snapshot(&self) -> SearchPublicationTelemetrySnapshot {
        let get = |metric| self.telemetry[metric as usize].load(Ordering::Relaxed);
        SearchPublicationTelemetrySnapshot {
            embedding_calls: get(Metric::Calls),
            embedding_inputs: get(Metric::Inputs),
            active_token_count: get(Metric::ActiveTokens),
            padded_token_count: get(Metric::PaddedTokens),
            embedding_queue_wait_us: get(Metric::QueueWait),
            embedding_ipc_wall_us: get(Metric::IpcWall),
            embedding_child_total_us: get(Metric::ChildTotal),
            embedding_tokenize_us: get(Metric::Tokenize),
            embedding_tensor_us: get(Metric::Tensor),
            embedding_onnx_us: get(Metric::Onnx),
            embedding_pool_us: get(Metric::Pool),
            embedding_normalize_us: get(Metric::Normalize),
            vector_publication_wall_us: get(Metric::VectorWall),
        }
    }

    fn record_vector_publication_wall(&self, elapsed: Duration) {
        self.add(
            Metric::VectorWall,
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
        );
    }
}

#[cfg(feature = "resident-role-isolation-experiment")]
struct ExperimentObserver {
    path: PathBuf,
    counters: Mutex<ExperimentObserverCounters>,
}

#[cfg(feature = "resident-role-isolation-experiment")]
#[derive(Default)]
struct ExperimentObserverCounters {
    completed_calls: u64,
    completed_inputs: u64,
    active_token_count: u64,
    nonconforming_calls: u64,
}

#[cfg(feature = "resident-role-isolation-experiment")]
impl ExperimentObserver {
    fn from_environment(experiment_active: bool) -> Result<Option<Self>> {
        let Some(path) = std::env::var_os(EXPERIMENT_OBSERVER_ENV).map(PathBuf::from) else {
            return Ok(None);
        };
        if !experiment_active || !path.is_absolute() || path.parent().is_none_or(|p| !p.is_dir()) {
            return Err(DaemonError::configuration_invalid(
                "resident role-isolation observer path is invalid",
            ));
        }
        Ok(Some(Self {
            path,
            counters: Mutex::new(ExperimentObserverCounters::default()),
        }))
    }

    fn record(&self, inputs: u64, active_tokens: u64) -> std::io::Result<()> {
        let mut counters = self
            .counters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        counters.completed_calls = counters.completed_calls.saturating_add(1);
        counters.completed_inputs = counters.completed_inputs.saturating_add(inputs);
        counters.active_token_count = counters.active_token_count.saturating_add(active_tokens);
        if inputs != 4 || active_tokens != 4 * 512 {
            counters.nonconforming_calls = counters.nonconforming_calls.saturating_add(1);
        }
        let body = serde_json::json!({
            "schema_version": "resume-ir.resident-role-isolation-observer.v1",
            "completed_calls": counters.completed_calls,
            "completed_inputs": counters.completed_inputs,
            "active_token_count": counters.active_token_count,
            "nonconforming_calls": counters.nonconforming_calls,
        });
        let staging = self.path.with_extension("next");
        let mut output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&staging)?;
        serde_json::to_writer(&mut output, &body).map_err(std::io::Error::other)?;
        output.write_all(b"\n")?;
        drop(output);
        fs::rename(staging, &self.path)
    }
}

impl ResidentPublicationVectorizer {
    fn add(&self, metric: Metric, value: u64) {
        let _ = self.telemetry[metric as usize].fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| Some(current.saturating_add(value)),
        );
    }
}

#[cfg(all(test, feature = "resident-role-isolation-experiment"))]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{Duration, Instant};

    use embedder::{
        EmbeddingBudget, EmbeddingInput, EmbeddingPriority, LocalEmbeddingCommandSpec,
        ResidentEmbeddingStatus,
    };
    use tempfile::TempDir;

    use super::{start_split, ExperimentObserver, ExperimentObserverCounters};

    #[test]
    fn experiment_observer_publishes_only_bounded_aggregate_counters() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("observer.json");
        let observer = ExperimentObserver {
            path: path.clone(),
            counters: std::sync::Mutex::new(ExperimentObserverCounters::default()),
        };
        for _ in 0..7 {
            observer.record(4, 2_048).unwrap();
        }
        observer.record(1, 32).unwrap();

        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "schema_version": "resume-ir.resident-role-isolation-observer.v1",
                "completed_calls": 8,
                "completed_inputs": 29,
                "active_token_count": 14_368,
                "nonconforming_calls": 1,
            })
        );
    }

    #[test]
    fn split_topology_runs_inflight_bulk_and_interactive_work_on_distinct_residents() {
        let worker = TestWorker::compile();
        let started = start_split(worker.command(), /*bulk_threads*/ 4).unwrap();
        wait_ready(&started.owner);
        assert_eq!(worker.spawn_count(), 2);

        let publication = started.publication_client.clone();
        let bulk_request = std::thread::spawn(move || {
            publication.embed_batch_with_cancel(
                EmbeddingPriority::Background,
                &[EmbeddingInput::new("document", "synthetic passage")],
                EmbeddingBudget::new(1, 64),
                1_000,
                || false,
            )
        });
        worker.wait_for_request_count(1, Duration::from_secs(1));

        let interactive = started.interactive_client.clone();
        let query_request = std::thread::spawn(move || {
            interactive.embed_batch_with_cancel(
                EmbeddingPriority::Interactive,
                &[EmbeddingInput::query("query", "synthetic query")],
                EmbeddingBudget::new(1, 64),
                1_000,
                || false,
            )
        });
        worker.wait_for_request_count(2, Duration::from_millis(150));
        assert_eq!(bulk_request.join().unwrap().unwrap().len(), 1);
        assert_eq!(query_request.join().unwrap().unwrap().len(), 1);

        let interactive = started.interactive_client;
        let publication = started.publication_client;
        drop(started.owner);
        wait_status(&interactive, ResidentEmbeddingStatus::Shutdown);
        wait_status(&publication, ResidentEmbeddingStatus::Shutdown);
    }

    fn wait_ready(owner: &super::ResidentEmbeddingTopologyOwner) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while owner.statuses() != [ResidentEmbeddingStatus::Ready; 2] {
            assert!(
                Instant::now() < deadline,
                "split topology did not become ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_status(client: &embedder::ResidentEmbeddingClient, expected: ResidentEmbeddingStatus) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while client.status() != expected {
            assert!(Instant::now() < deadline, "resident did not stop");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    struct TestWorker {
        _directory: TempDir,
        executable: PathBuf,
    }

    impl TestWorker {
        fn compile() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let executable = directory.path().join(format!(
                "resident_worker_slow_role_isolation{}",
                std::env::consts::EXE_SUFFIX
            ));
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../embedding-protocol/tests/fixtures/resident_worker.rs");
            let original = fs::read_to_string(fixture).unwrap();
            let generated = original.replace(
                "Some(\"--resident\")",
                "Some(\"--resident-role-isolation-experiment\")",
            );
            assert_ne!(generated, original);
            let source = directory.path().join("resident_worker.rs");
            fs::write(&source, generated).unwrap();
            let status = Command::new(option_env!("RUSTC").unwrap_or("rustc"))
                .arg("--edition=2021")
                .arg(source)
                .arg("-o")
                .arg(&executable)
                .status()
                .unwrap();
            assert!(status.success());
            Self {
                _directory: directory,
                executable,
            }
        }

        fn command(&self) -> LocalEmbeddingCommandSpec {
            LocalEmbeddingCommandSpec::new(
                &self.executable,
                Vec::<String>::new(),
                "fixture-local-model",
                4,
            )
            .unwrap()
            .with_timeout_ms(2_000)
            .unwrap()
        }

        fn spawn_count(&self) -> usize {
            line_count(&self.executable.with_extension("spawns"))
        }

        fn wait_for_request_count(&self, count: usize, timeout: Duration) {
            let deadline = Instant::now() + timeout;
            while line_count(&self.executable.with_extension("requests")) < count {
                assert!(
                    Instant::now() < deadline,
                    "requests did not run concurrently"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn line_count(path: &Path) -> usize {
        fs::read_to_string(path).unwrap_or_default().lines().count()
    }
}
