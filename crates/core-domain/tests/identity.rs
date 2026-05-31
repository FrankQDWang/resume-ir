#[test]
fn exposes_core_domain_crate_identity() {
    assert_eq!(core_domain::crate_name(), "core-domain");
}
