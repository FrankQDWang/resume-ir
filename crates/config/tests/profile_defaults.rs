#![allow(missing_docs)]

use config::{Profile, ProfileDefaults};

#[test]
fn balanced_is_the_default_profile() {
    assert_eq!(Profile::default(), Profile::Balanced);
}

#[test]
fn profile_defaults_scale_resource_use() {
    let economy = ProfileDefaults::for_profile(Profile::Economy);
    let balanced = ProfileDefaults::for_profile(Profile::Balanced);
    let turbo = ProfileDefaults::for_profile(Profile::Turbo);

    assert!(economy.max_parallel_ingest < balanced.max_parallel_ingest);
    assert!(balanced.max_parallel_ingest < turbo.max_parallel_ingest);
    assert!(economy.parser_timeout_ms >= balanced.parser_timeout_ms);
    assert!(turbo.search_result_limit >= balanced.search_result_limit);
}
