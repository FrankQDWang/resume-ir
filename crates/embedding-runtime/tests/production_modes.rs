use std::process::{Command, Output};

const ENVIRONMENT_INVALID: &str = "embedding runtime blocked: environment is invalid\n";

#[cfg(not(feature = "resident-role-isolation-experiment"))]
#[test]
fn production_binary_rejects_role_isolation_experiment_mode() {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-embedding-runtime"))
        .arg("--resident-role-isolation-experiment")
        .output()
        .unwrap();

    assert_blocked(output, ENVIRONMENT_INVALID);
}

#[test]
fn ordinary_resident_rejects_four_threads_even_when_experiment_is_compiled() {
    let directory = tempfile::tempdir().unwrap();
    let output = resident_command("--resident", directory.path(), /*threads*/ 4)
        .output()
        .unwrap();

    assert_blocked(output, ENVIRONMENT_INVALID);
}

#[cfg(feature = "resident-role-isolation-experiment")]
#[test]
fn experiment_mode_accepts_four_thread_configuration_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let output = resident_command(
        "--resident-role-isolation-experiment",
        directory.path(),
        /*threads*/ 4,
    )
    .output()
    .unwrap();

    assert_blocked(
        output,
        "embedding runtime blocked: runtime pack is invalid\n",
    );
}

fn resident_command(mode: &str, runtime_dir: &std::path::Path, threads: usize) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_resume-embedding-runtime"));
    command
        .arg(mode)
        .env("RESUME_IR_EMBEDDING_RUNTIME_DIR", runtime_dir)
        .env("RESUME_IR_EMBEDDING_MODEL_ID", "synthetic-model")
        .env("RESUME_IR_EMBEDDING_DIMENSION", "4")
        .env("RESUME_IR_EMBEDDING_INTRA_THREADS", threads.to_string());
    command
}

fn assert_blocked(output: Output, stderr: &str) {
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(String::from_utf8(output.stderr).unwrap(), stderr);
}
