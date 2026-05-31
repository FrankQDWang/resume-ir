use std::process::Command;

#[test]
fn daemon_binary_exposes_skeleton_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-daemon"))
        .arg("--identity")
        .output()
        .expect("run resume-daemon binary");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "resume-daemon\n");
    assert!(output.stderr.is_empty());
}
