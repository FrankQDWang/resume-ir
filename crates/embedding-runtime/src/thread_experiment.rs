#[cfg(feature = "thread-experiment")]
use std::ffi::OsString;

#[cfg(feature = "thread-experiment")]
use super::RuntimeError;

#[cfg(feature = "thread-experiment")]
pub(super) const THREAD_EXPERIMENT_INTRA_THREADS_ENV: &str =
    "RESUME_IR_EMBEDDING_THREAD_EXPERIMENT_INTRA_THREADS";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResidentThreadPolicy {
    Production,
    #[cfg(feature = "thread-experiment")]
    Experiment(usize),
}

#[cfg(feature = "thread-experiment")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ThreadExperimentProfiling {
    Disabled,
    Enabled,
}

#[cfg(feature = "thread-experiment")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ThreadExperimentMode {
    pub(super) intra_threads: usize,
    pub(super) profiling: ThreadExperimentProfiling,
}

#[cfg(feature = "thread-experiment")]
pub(super) fn parse_thread_experiment_mode(
    args: &[String],
    intra_threads: impl FnOnce() -> Option<OsString>,
) -> Result<Option<ThreadExperimentMode>, RuntimeError> {
    let profiling = match args {
        [mode] if mode == "--resident-thread-matrix" => ThreadExperimentProfiling::Disabled,
        [mode] if mode == "--resident-thread-profile" => ThreadExperimentProfiling::Enabled,
        _ => return Ok(None),
    };
    let intra_threads = intra_threads()
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=6).contains(value))
        .ok_or(RuntimeError::EnvironmentInvalid)?;
    Ok(Some(ThreadExperimentMode {
        intra_threads,
        profiling,
    }))
}

#[cfg(all(test, feature = "thread-experiment"))]
#[path = "thread_experiment_tests.rs"]
mod tests;
