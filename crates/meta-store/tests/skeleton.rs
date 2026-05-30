#[test]
fn exposes_crate_identity() {
    assert_eq!(meta_store::crate_name(), "meta-store");
}
