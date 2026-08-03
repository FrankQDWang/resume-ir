use std::{ffi::OsString, fs, path::PathBuf};

use ort::session::{builder::SessionBuilder, Session};

use super::{RuntimeError, MAX_RUNTIME_PATH_BYTES};

pub(super) const PROFILE_OUTPUT_PREFIX_ENV: &str = "RESUME_IR_EMBEDDING_PROFILE_OUTPUT_PREFIX";

#[derive(Debug, Eq, PartialEq)]
pub(super) enum RunMode {
    OneShot,
    Resident(ResidentMode),
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ResidentMode {
    pub(super) profiling: ProfilingMode,
    pub(super) thread_policy: ResidentThreadPolicy,
    pub(super) memory_pattern: MemoryPatternPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResidentThreadPolicy {
    Production,
    #[cfg(feature = "resident-embedding-pool-experiment")]
    PoolExperiment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MemoryPatternPolicy {
    Disabled,
    #[cfg(feature = "resident-embedding-pool-experiment")]
    Enabled,
}

impl MemoryPatternPolicy {
    pub(super) fn enabled(self) -> bool {
        match self {
            Self::Disabled => false,
            #[cfg(feature = "resident-embedding-pool-experiment")]
            Self::Enabled => true,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) enum ProfilingMode {
    Disabled,
    OperatorTrace(PathBuf),
}

pub(super) fn parse_run_mode(
    args: &[String],
    profile_output_prefix: impl FnOnce() -> Option<OsString>,
) -> Result<RunMode, RuntimeError> {
    match args {
        [] => Ok(RunMode::OneShot),
        [mode] if mode == "--resident" => Ok(RunMode::Resident(ResidentMode {
            profiling: ProfilingMode::Disabled,
            thread_policy: ResidentThreadPolicy::Production,
            memory_pattern: MemoryPatternPolicy::Disabled,
        })),
        [mode] if mode == "--resident-profile" => {
            let output_prefix = validate_output_prefix(
                profile_output_prefix().ok_or(RuntimeError::EnvironmentInvalid)?,
            )?;
            Ok(RunMode::Resident(ResidentMode {
                profiling: ProfilingMode::OperatorTrace(output_prefix),
                thread_policy: ResidentThreadPolicy::Production,
                memory_pattern: MemoryPatternPolicy::Disabled,
            }))
        }
        #[cfg(feature = "resident-embedding-pool-experiment")]
        [mode] if mode == "--resident-embedding-pool-experiment" => {
            Ok(RunMode::Resident(ResidentMode {
                profiling: ProfilingMode::Disabled,
                thread_policy: ResidentThreadPolicy::PoolExperiment,
                memory_pattern: MemoryPatternPolicy::Disabled,
            }))
        }
        #[cfg(feature = "resident-embedding-pool-experiment")]
        [mode, role]
            if mode == "--resident-embedding-pool-experiment"
                && role == "--resident-embedding-pool-role=interactive" =>
        {
            Ok(RunMode::Resident(ResidentMode {
                profiling: ProfilingMode::Disabled,
                thread_policy: ResidentThreadPolicy::PoolExperiment,
                memory_pattern: MemoryPatternPolicy::Disabled,
            }))
        }
        #[cfg(feature = "resident-embedding-pool-experiment")]
        [mode, role]
            if mode == "--resident-embedding-pool-experiment"
                && role == "--resident-embedding-pool-role=bulk" =>
        {
            Ok(RunMode::Resident(ResidentMode {
                profiling: ProfilingMode::Disabled,
                thread_policy: ResidentThreadPolicy::PoolExperiment,
                memory_pattern: MemoryPatternPolicy::Enabled,
            }))
        }
        _ => Err(RuntimeError::EnvironmentInvalid),
    }
}

impl ProfilingMode {
    pub(super) fn configure_builder(
        &self,
        builder: SessionBuilder,
    ) -> Result<SessionBuilder, RuntimeError> {
        match self {
            Self::Disabled => Ok(builder),
            Self::OperatorTrace(output_prefix) => builder
                .with_profiling(output_prefix)
                .map_err(|_| RuntimeError::ModelUnavailable),
        }
    }

    pub(super) fn finish(&self, session: &mut Session) -> Result<(), RuntimeError> {
        match self {
            Self::Disabled => Ok(()),
            Self::OperatorTrace(_) => session
                .end_profiling()
                .map(|_| ())
                .map_err(|_| RuntimeError::RuntimeUnavailable),
        }
    }
}

fn validate_output_prefix(value: OsString) -> Result<PathBuf, RuntimeError> {
    let output_prefix = PathBuf::from(value);
    let text = output_prefix
        .to_str()
        .ok_or(RuntimeError::EnvironmentInvalid)?;
    if !output_prefix.is_absolute()
        || text.len() > MAX_RUNTIME_PATH_BYTES
        || output_prefix.file_name().is_none()
        || output_prefix.exists()
    {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    let parent = output_prefix
        .parent()
        .ok_or(RuntimeError::EnvironmentInvalid)?;
    let metadata = fs::symlink_metadata(parent).map_err(|_| RuntimeError::EnvironmentInvalid)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(RuntimeError::EnvironmentInvalid);
    }
    Ok(output_prefix)
}

#[cfg(test)]
#[path = "profiling_tests.rs"]
mod tests;
