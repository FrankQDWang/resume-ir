//! Configuration profiles for the local-first resume search kernel.

use serde::{Deserialize, Serialize};

/// Runtime resource profile.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub enum Profile {
    /// Conservative resource use for battery or low-power machines.
    Economy,
    /// Balanced defaults for normal local use.
    #[default]
    Balanced,
    /// Higher local throughput for powerful machines.
    Turbo,
}

/// Concrete defaults derived from a profile.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ProfileDefaults {
    /// Maximum concurrent ingestion workers.
    pub max_parallel_ingest: usize,
    /// Parser timeout in milliseconds.
    pub parser_timeout_ms: u64,
    /// Default search result limit.
    pub search_result_limit: usize,
    /// Whether semantic embedding work is enabled by default.
    pub embeddings_enabled: bool,
}

impl ProfileDefaults {
    /// Returns deterministic defaults for a profile.
    #[must_use]
    pub fn for_profile(profile: Profile) -> Self {
        match profile {
            Profile::Economy => Self {
                max_parallel_ingest: 1,
                parser_timeout_ms: 30_000,
                search_result_limit: 20,
                embeddings_enabled: false,
            },
            Profile::Balanced => Self {
                max_parallel_ingest: 2,
                parser_timeout_ms: 20_000,
                search_result_limit: 50,
                embeddings_enabled: false,
            },
            Profile::Turbo => Self {
                max_parallel_ingest: 4,
                parser_timeout_ms: 15_000,
                search_result_limit: 100,
                embeddings_enabled: false,
            },
        }
    }
}

/// Minimal application configuration foundation.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct AppConfig {
    /// Selected runtime profile.
    pub profile: Profile,
    /// Defaults resolved from the selected profile.
    pub defaults: ProfileDefaults,
}

impl Default for AppConfig {
    fn default() -> Self {
        let profile = Profile::default();

        Self {
            profile,
            defaults: ProfileDefaults::for_profile(profile),
        }
    }
}
