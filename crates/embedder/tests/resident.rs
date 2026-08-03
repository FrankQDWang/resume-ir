use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(feature = "resident-embedding-pool-experiment")]
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(feature = "resident-embedding-pool-experiment")]
use embedder::ResidentEmbeddingPoolRole;
use embedder::{
    EmbeddingBudget, EmbeddingError, EmbeddingInput, EmbeddingPriority, LocalEmbeddingCommandSpec,
    ResidentEmbeddingOwner, ResidentEmbeddingSpec, ResidentEmbeddingStatus,
};
use tempfile::TempDir;

#[cfg(feature = "resident-embedding-pool-experiment")]
use embedder::ResidentEmbeddingTelemetryObserver;

#[test]
fn production_spec_rejects_four_threads() {
    let worker = TestWorker::compile("production_limit");
    assert!(matches!(
        ResidentEmbeddingSpec::new(worker.command()).with_intra_threads(4),
        Err(EmbeddingError::InvalidRequest)
    ));
}

#[cfg(feature = "resident-embedding-pool-experiment")]
#[test]
fn pool_requests_enter_distinct_residents_and_all_owners_shutdown_together() {
    let first_worker =
        TestWorker::compile_for_mode("pool_first_slow", "--resident-embedding-pool-experiment");
    let second_worker =
        TestWorker::compile_for_mode("pool_second_slow", "--resident-embedding-pool-experiment");
    let mut first_owner = first_worker.pool_owner(/*threads*/ 4);
    let mut second_owner = second_worker.pool_owner(/*threads*/ 3);
    let first = first_owner.client();
    let second = second_owner.client();
    wait_for_status_with_timeout(
        &first,
        ResidentEmbeddingStatus::Ready,
        Duration::from_secs(10),
    );
    wait_for_status_with_timeout(
        &second,
        ResidentEmbeddingStatus::Ready,
        Duration::from_secs(10),
    );

    let first_request = first
        .enqueue_batch_with_telemetry(
            EmbeddingPriority::Background,
            &[EmbeddingInput::new("first", "synthetic passage one")],
            EmbeddingBudget::new(1, 64),
            2_000,
        )
        .unwrap();
    let second_request = second
        .enqueue_batch_with_telemetry(
            EmbeddingPriority::Background,
            &[EmbeddingInput::new("second", "synthetic passage two")],
            EmbeddingBudget::new(1, 64),
            2_000,
        )
        .unwrap();
    first_worker.wait_for_request_count(1);
    second_worker.wait_for_request_count(1);

    let first_vectors = first_request.wait_with_cancel(|| false).unwrap().0;
    let second_vectors = second_request.wait_with_cancel(|| false).unwrap().0;
    assert_eq!(first_vectors[0].id(), "first");
    assert_eq!(second_vectors[0].id(), "second");

    let cancelled_request = first
        .enqueue_batch_with_telemetry(
            EmbeddingPriority::Background,
            &[EmbeddingInput::new(
                "cancelled",
                "synthetic cancelled passage",
            )],
            EmbeddingBudget::new(1, 64),
            2_000,
        )
        .unwrap();
    first_worker.wait_for_request_count(2);
    drop(cancelled_request);
    first_worker.wait_for_ready_generation(2);
    assert_eq!(first.status(), ResidentEmbeddingStatus::Ready);

    let mut owners = [&mut first_owner, &mut second_owner];
    ResidentEmbeddingOwner::shutdown_experiment_pool(&mut owners);
    wait_for_status(&first, ResidentEmbeddingStatus::Shutdown);
    wait_for_status(&second, ResidentEmbeddingStatus::Shutdown);
}

