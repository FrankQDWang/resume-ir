#[test]
fn exposes_crate_identity() {
    assert_eq!(daemon::crate_name(), "daemon");
}

#[test]
fn exposes_binary_name() {
    assert_eq!(daemon::binary_name(), "resume-daemon");
}
