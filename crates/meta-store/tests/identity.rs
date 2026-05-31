#[test]
fn exposes_meta_store_crate_identity() {
    assert_eq!(meta_store::crate_name(), "meta-store");
}