#[cfg(feature = "resident-embedding-pool-experiment")]
#[test]
fn pool_observer_receives_only_completed_content_free_telemetry() {
    let worker =
        TestWorker::compile_for_mode("pool_observer", "--resident-embedding-pool-experiment");
    let owner = worker.pool_owner(/*threads*/ 3);
    let observer = Arc::new(RecordingObserver::default());
    let client = owner
        .client()
        .with_pool_experiment_telemetry_observer(observer.clone());
    wait_ready(&client);

    client
        .embed_batch_with_cancel(
            EmbeddingPriority::Interactive,
            &[EmbeddingInput::query("query", "synthetic query")],
            EmbeddingBudget::new(1, 64),
            1_000,
            || false,
        )
        .unwrap();
    client
        .enqueue_batch_with_telemetry(
            EmbeddingPriority::Background,
            &[EmbeddingInput::new("passage", "synthetic passage")],
            EmbeddingBudget::new(1, 64),
            1_000,
        )
        .unwrap()
        .wait_with_cancel(|| false)
        .unwrap();

    let observations = observer.0.lock().unwrap();
    assert_eq!(observations.len(), 2);
    assert_eq!(observations[0].0, EmbeddingPriority::Interactive);
    assert_eq!(observations[1].0, EmbeddingPriority::Background);
    assert_eq!(observations[0].1.input_count, 0);
    assert_eq!(observations[1].1.input_count, 0);
}

#[cfg(feature = "resident-embedding-pool-experiment")]
#[derive(Default)]
struct RecordingObserver(Mutex<Vec<(EmbeddingPriority, embedding_protocol::EmbeddingTelemetry)>>);

#[cfg(feature = "resident-embedding-pool-experiment")]
impl ResidentEmbeddingTelemetryObserver for RecordingObserver {
    fn observe(
        &self,
        priority: EmbeddingPriority,
        telemetry: &embedding_protocol::EmbeddingTelemetry,
    ) -> Result<(), EmbeddingError> {
        self.0.lock().unwrap().push((priority, *telemetry));
        Ok(())
    }
}

#[test]
fn repeated_requests_reuse_one_generation_and_keep_payloads_redacted() {
    let worker = TestWorker::compile("fast");
    let owner = worker.owner();
    let client = owner.client();
    wait_ready(&client);
    let input = EmbeddingInput::query("local-id", "synthetic private query");
    for _ in 0..2 {
        let vectors = client
            .embed_batch_with_cancel(
                EmbeddingPriority::Interactive,
                std::slice::from_ref(&input),
                EmbeddingBudget::new(1, 64),
                1_000,
                || false,
            )
            .unwrap();
        assert_eq!(vectors[0].values(), &[1.0, 0.0, 0.0, 0.0]);
    }
    assert_eq!(worker.spawn_count(), 1);
    assert!(!format!("{client:?} {input:?}").contains("synthetic private query"));
}

#[test]
fn timeout_reaps_the_generation_and_the_next_request_recovers() {
    let worker = TestWorker::compile("slow");
    let owner = worker.owner();
    let client = owner.client();
    wait_ready(&client);
    let input = EmbeddingInput::query("local-id", "synthetic timeout query");
    assert!(matches!(
        client.embed_batch_with_cancel(
            EmbeddingPriority::Interactive,
            std::slice::from_ref(&input),
            EmbeddingBudget::new(1, 64),
            1_000,
            || true,
        ),
        Err(EmbeddingError::Cancelled)
    ));
    assert!(matches!(
        client.embed_batch_with_cancel(
            EmbeddingPriority::Interactive,
            std::slice::from_ref(&input),
            EmbeddingBudget::new(1, 64),
            30,
            || false,
        ),
        Err(EmbeddingError::Timeout)
    ));
    let vectors = client
        .embed_batch_with_cancel(
            EmbeddingPriority::Interactive,
            &[input],
            EmbeddingBudget::new(1, 64),
            1_000,
            || false,
        )
        .unwrap();
    assert_eq!(vectors.len(), 1);
    assert!(worker.spawn_count() >= 2);
}

