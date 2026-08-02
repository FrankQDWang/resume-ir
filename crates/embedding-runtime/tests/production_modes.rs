#[cfg(not(feature = "thread-experiment"))]
#[test]
fn production_binary_rejects_thread_experiment_modes() {
    for mode in ["--resident-thread-matrix", "--resident-thread-profile"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_resume-embedding-runtime"))
            .arg(mode)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "embedding runtime blocked: environment is invalid\n"
        );
    }
}
