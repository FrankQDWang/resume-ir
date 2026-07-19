#[test]
fn manifest_keeps_migration_fixture_support_non_default() {
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .unwrap();
    let features = manifest
        .split_once("[features]")
        .unwrap()
        .1
        .split_once("\n[")
        .unwrap()
        .0;

    assert!(features.lines().any(|line| line.trim() == "default = []"));
    assert!(features
        .lines()
        .any(|line| line.trim() == "migration-test-support = []"));
}
