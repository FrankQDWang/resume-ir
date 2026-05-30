#[test]
fn exposes_crate_identity() {
    assert_eq!(resume_cli::crate_name(), "resume-cli");
}

#[test]
fn exposes_binary_name() {
    assert_eq!(resume_cli::binary_name(), "resume-cli");
}
