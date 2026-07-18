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
    let worker = TestWorker::compile("crash_once");
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
        second_client.embed_batch_with_cancel(
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
    second.join().unwrap().unwrap();
    assert_eq!(worker.order(), ["passage", "query", "passage"]);
}

fn wait_ready(client: &embedder::ResidentEmbeddingClient) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while client.status() != ResidentEmbeddingStatus::Ready {
        assert!(
            Instant::now() < deadline,
            "resident worker did not become ready"
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
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join(format!(
            "resident_worker_{behavior}{}",
            std::env::consts::EXE_SUFFIX
        ));
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../embedding-protocol/tests/fixtures/resident_worker.rs");
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
        let command = LocalEmbeddingCommandSpec::new(
            &self.executable,
            Vec::<String>::new(),
            "fixture-local-model",
            4,
        )
        .unwrap()
        .with_timeout_ms(2_000)
        .unwrap();
        ResidentEmbeddingOwner::start(
            ResidentEmbeddingSpec::new(command)
                .with_intra_threads(1)
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

    fn order(&self) -> Vec<String> {
        fs::read_to_string(self.executable.with_extension("order"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}
