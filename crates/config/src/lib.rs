#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    Economy,
    Balanced,
    Turbo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeProfile {
    pub profile: Profile,
    pub max_cpu_threads: u16,
    pub enable_ocr: bool,
    pub enable_vectors: bool,
    pub query_timeout_ms: u64,
    pub background_priority: BackgroundPriority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackgroundPriority {
    Low,
    Normal,
    High,
}

impl RuntimeProfile {
    #[must_use]
    pub fn for_profile(profile: Profile) -> Self {
        match profile {
            Profile::Economy => Self {
                profile,
                max_cpu_threads: 2,
                enable_ocr: false,
                enable_vectors: false,
                query_timeout_ms: 200,
                background_priority: BackgroundPriority::Low,
            },
            Profile::Balanced => Self {
                profile,
                max_cpu_threads: 4,
                enable_ocr: true,
                enable_vectors: true,
                query_timeout_ms: 200,
                background_priority: BackgroundPriority::Normal,
            },
            Profile::Turbo => Self {
                profile,
                max_cpu_threads: 8,
                enable_ocr: true,
                enable_vectors: true,
                query_timeout_ms: 200,
                background_priority: BackgroundPriority::High,
            },
        }
    }
}

impl Default for RuntimeProfile {
    fn default() -> Self {
        Self::for_profile(Profile::Balanced)
    }
}

#[must_use]
pub fn crate_name() -> &'static str {
    "config"
}
