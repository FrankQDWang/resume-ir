#[test]
fn exposes_crate_identity() {
    assert_eq!(config::crate_name(), "config");
}
