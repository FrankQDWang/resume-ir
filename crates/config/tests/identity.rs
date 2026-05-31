#[test]
fn exposes_config_crate_identity() {
    assert_eq!(config::crate_name(), "config");
}
