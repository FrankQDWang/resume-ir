#[test]
fn foreground_lifecycle_skeleton_is_user_readable() {
    let mut stdout = Vec::new();

    daemon::run_foreground_once(&mut stdout).expect("foreground run");

    let output = String::from_utf8(stdout).expect("stdout utf8");
    assert!(output.contains("resume-daemon"));
    assert!(output.contains("foreground"));
}
