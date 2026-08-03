use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "resident-embedding-pool-experiment")]
use embedder::ResidentEmbeddingPoolRole;
use embedder::{
    EmbeddingBudget, EmbeddingError, EmbeddingInput, EmbeddingPriority, EmbeddingVector,
    LocalEmbeddingCommandSpec, ResidentEmbeddingClient, ResidentEmbeddingOwner,
    ResidentEmbeddingSpec, ResidentEmbeddingStatus,
};
use import_pipeline::{
    ImportResourcePolicy, SearchPublicationEmbeddingFailure, SearchPublicationEmbeddingInput,
    SearchPublicationEmbeddingOutput, SearchPublicationTelemetrySnapshot,
    SearchPublicationVectorization, SearchPublicationVectorizer,
};

use crate::daemon_error::{DaemonError, Result};
#[cfg(feature = "resident-embedding-pool-experiment")]
use crate::run_options::ResidentEmbeddingPoolArm;
use crate::run_options::{usage, RunOptions};

pub(crate) enum ResidentEmbeddingTopologyOwner {
    Shared(ResidentEmbeddingOwner),
    #[cfg(feature = "resident-embedding-pool-experiment")]
    Pool(PoolResidentEmbeddingOwners),
}

impl ResidentEmbeddingTopologyOwner {
    pub(crate) fn all_ready(&self) -> bool {
        match self {
            Self::Shared(owner) => owner.client().status() == ResidentEmbeddingStatus::Ready,
            #[cfg(feature = "resident-embedding-pool-experiment")]
            Self::Pool(owners) => {
                owners.interactive.client().status() == ResidentEmbeddingStatus::Ready
                    && owners
                        .bulk
                        .iter()
                        .all(|owner| owner.client().status() == ResidentEmbeddingStatus::Ready)
            }
        }
    }

    pub(crate) fn any_terminal(&self) -> bool {
        let terminal = |status| {
            matches!(
                status,
                ResidentEmbeddingStatus::Unavailable | ResidentEmbeddingStatus::Shutdown
            )
        };
        match self {
            Self::Shared(owner) => terminal(owner.client().status()),
            #[cfg(feature = "resident-embedding-pool-experiment")]
            Self::Pool(owners) => {
                terminal(owners.interactive.client().status())
                    || owners
                        .bulk
                        .iter()
                        .any(|owner| terminal(owner.client().status()))
            }
        }
    }
}

#[cfg(feature = "resident-embedding-pool-experiment")]
pub(crate) struct PoolResidentEmbeddingOwners {
    interactive: ResidentEmbeddingOwner,
    bulk: Vec<ResidentEmbeddingOwner>,
}

#[cfg(feature = "resident-embedding-pool-experiment")]
impl Drop for PoolResidentEmbeddingOwners {
    fn drop(&mut self) {
        let Self { interactive, bulk } = self;
        let mut owners = Vec::with_capacity(bulk.len() + 1);
        owners.push(interactive);
        owners.extend(bulk.iter_mut());
        ResidentEmbeddingOwner::shutdown_experiment_pool(&mut owners);
    }
}

struct StartedResidentEmbeddingTopology {
    owner: ResidentEmbeddingTopologyOwner,
    interactive_client: ResidentEmbeddingClient,
    publication_clients: Vec<ResidentEmbeddingClient>,
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
    #[cfg(feature = "resident-embedding-pool-experiment")]
    let started = match selected_pool_arm(options.resident_embedding_pool_arm) {
        Some(ResidentEmbeddingPoolArm::I3Bulk1x4B4) => start_pool(command, &[4]),
        Some(ResidentEmbeddingPoolArm::I3Bulk2x2B4) => start_pool(command, &[2, 2]),
        Some(ResidentEmbeddingPoolArm::I3Bulk2x3B4) => start_pool(command, &[3, 3]),
        None => start_shared(command, inference_threads),
    }?;
    #[cfg(not(feature = "resident-embedding-pool-experiment"))]
    let started = start_shared(command, inference_threads)?;

    options.search_vectorization =
        SearchPublicationVectorization::enabled(Arc::new(ResidentPublicationVectorizer::new(
            started.publication_clients.clone(),
            options.embedding_timeout_ms,
        )));
    options.resident_embedding = Some(started.interactive_client);
    options.publication_resident_embeddings = started.publication_clients;
    Ok(Some(started.owner))
}

#[cfg(feature = "resident-embedding-pool-experiment")]
fn selected_pool_arm(
    configured: Option<ResidentEmbeddingPoolArm>,
) -> Option<ResidentEmbeddingPoolArm> {
    if configured.is_some() {
        return configured;
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some(ResidentEmbeddingPoolArm::I3Bulk2x3B4)
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        None
    }
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
        publication_clients: vec![client],
    })
}

