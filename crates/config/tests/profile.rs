use config::{Profile, RuntimeProfile};

#[test]
fn balanced_is_the_default_profile() {
    let settings = RuntimeProfile::default();

    assert_eq!(settings.profile, Profile::Balanced);
    assert!(settings.enable_ocr);
    assert!(settings.enable_vectors);
    assert_eq!(settings.query_timeout_ms, 200);
}

#[test]
fn economy_defaults_reduce_optional_work() {
    let settings = RuntimeProfile::for_profile(Profile::Economy);

    assert_eq!(settings.profile, Profile::Economy);
    assert!(!settings.enable_ocr);
    assert!(!settings.enable_vectors);
    assert_eq!(settings.max_cpu_threads, 2);
}

#[test]
fn turbo_defaults_allow_more_background_work() {
    let settings = RuntimeProfile::for_profile(Profile::Turbo);

    assert_eq!(settings.profile, Profile::Turbo);
    assert!(settings.enable_ocr);
    assert!(settings.enable_vectors);
    assert!(settings.max_cpu_threads >= 8);
}
