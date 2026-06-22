use std::path::PathBuf;
#[cfg(unix)]
use std::sync::{Arc, Barrier};
#[cfg(unix)]
use std::sync::{Mutex, MutexGuard, OnceLock};
#[cfg(unix)]
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use embedder::{
    DeterministicTestEmbedder, Embedder, EmbeddingBudget, EmbeddingError, EmbeddingInput,
    LocalEmbeddingCommandEmbedder, LocalEmbeddingCommandSpec,
};

#[cfg(unix)]
fn local_embedding_process_test_lock() -> MutexGuard<'static, ()> {
    static LOCAL_EMBEDDING_PROCESS_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCAL_EMBEDDING_PROCESS_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn exposes_embedder_crate_identity() {
    assert_eq!(embedder::crate_name(), "embedder");
}

#[test]
fn deterministic_test_embedder_is_stable_and_budgeted_without_text_leakage() {
    let embedder = DeterministicTestEmbedder::new("test-lexical-hash", 8).unwrap();
    let inputs = [
        EmbeddingInput::new("doc_java", "Java Spring Cloud platform"),
        EmbeddingInput::new("doc_rust", "Rust search index"),
    ];

    let vectors = embedder
        .embed_batch(&inputs, EmbeddingBudget::new(2, 128))
        .unwrap();
    let repeated = embedder
        .embed_batch(&inputs, EmbeddingBudget::new(2, 128))
        .unwrap();

    assert_eq!(vectors, repeated);
    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0].id(), "doc_java");
    assert_eq!(vectors[0].model_id(), "test-lexical-hash");
    assert_eq!(vectors[0].values().len(), 8);
    assert!(vectors[0].values().iter().any(|value| *value != 0.0));
    assert!(!format!("{:?}", inputs[0]).contains("Java"));
    assert!(!format!("{:?}", vectors[0]).contains("0."));

    let error = embedder
        .embed_batch(&inputs, EmbeddingBudget::new(1, 128))
        .unwrap_err();
    assert!(!format!("{error:?}").contains("Java"));
}

#[cfg(unix)]
#[test]
fn local_command_embedder_runs_configured_binary_and_parses_structured_vectors() {
    let _lock = local_embedding_process_test_lock();
    let command = write_fixture_executable(
        "fixture-embedding-command",
        r#"#!/bin/sh
input_size="$(wc -c < "$RESUME_IR_EMBEDDING_INPUT_PATH" | tr -d ' ')"
if [ ! -s "$RESUME_IR_EMBEDDING_INPUT_PATH" ]; then
  exit 7
fi
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=3\n'
printf 'vector=doc_java\t0.5,0.5,0.70710677\n'
printf 'vector=doc_rust\t0.0,1.0,0.0\n'
printf 'metadata=input_bytes:%s\n' "$input_size"
"#,
    );
    let embedder = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), "fixture-local-model", 3)
            .unwrap()
            .with_timeout_ms(5_000)
            .unwrap(),
    );
    let inputs = [
        EmbeddingInput::new("doc_java", "PRIVATE Java Spring Cloud"),
        EmbeddingInput::new("doc_rust", "PRIVATE Rust search index"),
    ];

    let vectors = embedder
        .embed_batch(&inputs, EmbeddingBudget::new(2, 256))
        .unwrap();

    assert_eq!(vectors.len(), 2);
    assert_eq!(vectors[0].id(), "doc_java");
    assert_eq!(vectors[0].model_id(), "fixture-local-model");
    assert_eq!(vectors[0].values(), &[0.5, 0.5, 0.70710677]);
    assert_eq!(vectors[1].id(), "doc_rust");
    assert_eq!(vectors[1].values(), &[0.0, 1.0, 0.0]);
    assert!(!format!("{:?}", vectors[0]).contains("PRIVATE"));
}

#[cfg(unix)]
#[test]
fn local_command_embedder_handles_parallel_requests_without_temp_dir_collision() {
    let _lock = local_embedding_process_test_lock();
    let command = write_fixture_executable(
        "fixture-embedding-parallel",
        r#"#!/bin/sh
if [ ! -s "$RESUME_IR_EMBEDDING_INPUT_PATH" ]; then
  exit 7
fi
printf 'resume-ir-embedding-v1\n'
printf 'model_id=fixture-local-model\n'
printf 'dimension=2\n'
printf 'vector=doc_parallel\t1.0,0.0\n'
"#,
    );
    let embedder = Arc::new(LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), "fixture-local-model", 2)
            .unwrap()
            .with_timeout_ms(5_000)
            .unwrap(),
    ));
    let workers = 24;
    let barrier = Arc::new(Barrier::new(workers));
    let handles = (0..workers)
        .map(|_| {
            let embedder = Arc::clone(&embedder);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                embedder.embed_batch(
                    &[EmbeddingInput::new("doc_parallel", "PRIVATE parallel text")],
                    EmbeddingBudget::new(1, 128),
                )
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        let vectors = handle.join().unwrap().unwrap();
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].values(), &[1.0, 0.0]);
        assert!(!format!("{:?}", vectors[0]).contains("PRIVATE"));
    }
}