#[cfg(feature = "resident-embedding-pool-experiment")]
fn start_pool(
    command: LocalEmbeddingCommandSpec,
    bulk_threads: &[usize],
) -> Result<StartedResidentEmbeddingTopology> {
    if !matches!(bulk_threads, [4] | [2, 2] | [3, 3]) {
        return Err(DaemonError::embedding(EmbeddingError::InvalidRequest));
    }
    let interactive = ResidentEmbeddingOwner::start(
        ResidentEmbeddingSpec::for_pool_experiment(
            command.clone(),
            ResidentEmbeddingPoolRole::Interactive,
            /*intra_threads*/ 3,
        )
        .map_err(DaemonError::embedding)?,
    )
    .map_err(DaemonError::embedding)?;
    let mut bulk = Vec::with_capacity(bulk_threads.len());
    for &threads in bulk_threads {
        bulk.push(
            ResidentEmbeddingOwner::start(
                ResidentEmbeddingSpec::for_pool_experiment(
                    command.clone(),
                    ResidentEmbeddingPoolRole::Bulk,
                    threads,
                )
                .map_err(DaemonError::embedding)?,
            )
            .map_err(DaemonError::embedding)?,
        );
    }
    let interactive_client = interactive.client();
    let publication_clients = bulk.iter().map(ResidentEmbeddingOwner::client).collect();
    Ok(StartedResidentEmbeddingTopology {
        owner: ResidentEmbeddingTopologyOwner::Pool(PoolResidentEmbeddingOwners {
            interactive,
            bulk,
        }),
        interactive_client,
        publication_clients,
    })
}