#[test]
fn child_exit_restarts_before_a_later_request() {
    let worker = TestWorker::compile("crash_once_barrier_restart");
    let owner = worker.owner();
    let client = owner.client();
    wait_ready(&client);
    let input = EmbeddingInput::new("local-id", "synthetic passage");
    assert!(matches!(
        client.embed_batch_with_cancel(
            EmbeddingPriority::Background,
            std::slice::from_ref(&input),
            EmbeddingBudget::new(1, 64),
            1_000,
            || false,
        ),
        Err(EmbeddingError::EngineFailed)
    ));
    assert_eq!(worker.ready_count(), 1);
    assert_eq!(client.status(), ResidentEmbeddingStatus::Restarting);
    worker.release_restart();
    worker.wait_for_ready_generation(2);
    assert_eq!(client.status(), ResidentEmbeddingStatus::Ready);
    assert_eq!(
        client
            .embed_batch_with_cancel(
                EmbeddingPriority::Interactive,
                &[input],
                EmbeddingBudget::new(1, 64),
                1_000,
                || false,
            )
            .unwrap()
            .len(),
        1
    );
    assert!(worker.spawn_count() >= 2);
}

#[cfg(unix)]
#[test]
fn failed_eager_restart_publishes_unavailable_and_preserves_a_later_retry() {
    let worker = TestWorker::compile("remove_before_crash");
    let owner = worker.owner();
    let client = owner.client();
    wait_ready(&client);
    let input = EmbeddingInput::new("local-id", "synthetic passage");
    assert!(matches!(
        client.embed_batch_with_cancel(
            EmbeddingPriority::Background,
            std::slice::from_ref(&input),
            EmbeddingBudget::new(1, 64),
            1_000,
            || false,
        ),
        Err(EmbeddingError::EngineFailed)
    ));
    wait_for_status(&client, ResidentEmbeddingStatus::Unavailable);
    assert!(matches!(
        client.embed_batch_with_cancel(
            EmbeddingPriority::Interactive,
            &[input],
            EmbeddingBudget::new(1, 64),
            1_000,
            || false,
        ),
        Err(EmbeddingError::WorkerUnavailable)
    ));
    assert_eq!(client.status(), ResidentEmbeddingStatus::Unavailable);
}

