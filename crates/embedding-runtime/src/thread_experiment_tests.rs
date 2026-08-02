use std::{cell::Cell, ffi::OsString};

use crate::profiling::{parse_run_mode, ProfilingMode, ResidentMode, RunMode};
use tempfile::TempDir;

use super::*;

fn args(mode: &str) -> [String; 1] {
    [mode.to_string()]
}

#[test]
fn rejects_missing_malformed_and_out_of_range_thread_counts() {
    for value in ["1", "6"] {
        assert!(
            parse_thread_experiment_mode(&args("--resident-thread-matrix"), || Some(
                OsString::from(value)
            ))
            .unwrap()
            .is_some()
        );
    }
    for value in [None, Some(""), Some("0"), Some("7"), Some("3.0")] {
        assert!(matches!(
            parse_thread_experiment_mode(&args("--resident-thread-matrix"), || {
                value.map(OsString::from)
            }),
            Err(RuntimeError::EnvironmentInvalid)
        ));
    }
}

#[test]
fn unrelated_modes_do_not_read_the_experiment_environment() {
    let read = Cell::new(false);
    let parsed = parse_thread_experiment_mode(&args("--resident"), || {
        read.set(true);
        Some(OsString::from("3"))
    })
    .unwrap();
    assert_eq!(parsed, None);
    assert!(!read.get());
}

#[test]
fn standard_modes_ignore_the_experiment_environment_in_the_full_parser() {
    let profile_read = Cell::new(false);
    let experiment_read = Cell::new(false);
    let mode = parse_run_mode(
        &args("--resident"),
        || {
            profile_read.set(true);
            None
        },
        || {
            experiment_read.set(true);
            Some(OsString::from("6"))
        },
    )
    .unwrap();
    assert_eq!(
        mode,
        RunMode::Resident(ResidentMode {
            profiling: ProfilingMode::Disabled,
            intra_threads: ResidentThreadPolicy::Production,
        })
    );
    assert!(!profile_read.get());
    assert!(!experiment_read.get());
}

#[test]
fn matrix_mode_reads_only_threads_and_profile_mode_requires_both_inputs() {
    let profile_read = Cell::new(false);
    let mode = parse_run_mode(
        &args("--resident-thread-matrix"),
        || {
            profile_read.set(true);
            None
        },
        || Some(OsString::from("4")),
    )
    .unwrap();
    assert_eq!(
        mode,
        RunMode::Resident(ResidentMode {
            profiling: ProfilingMode::Disabled,
            intra_threads: ResidentThreadPolicy::Experiment(4),
        })
    );
    assert!(!profile_read.get());

    let directory = TempDir::new().unwrap();
    let prefix = directory.path().join("thread-profile");
    let mode = parse_run_mode(
        &args("--resident-thread-profile"),
        || Some(prefix.clone().into_os_string()),
        || Some(OsString::from("2")),
    )
    .unwrap();
    assert_eq!(
        mode,
        RunMode::Resident(ResidentMode {
            profiling: ProfilingMode::OperatorTrace(prefix),
            intra_threads: ResidentThreadPolicy::Experiment(2),
        })
    );

    assert!(matches!(
        parse_run_mode(
            &args("--resident-thread-profile"),
            || None,
            || Some(OsString::from("2")),
        ),
        Err(RuntimeError::EnvironmentInvalid)
    ));
}
