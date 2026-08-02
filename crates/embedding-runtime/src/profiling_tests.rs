use std::{cell::Cell, ffi::OsString};

use tempfile::TempDir;

use super::*;

fn profile_args() -> [String; 1] {
    ["--resident-profile".to_string()]
}

fn assert_invalid(value: Option<OsString>) {
    assert!(matches!(
        parse_run_mode(&profile_args(), || value),
        Err(RuntimeError::EnvironmentInvalid)
    ));
}

#[test]
fn ordinary_modes_do_not_read_profile_output() {
    let read = Cell::new(false);
    let parse = |args: &[String]| {
        parse_run_mode(args, || {
            read.set(true);
            None
        })
        .unwrap()
    };
    assert_eq!(parse(&[]), RunMode::OneShot);
    assert_eq!(
        parse(&["--resident".into()]),
        RunMode::Resident(ResidentMode {
            profiling: ProfilingMode::Disabled,
        })
    );
    assert!(!read.get());
}

#[test]
fn profiling_mode_requires_a_new_absolute_prefix_in_a_real_directory() {
    let directory = TempDir::new().unwrap();
    let prefix = directory.path().join("operator-profile");
    let mode = parse_run_mode(&profile_args(), || Some(prefix.clone().into_os_string())).unwrap();
    assert_eq!(
        mode,
        RunMode::Resident(ResidentMode {
            profiling: ProfilingMode::OperatorTrace(prefix.clone()),
        })
    );

    std::fs::write(&prefix, b"occupied").unwrap();
    assert_invalid(Some(prefix.into_os_string()));
}

#[test]
fn profiling_mode_rejects_missing_relative_and_oversized_prefixes() {
    assert_invalid(None);
    assert_invalid(Some(OsString::from("relative-profile")));
    assert_invalid(Some(OsString::from(format!(
        "/{}",
        "x".repeat(MAX_RUNTIME_PATH_BYTES + 1)
    ))));
    let missing_parent = std::env::temp_dir()
        .join("resume-ir-missing-profile-parent")
        .join("operator-profile");
    assert_invalid(Some(missing_parent.into_os_string()));
}

#[test]
fn unexpected_argument_shapes_remain_invalid() {
    for args in [
        vec!["--unknown".to_string()],
        vec!["--resident".to_string(), "extra".to_string()],
        vec!["--resident-profile".to_string(), "extra".to_string()],
        vec!["--resident-artifact-matrix".to_string()],
        vec!["--resident-artifact-profile".to_string()],
    ] {
        assert!(matches!(
            parse_run_mode(&args, || Some(OsString::from("/unused"))),
            Err(RuntimeError::EnvironmentInvalid)
        ));
    }
}

#[cfg(unix)]
#[test]
fn profiling_mode_rejects_a_symlinked_parent() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let real = root.path().join("real");
    let linked = root.path().join("linked");
    std::fs::create_dir(&real).unwrap();
    symlink(&real, &linked).unwrap();
    assert_invalid(Some(linked.join("operator-profile").into_os_string()));
}