struct ResidentPublicationVectorizer {
    clients: Vec<ResidentEmbeddingClient>,
    timeout_ms: u64,
    telemetry: [AtomicU64; Metric::Count as usize],
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
        self.primary_client().model_id()
    }

    fn dimension(&self) -> usize {
        self.primary_client().dimension()
    }

    fn max_batch_inputs(&self) -> usize {
        embedding_protocol::MAX_INPUTS * self.clients.len()
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
        self.embed_resident_batch(&resident_inputs, is_cancelled)
            .map(|outputs| {
                outputs
                    .into_iter()
                    .map(|output| {
                        SearchPublicationEmbeddingOutput::new(
                            output.id(),
                            output.model_id(),
                            output.values().to_vec(),
                        )
                    })
                    .collect()
            })
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

impl ResidentPublicationVectorizer {
    fn new(clients: Vec<ResidentEmbeddingClient>, timeout_ms: u64) -> Self {
        assert!(
            !clients.is_empty(),
            "resident vectorizer requires one client"
        );
        Self {
            clients,
            timeout_ms,
            telemetry: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    fn primary_client(&self) -> &ResidentEmbeddingClient {
        &self.clients[0]
    }

    fn embed_resident_batch(
        &self,
        inputs: &[EmbeddingInput],
        is_cancelled: &dyn Fn() -> bool,
    ) -> std::result::Result<Vec<EmbeddingVector>, SearchPublicationEmbeddingFailure> {
        if inputs.is_empty() || inputs.len() > self.max_batch_inputs() {
            return Err(SearchPublicationEmbeddingFailure::InvalidOutput);
        }
        #[cfg(feature = "resident-embedding-pool-experiment")]
        if self.clients.len() > 1 {
            return self.embed_pool_batch(inputs, is_cancelled);
        }
        self.primary_client()
            .embed_batch_with_telemetry(
                EmbeddingPriority::Background,
                inputs,
                EmbeddingBudget::new(inputs.len(), embedding_protocol::MAX_TEXT_BYTES),
                self.timeout_ms,
                is_cancelled,
            )
            .map(|(outputs, telemetry)| {
                self.record_telemetry(&telemetry);
                outputs
            })
            .map_err(Self::map_error)
    }

    #[cfg(feature = "resident-embedding-pool-experiment")]
    fn embed_pool_batch(
        &self,
        inputs: &[EmbeddingInput],
        is_cancelled: &dyn Fn() -> bool,
    ) -> std::result::Result<Vec<EmbeddingVector>, SearchPublicationEmbeddingFailure> {
        let mut requests = Vec::with_capacity(self.clients.len());
        for (ordinal, batch) in inputs.chunks(embedding_protocol::MAX_INPUTS).enumerate() {
            let client = self
                .clients
                .get(ordinal)
                .ok_or(SearchPublicationEmbeddingFailure::InvalidOutput)?;
            requests.push(
                client
                    .enqueue_batch_with_telemetry(
                        EmbeddingPriority::Background,
                        batch,
                        EmbeddingBudget::new(batch.len(), embedding_protocol::MAX_TEXT_BYTES),
                        self.timeout_ms,
                    )
                    .map_err(Self::map_error)?,
            );
        }
        let mut outputs = Vec::with_capacity(inputs.len());
        for request in requests {
            let (batch, telemetry) = request
                .wait_with_cancel(is_cancelled)
                .map_err(Self::map_error)?;
            self.record_telemetry(&telemetry);
            outputs.extend(batch);
        }
        Ok(outputs)
    }

    fn record_telemetry(&self, telemetry: &embedding_protocol::EmbeddingTelemetry) {
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
    }

    fn map_error(error: EmbeddingError) -> SearchPublicationEmbeddingFailure {
        match error {
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
        }
    }

    fn add(&self, metric: Metric, value: u64) {
        let _ = self.telemetry[metric as usize].fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| Some(current.saturating_add(value)),
        );
    }
}

#[cfg(all(test, feature = "resident-embedding-pool-experiment"))]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use embedder::{
        EmbeddingBudget, EmbeddingInput, EmbeddingPriority, LocalEmbeddingCommandSpec,
        ResidentEmbeddingStatus,
    };
    use import_pipeline::{SearchPublicationEmbeddingInput, SearchPublicationVectorizer as _};
    use tempfile::TempDir;

    use super::{selected_pool_arm, start_pool, ResidentPublicationVectorizer};

    #[test]
    fn production_default_selects_the_winning_pool_only_on_macos_arm() {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert_eq!(
            selected_pool_arm(None),
            Some(super::ResidentEmbeddingPoolArm::I3Bulk2x3B4)
        );
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        assert_eq!(selected_pool_arm(None), None);

        assert_eq!(
            selected_pool_arm(Some(super::ResidentEmbeddingPoolArm::I3Bulk2x2B4)),
            Some(super::ResidentEmbeddingPoolArm::I3Bulk2x2B4)
        );
    }

    #[test]
    fn two_bulk_residents_run_complete_b4s_concurrently_and_reassemble_in_order() {
        let worker = TestWorker::compile();
        let started = start_pool(worker.command(), &[2, 2]).unwrap();
        wait_ready(&started.owner);
        assert_eq!(worker.spawn_count(), 3);

        let vectorizer = Arc::new(ResidentPublicationVectorizer::new(
            started.publication_clients.clone(),
            /*timeout_ms*/ 2_000,
        ));
        assert_eq!(vectorizer.max_batch_inputs(), 8);
        let inputs = (0..8)
            .map(|ordinal| {
                SearchPublicationEmbeddingInput::new(
                    format!("input-{ordinal}"),
                    format!("synthetic passage {ordinal}"),
                )
            })
            .collect::<Vec<_>>();
        let bulk_vectorizer = Arc::clone(&vectorizer);
        let bulk = std::thread::spawn(move || bulk_vectorizer.embed_batch(&inputs, &|| false));
        worker.wait_for_request_count(2);

        let query = started
            .interactive_client
            .embed_batch_with_cancel(
                EmbeddingPriority::Interactive,
                &[EmbeddingInput::query("query", "synthetic query")],
                EmbeddingBudget::new(1, 64),
                2_000,
                || false,
            )
            .unwrap();
        let outputs = bulk.join().unwrap().unwrap();

        assert_eq!(query[0].id(), "query");
        assert_eq!(worker.request_count(), 3);
        assert_eq!(
            outputs.iter().map(|output| output.id()).collect::<Vec<_>>(),
            (0..8)
                .map(|ordinal| format!("input-{ordinal}"))
                .collect::<Vec<_>>()
        );
        let telemetry = vectorizer.telemetry_snapshot();
        assert_eq!(telemetry.embedding_calls, 2);

        let clients = std::iter::once(started.interactive_client)
            .chain(started.publication_clients)
            .collect::<Vec<_>>();
        drop(started.owner);
        for client in clients {
            wait_status(&client, ResidentEmbeddingStatus::Shutdown);
        }
    }

    fn wait_ready(owner: &super::ResidentEmbeddingTopologyOwner) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !owner.all_ready() {
            assert!(
                Instant::now() < deadline,
                "resident pool did not become ready"
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
                "resident_worker_parallel_barrier{}",
                std::env::consts::EXE_SUFFIX
            ));
            let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../embedding-protocol/tests/fixtures/resident_worker.rs");
            let original = fs::read_to_string(fixture).unwrap();
            let generated = original
                .replace(
                    "Some(\"--resident\")",
                    "Some(\"--resident-embedding-pool-experiment\")",
                )
                .replace(
                    "append(&executable.with_extension(\"requests\"), b\"request\\n\");",
                    "let request_path = executable.with_extension(\"requests\");\n        append(&request_path, b\"request\\n\");\n        if name.contains(\"parallel_barrier\") && !payload.contains(\"\\\"role\\\":\\\"query\\\"\") {\n            while line_count(&request_path) < 3 {\n                thread::sleep(Duration::from_millis(10));\n            }\n        }",
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
            .with_timeout_ms(10_000)
            .unwrap()
        }

        fn spawn_count(&self) -> usize {
            line_count(&self.executable.with_extension("spawns"))
        }

        fn request_count(&self) -> usize {
            line_count(&self.executable.with_extension("requests"))
        }

        fn wait_for_request_count(&self, expected: usize) {
            let deadline = Instant::now() + Duration::from_secs(2);
            while self.request_count() < expected {
                assert!(
                    Instant::now() < deadline,
                    "complete B4 requests did not enter distinct residents"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    fn line_count(path: &Path) -> usize {
        fs::read_to_string(path).unwrap_or_default().lines().count()
    }
}
