#[test]
fn crate_identity_is_stable() {
    assert_eq!(fs_crawler::crate_name(), "fs-crawler");
}
