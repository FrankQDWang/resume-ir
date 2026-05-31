use config::{Profile, ProfileDefaults};

#[test]
fn balanced_is_the_default_profile() {
    assert_eq!(Profile::default(), Profile::Balanced);

    let defaults = ProfileDefaults::default();

    assert_eq!(defaults.profile, Profile::Balanced);
}

#[test]
fn profile_defaults_are_deterministic_resource_tiers() {
    let economy = Profile::Economy.defaults();
    let balanced = Profile::Balanced.defaults();
    let turbo = Profile::Turbo.defaults();

    assert_eq!(economy.max_worker_threads, 1);
    assert_eq!(balanced.max_worker_threads, 4);
    assert_eq!(turbo.max_worker_threads, 8);

    assert!(economy.max_parallel_documents < balanced.max_parallel_documents);
    assert!(balanced.max_parallel_documents < turbo.max_parallel_documents);
    assert!(economy.index_commit_batch_size < balanced.index_commit_batch_size);
    assert!(balanced.index_commit_batch_size < turbo.index_commit_batch_size);
    assert!(economy.max_document_bytes < balanced.max_document_bytes);
    assert!(balanced.max_document_bytes < turbo.max_document_bytes);
}