#[test]
fn owner_shutdown_interrupts_inference_and_joins_the_runtime() {
    let worker = TestWorker::compile("slow_shutdown");
    let owner = worker.owner();
    let client = owner.client();
    wait_ready(&client);
    let request = std::thread::spawn(move || {
        client.embed_batch_with_cancel(
            EmbeddingPriority::Interactive,
            &[EmbeddingInput::query(
                "local-id",
                "synthetic shutdown query",
            )],
            EmbeddingBudget::new(1, 64),
            2_000,
            || false,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    while worker.request_count() == 0 {
        assert!(
            Instant::now() < deadline,
            "request did not reach resident worker"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(owner);
    assert!(matches!(
        request.join().unwrap(),
        Err(EmbeddingError::WorkerUnavailable)
    ));
}

#[test]
fn interactive_queue_is_selected_before_waiting_background_work() {
    let worker = TestWorker::compile("slow_priority");
    let owner = worker.owner();
    let client = owner.client();
    wait_ready(&client);
    let first_client = client.clone();
    let first = std::thread::spawn(move || {
        first_client.embed_batch_with_cancel(
            EmbeddingPriority::Background,
            &[EmbeddingInput::new("background-1", "synthetic passage one")],
            EmbeddingBudget::new(1, 64),
            2_000,
            || false,
        )
    });
    let deadline = Instant::now() + Duration::from_secs(1);
    while worker.request_count() == 0 {
        assert!(Instant::now() < deadline, "first request did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    let second_client = client.clone();
    let second = std::thread::spawn(move || {
        second_client.embed_batch_with_telemetry(
            EmbeddingPriority::Background,
            &[EmbeddingInput::new("background-2", "synthetic passage two")],
            EmbeddingBudget::new(1, 64),
            2_000,
            || false,
        )
    });
    std::thread::sleep(Duration::from_millis(20));
    let interactive = std::thread::spawn(move || {
        client.embed_batch_with_cancel(
            EmbeddingPriority::Interactive,
            &[EmbeddingInput::query("query", "synthetic query")],
            EmbeddingBudget::new(1, 64),
            2_000,
            || false,
        )
    });
    first.join().unwrap().unwrap();
    interactive.join().unwrap().unwrap();
    let (vectors, telemetry) = second.join().unwrap().unwrap();
    assert_eq!(vectors.len(), 1);
    assert!(telemetry.queue_wait_us >= 100_000);
    assert!(telemetry.ipc_wall_us >= 100_000);
    assert_eq!(
        embedding_protocol::EmbeddingTelemetry {
            queue_wait_us: 0,
            ipc_wall_us: 0,
            ..telemetry
        },
        Default::default()
    );
    assert_eq!(worker.order(), ["passage", "query", "passage"]);
}

fn wait_ready(client: &embedder::ResidentEmbeddingClient) {
    wait_for_status(client, ResidentEmbeddingStatus::Ready);
}

fn wait_for_status(client: &embedder::ResidentEmbeddingClient, expected: ResidentEmbeddingStatus) {
    wait_for_status_with_timeout(client, expected, Duration::from_secs(2));
}

fn wait_for_status_with_timeout(
    client: &embedder::ResidentEmbeddingClient,
    expected: ResidentEmbeddingStatus,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    while client.status() != expected {
        assert!(
            Instant::now() < deadline,
            "resident worker did not reach expected status"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct TestWorker {
    _directory: TempDir,
    executable: PathBuf,
}

impl TestWorker {
    fn compile(behavior: &str) -> Self {
        Self::compile_for_mode(behavior, "--resident")
    }

    fn compile_for_mode(behavior: &str, expected_mode: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join(format!(
            "resident_worker_{behavior}{}",
            std::env::consts::EXE_SUFFIX
        ));
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../embedding-protocol/tests/fixtures/resident_worker.rs");
        let original = fs::read_to_string(fixture).unwrap();
        let generated = if expected_mode == "--resident" {
            original
        } else {
            let generated = original.replace(
                "Some(\"--resident\")",
                &format!("Some(\"{expected_mode}\")"),
            );
            assert_ne!(generated, original);
            generated
        };
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

    fn owner(&self) -> ResidentEmbeddingOwner {
        ResidentEmbeddingOwner::start(
            ResidentEmbeddingSpec::new(self.command())
                .with_intra_threads(1)
                .unwrap(),
        )
        .unwrap()
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

    #[cfg(feature = "resident-embedding-pool-experiment")]
    fn pool_owner(&self, threads: usize) -> ResidentEmbeddingOwner {
        ResidentEmbeddingOwner::start(
            ResidentEmbeddingSpec::for_pool_experiment(
                self.command().with_timeout_ms(10_000).unwrap(),
                ResidentEmbeddingPoolRole::Bulk,
                threads,
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn spawn_count(&self) -> usize {
        fs::read_to_string(self.executable.with_extension("spawns"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn request_count(&self) -> usize {
        fs::read_to_string(self.executable.with_extension("requests"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    #[cfg(feature = "resident-embedding-pool-experiment")]
    fn wait_for_request_count(&self, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while self.request_count() < expected {
            assert!(
                Instant::now() < deadline,
                "request did not reach resident worker"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn ready_count(&self) -> usize {
        fs::read_to_string(self.executable.with_extension("ready"))
            .unwrap_or_default()
            .lines()
            .count()
    }

    fn release_restart(&self) {
        fs::write(
            self.executable.with_extension("release_restart"),
            b"release",
        )
        .unwrap();
    }

    fn wait_for_ready_generation(&self, generation: usize) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.ready_count() < generation {
            assert!(
                Instant::now() < deadline,
                "resident worker did not publish the expected ready generation"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn order(&self) -> Vec<String> {
        fs::read_to_string(self.executable.with_extension("order"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}
