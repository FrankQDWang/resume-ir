use std::process::Command;

#[test]
fn cli_binary_exposes_skeleton_identity() {
    let output = Command::new(env!("CARGO_BIN_EXE_resume-cli"))
        .arg("--identity")
        .output()
        .expect("run resume-cli binary");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "resume-cli\n");
    assert!(output.stderr.is_empty());
}