#[cfg(unix)]
#[test]
fn local_command_embedder_rejects_missing_binary_and_bad_output_without_payload_leaks() {
    let _lock = local_embedding_process_test_lock();
    let missing = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(
            "/definitely/missing/resume-ir-embedding-command",
            Vec::<String>::new(),
            "fixture-local-model",
            3,
        )
        .unwrap()
        .with_timeout_ms(500)
        .unwrap(),
    );
    let input = [EmbeddingInput::new("doc_private", "PRIVATE embedding text")];
    let missing_error = missing
        .embed_batch(&input, EmbeddingBudget::new(1, 128))
        .unwrap_err();
    assert_eq!(missing_error, EmbeddingError::WorkerUnavailable);
    assert!(!format!("{missing_error:?}").contains("PRIVATE"));

    let malformed_command = write_fixture_executable(
        "fixture-embedding-malformed",
        r#"#!/bin/sh
printf 'not the schema\nPRIVATE embedding text\n'
"#,
    );
    let malformed = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(
            malformed_command,
            Vec::<String>::new(),
            "fixture-local-model",
            3,
        )
        .unwrap()
        .with_timeout_ms(5_000)
        .unwrap(),
    );
    let malformed_error = malformed
        .embed_batch(&input, EmbeddingBudget::new(1, 128))
        .unwrap_err();
    assert_eq!(malformed_error, EmbeddingError::EngineFailed);
    assert!(!format!("{malformed_error:?}").contains("PRIVATE"));
}

#[cfg(unix)]
#[test]
fn local_command_embedder_times_out_and_keeps_input_file_private() {
    let _lock = local_embedding_process_test_lock();
    let permission_marker = inputs_temp_dir_root().join("permissions.txt");
    std::fs::create_dir_all(permission_marker.parent().unwrap()).unwrap();
    let slow_command = write_fixture_executable(
        "fixture-embedding-slow",
        r#"#!/bin/sh
permissions="$(stat -c '%a' "$RESUME_IR_EMBEDDING_INPUT_PATH" 2>/dev/null || stat -f '%Lp' "$RESUME_IR_EMBEDDING_INPUT_PATH")"
printf '%s' "$permissions" > "$1"
sleep 5
"#,
    );
    let embedder = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(
            slow_command,
            [permission_marker.to_string_lossy().into_owned()],
            "fixture-model",
            2,
        )
        .unwrap()
        .with_timeout_ms(1_000)
        .unwrap(),
    );
    let error = embedder
        .embed_batch(
            &[EmbeddingInput::new("doc_private", "PRIVATE timeout text")],
            EmbeddingBudget::new(1, 128),
        )
        .unwrap_err();
    assert_eq!(error, EmbeddingError::Timeout);
    assert_eq!(std::fs::read_to_string(permission_marker).unwrap(), "600");
    assert!(!format!("{error:?}").contains("PRIVATE"));
}

#[cfg(unix)]
#[test]
fn local_command_embedder_terminates_descendants_that_keep_output_pipes_open() {
    let _lock = local_embedding_process_test_lock();
    let command = write_fixture_executable(
        "fixture-embedding-descendant",
        r#"#!/bin/sh
(trap "" HUP; sleep 2; printf 'resume-ir-embedding-v1\nmodel_id=fixture-model\ndimension=2\nvector=doc_private\t1,0\n') &
sleep 2
"#,
    );
    let embedder = LocalEmbeddingCommandEmbedder::new(
        LocalEmbeddingCommandSpec::new(command, Vec::<String>::new(), "fixture-model", 2)
            .unwrap()
            .with_timeout_ms(50)
            .unwrap(),
    );
    let started_at = Instant::now();
    let error = embedder
        .embed_batch(
            &[EmbeddingInput::new(
                "doc_private",
                "PRIVATE descendant text",
            )],
            EmbeddingBudget::new(1, 128),
        )
        .unwrap_err();

    assert_eq!(error, EmbeddingError::Timeout);
    assert!(
        started_at.elapsed() < Duration::from_millis(750),
        "timeout returned only after descendant closed inherited pipes"
    );
    assert!(!format!("{error:?}").contains("PRIVATE"));
}

#[cfg(unix)]
fn write_fixture_executable(name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let directory = inputs_temp_dir_root();
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    std::fs::write(&path, body).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
fn inputs_temp_dir_root() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("resume-ir-embedder-test-{unique}"))
}
