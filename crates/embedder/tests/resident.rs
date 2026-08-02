use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use embedder::{
    EmbeddingBudget, EmbeddingError, EmbeddingInput, EmbeddingPriority, LocalEmbeddingCommandSpec,
    ResidentEmbeddingOwner, ResidentEmbeddingSpec, ResidentEmbeddingStatus,
};
use tempfile::TempDir;

#[test]
fn production_spec_rejects_four_threads() {
    let worker = TestWorker::compile("production_limit");
    assert!(matches!(
        ResidentEmbeddingSpec::new(worker.command()).with_intra_threads(4),
        Err(EmbeddingError::InvalidRequest)
    ));
}

#[cfg(feature = "resident-role-isolation-experiment")]
#[test]
fn role_owners_route_to_distinct_workers_and_shutdown_together() {
    let interactive_worker =
        TestWorker::compile_for_mode("role_interactive", "--resident-role-isolation-experiment");
    let bulk_worker =
        TestWorker::compile_for_mode("role_bulk", "--resident-role-isolation-experiment");
    let mut interactive_owner = interactive_worker.role_owner(/*threads*/ 3);
    let mut bulk_owner = bulk_worker.role_owner(/*threads*/ 4);
    let interactive = interactive_owner.client();
    let bulk = bulk_owner.client();
    wait_ready(&interactive);
    wait_ready(&bulk);

    interactive
        .embed_batch_with_cancel(
            EmbeddingPriority::Interactive,
            &[EmbeddingInput::query("query", "synthetic query")],
            EmbeddingBudget::new(1, 64),
            1_000,
            || false,
        )
        .unwrap();
    bulk.embed_batch_with_cancel(
        EmbeddingPriority::Background,
        &[EmbeddingInput::new("document", "synthetic passage")],
        EmbeddingBudget::new(1, 64),
        1_000,
        || false,
    )
    .unwrap();

    assert_eq!(interactive_worker.order(), ["query"]);
    assert_eq!(bulk_worker.order(), ["passage"]);
    ResidentEmbeddingOwner::shutdown_role_isolation_pair(&mut interactive_owner, &mut bulk_owner);
    wait_for_status(&interactive, ResidentEmbeddingStatus::Shutdown);
    wait_for_status(&bulk, ResidentEmbeddingStatus::Shutdown);
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
    let deadline = Instant::now() + Duration::from_secs(2);
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

    #[cfg(feature = "resident-role-isolation-experiment")]
    fn role_owner(&self, threads: usize) -> ResidentEmbeddingOwner {
        ResidentEmbeddingOwner::start(
            ResidentEmbeddingSpec::for_role_isolation_experiment(self.command(), threads).unwrap(),
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
