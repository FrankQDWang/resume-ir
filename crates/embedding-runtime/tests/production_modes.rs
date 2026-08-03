use std::process::{Command, Output};

use tempfile::TempDir;

#[test]
fn production_resident_rejects_four_threads_before_loading_the_pack() {
    let output = run_resident(&["--resident"], /*threads*/ 4);

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("environment is invalid"));
}

#[cfg(feature = "resident-embedding-pool-experiment")]
#[test]
fn experiment_resident_accepts_four_threads_before_loading_the_pack() {
    let output = run_resident(
        &["--resident-embedding-pool-experiment"],
        /*threads*/ 4,
    );

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("runtime pack is invalid"));
}

#[cfg(feature = "resident-embedding-pool-experiment")]
#[test]
fn experiment_resident_accepts_closed_pool_roles_before_loading_the_pack() {
    for role in [
        "--resident-embedding-pool-role=interactive",
        "--resident-embedding-pool-role=bulk",
    ] {
        let output = run_resident(
            &["--resident-embedding-pool-experiment", role],
            /*threads*/ 4,
        );

        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("runtime pack is invalid"));
    }
}

fn run_resident(arguments: &[&str], threads: usize) -> Output {
    let runtime_dir = TempDir::new().unwrap();
    Command::new(env!("CARGO_BIN_EXE_resume-embedding-runtime"))
        .args(arguments)
        .env("RESUME_IR_EMBEDDING_RUNTIME_DIR", runtime_dir.path())
        .env("RESUME_IR_EMBEDDING_MODEL_ID", "synthetic-model")
        .env("RESUME_IR_EMBEDDING_DIMENSION", "384")
        .env("RESUME_IR_EMBEDDING_INTRA_THREADS", threads.to_string())
        .output()
        .unwrap()
}
