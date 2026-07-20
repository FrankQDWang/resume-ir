use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::native_import::MAX_DIAGNOSTICS_EXPORT_BYTES;

#[derive(Default)]
struct FakeState {
    alive: AtomicBool,
    spawn_count: AtomicUsize,
    transient_spawn_failure: AtomicBool,
    protocol_mismatch: AtomicBool,
    unavailable_probes: AtomicUsize,
    fatal_exit: Mutex<Option<ChildExitOutcome>>,
}

struct FakeRuntime {
    state: Arc<FakeState>,
}

struct FakeChild {
    state: Arc<FakeState>,
}

impl SupervisedChild for FakeChild {
    fn poll_exit(&mut self) -> ChildExitOutcome {
        if let Ok(mut fatal_exit) = self.state.fatal_exit.lock() {
            if let Some(outcome) = fatal_exit.take() {
                self.state.alive.store(false, Ordering::Release);
                return outcome;
            }
        }
        if self.state.alive.load(Ordering::Acquire) {
            ChildExitOutcome::Running
        } else {
            ChildExitOutcome::Exited
        }
    }

    fn stop(self) {
        self.state.alive.store(false, Ordering::Release);
    }
}

impl DaemonRuntime for FakeRuntime {
    type Child = FakeChild;

    fn spawn(&mut self) -> Result<Self::Child, RuntimeFailure> {
        self.state.spawn_count.fetch_add(1, Ordering::AcqRel);
        if self.state.transient_spawn_failure.load(Ordering::Acquire) {
            return Err(RuntimeFailure::Transient);
        }
        self.state.alive.store(true, Ordering::Release);
        Ok(FakeChild {
            state: Arc::clone(&self.state),
        })
    }

    fn probe(&mut self, _timeout: Duration) -> DaemonProbe {
        if self.state.protocol_mismatch.load(Ordering::Acquire) {
            return DaemonProbe::ProtocolMismatch;
        }
        if self
            .state
            .unavailable_probes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return DaemonProbe::Unavailable;
        }
        if self.state.alive.load(Ordering::Acquire) {
            DaemonProbe::Ready
        } else {
            DaemonProbe::Unavailable
        }
    }
}

fn test_timing() -> SupervisorTiming {
    SupervisorTiming {
        tick: Duration::from_millis(1),
        startup_deadline: Duration::from_millis(20),
        heartbeat_interval: Duration::from_millis(10),
        heartbeat_timeout: Duration::from_millis(1),
        heartbeat_failure_limit: 3,
        policy: RestartPolicyConfig {
            window: Duration::from_secs(1),
            stable_reset: Duration::from_millis(100),
            circuit_open: Duration::from_millis(100),
            backoff: [
                Duration::from_millis(1),
                Duration::from_millis(2),
                Duration::from_millis(3),
                Duration::from_millis(4),
                Duration::from_millis(5),
            ],
        },
    }
}

fn durable_test_timing() -> SupervisorTiming {
    let mut timing = test_timing();
    timing.policy.window = Duration::from_secs(60);
    timing.policy.stable_reset = Duration::from_secs(5);
    timing.policy.circuit_open = Duration::from_secs(5);
    timing
}

fn wall_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
        .try_into()
        .unwrap()
}

fn write_restart_ledger(
    data_dir: &Path,
    restart_attempts_unix_ms: Vec<u64>,
    circuit_opened_at_unix_ms: Option<u64>,
) {
    fs::create_dir_all(data_dir).unwrap();
    let path = data_dir.join("desktop-daemon-restart-window.v1.json");
    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": "resume-ir.desktop-daemon-restart-window.v1",
            "restart_attempts_unix_ms": restart_attempts_unix_ms,
            "scheduled_restart_not_before_unix_ms": null,
            "circuit_opened_at_unix_ms": circuit_opened_at_unix_ms,
            "clean_shutdown_at_unix_ms": null,
        }))
        .unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn restart_ledger(data_dir: &Path) -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(data_dir.join("desktop-daemon-restart-window.v1.json")).unwrap(),
    )
    .unwrap()
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    wait_until_for(Duration::from_secs(2), &mut condition);
}

fn wait_until_for(timeout: Duration, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "condition did not become true");
        thread::yield_now();
    }
}

#[test]
fn actor_owns_restart_and_recovers_after_an_unexpected_exit() {
    let fake = Arc::new(FakeState::default());
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 1);

    fake.alive.store(false, Ordering::Release);
    wait_until(|| {
        state.snapshot().unwrap().generation >= 2
            && state.snapshot().unwrap().state == DaemonLifecycleKind::Ready
    });
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 2);
    state.shutdown();
    assert!(!fake.alive.load(Ordering::Acquire));
}

#[test]
fn two_heartbeat_failures_do_not_restart_a_healthy_child() {
    let fake = Arc::new(FakeState::default());
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    fake.unavailable_probes.store(2, Ordering::Release);
    wait_until(|| {
        state
            .snapshot()
            .is_ok_and(|snapshot| snapshot.consecutive_heartbeat_failures == 2)
    });
    wait_until(|| {
        state
            .snapshot()
            .is_ok_and(|snapshot| snapshot.consecutive_heartbeat_failures == 0)
    });
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 1);
    state.shutdown();
}

#[test]
fn third_consecutive_heartbeat_failure_restarts_the_child() {
    let fake = Arc::new(FakeState::default());
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    fake.unavailable_probes.store(3, Ordering::Release);
    wait_until(|| {
        state
            .snapshot()
            .is_ok_and(|snapshot| snapshot.generation >= 2)
    });
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 2);
    assert_eq!(
        state.snapshot().unwrap().last_exit,
        Some(DaemonExitClass::HeartbeatTimeout)
    );
    state.shutdown();
}

#[test]
fn closed_fatal_events_drive_blocked_or_restartable_supervisor_outcomes() {
    let blocked = Arc::new(FakeState::default());
    let blocked_state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&blocked),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| blocked_state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    *blocked.fatal_exit.lock().unwrap() = Some(ChildExitOutcome::Blocked(
        DaemonBlockedReason::RuntimeIntegrity,
    ));
    wait_until(|| blocked_state.snapshot().unwrap().state == DaemonLifecycleKind::Blocked);
    assert_eq!(
        blocked_state.snapshot().unwrap().blocked_reason,
        Some(DaemonBlockedReason::RuntimeIntegrity)
    );
    assert_eq!(blocked.spawn_count.load(Ordering::Acquire), 1);
    blocked_state.shutdown();

    let restartable = Arc::new(FakeState::default());
    let restartable_state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&restartable),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| restartable_state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    *restartable.fatal_exit.lock().unwrap() = Some(ChildExitOutcome::RestartableFatal);
    wait_until(|| restartable_state.snapshot().unwrap().generation >= 2);
    assert_eq!(
        restartable_state.snapshot().unwrap().last_exit,
        Some(DaemonExitClass::ControlPlaneFailure)
    );
    assert_eq!(restartable.spawn_count.load(Ordering::Acquire), 2);
    restartable_state.shutdown();
}

#[test]
fn every_blocked_reason_ignores_manual_retry_without_spawning() {
    for reason in [
        DaemonBlockedReason::ConfigurationInvalid,
        DaemonBlockedReason::RuntimeIntegrity,
        DaemonBlockedReason::ProtocolMismatch,
        DaemonBlockedReason::OwnershipConflict,
        DaemonBlockedReason::SupervisorUnavailable,
        DaemonBlockedReason::RestartLedgerInvalid,
    ] {
        let fake = Arc::new(FakeState::default());
        let state = DaemonLifecycleState::launch_with_timing(
            FakeRuntime {
                state: Arc::clone(&fake),
            },
            test_timing(),
        )
        .unwrap();
        wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
        *fake.fatal_exit.lock().unwrap() = Some(ChildExitOutcome::Blocked(reason));
        wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Blocked);
        let spawn_count = fake.spawn_count.load(Ordering::Acquire);

        let retry = state.retry().unwrap();
        assert_eq!(retry.state, DaemonLifecycleKind::Blocked, "{reason:?}");
        assert_eq!(retry.blocked_reason, Some(reason), "{reason:?}");
        thread::sleep(Duration::from_millis(10));
        assert_eq!(state.snapshot().unwrap(), retry, "{reason:?}");
        assert_eq!(fake.spawn_count.load(Ordering::Acquire), spawn_count);
        state.shutdown();
    }
}

#[test]
fn repeated_start_failures_open_the_circuit_and_manual_retry_is_half_open() {
    let fake = Arc::new(FakeState::default());
    fake.transient_spawn_failure.store(true, Ordering::Release);
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen);
    assert_eq!(state.snapshot().unwrap().restart_attempt, 5);

    fake.transient_spawn_failure.store(false, Ordering::Release);
    let retry = state.retry().unwrap();
    assert_eq!(retry.state, DaemonLifecycleKind::Recovering);
    assert_eq!(retry.restart_attempt, 5);
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    state.shutdown();
}

#[test]
fn full_restart_budget_survives_actor_reopen_without_an_extra_spawn() {
    let directory = tempfile::tempdir().unwrap();
    let first = Arc::new(FakeState::default());
    first.transient_spawn_failure.store(true, Ordering::Release);
    let first_state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&first),
        },
        durable_test_timing(),
        directory.path(),
    )
    .unwrap();
    wait_until(|| first_state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen);
    assert_eq!(first_state.snapshot().unwrap().restart_attempt, 5);
    first_state.shutdown();

    let reopened = Arc::new(FakeState::default());
    let reopened_state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&reopened),
        },
        durable_test_timing(),
        directory.path(),
    )
    .unwrap();
    wait_until(|| reopened_state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen);
    assert_eq!(reopened_state.snapshot().unwrap().restart_attempt, 5);
    assert_eq!(reopened.spawn_count.load(Ordering::Acquire), 0);
    reopened_state.shutdown();
}

#[test]
fn fifth_scheduled_restart_keeps_its_deadline_and_clean_shutdown_grants_one_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let first = Arc::new(FakeState::default());
    first.transient_spawn_failure.store(true, Ordering::Release);
    let mut timing = durable_test_timing();
    timing.policy.backoff[4] = Duration::from_secs(3);
    let first_state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&first),
        },
        timing,
        directory.path(),
    )
    .unwrap();
    wait_until(|| {
        first_state.snapshot().is_ok_and(|snapshot| {
            snapshot.state == DaemonLifecycleKind::Recovering && snapshot.restart_attempt == 5
        })
    });
    assert_eq!(first.spawn_count.load(Ordering::Acquire), 5);
    assert!(
        restart_ledger(directory.path())["scheduled_restart_not_before_unix_ms"]
            .as_u64()
            .is_some()
    );
    first_state.shutdown();

    let second = Arc::new(FakeState::default());
    let second_state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&second),
        },
        timing,
        directory.path(),
    )
    .unwrap();
    wait_until(|| {
        second_state
            .snapshot()
            .is_ok_and(|snapshot| snapshot.state == DaemonLifecycleKind::Recovering)
    });
    thread::sleep(Duration::from_millis(25));
    assert_eq!(second.spawn_count.load(Ordering::Acquire), 0);
    wait_until_for(Duration::from_secs(5), || {
        second_state
            .snapshot()
            .is_ok_and(|snapshot| snapshot.state == DaemonLifecycleKind::Ready)
    });
    assert_eq!(second.spawn_count.load(Ordering::Acquire), 1);
    assert!(restart_ledger(directory.path())["scheduled_restart_not_before_unix_ms"].is_null());
    assert!(
        restart_ledger(directory.path())["circuit_opened_at_unix_ms"]
            .as_u64()
            .is_some()
    );
    second_state.shutdown();
    assert!(restart_ledger(directory.path())["circuit_opened_at_unix_ms"].is_null());
    assert!(
        restart_ledger(directory.path())["clean_shutdown_at_unix_ms"]
            .as_u64()
            .is_some()
    );

    let third = Arc::new(FakeState::default());
    let third_state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&third),
        },
        timing,
        directory.path(),
    )
    .unwrap();
    wait_until(|| {
        third_state
            .snapshot()
            .is_ok_and(|snapshot| snapshot.state == DaemonLifecycleKind::Ready)
    });
    assert_eq!(third.spawn_count.load(Ordering::Acquire), 1);
    assert!(restart_ledger(directory.path())["clean_shutdown_at_unix_ms"].is_null());
    assert!(
        restart_ledger(directory.path())["circuit_opened_at_unix_ms"]
            .as_u64()
            .is_some()
    );
    third_state.shutdown();
}

#[test]
fn expired_reopened_circuit_consumes_exactly_one_durable_half_open_attempt() {
    let directory = tempfile::tempdir().unwrap();
    let now = wall_now_ms();
    write_restart_ledger(
        directory.path(),
        (0..5).map(|offset| now - 6_000 + offset).collect(),
        Some(now - 5_001),
    );
    let fake = Arc::new(FakeState::default());
    fake.transient_spawn_failure.store(true, Ordering::Release);
    let state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        durable_test_timing(),
        directory.path(),
    )
    .unwrap();

    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 1);
    assert_eq!(state.snapshot().unwrap().restart_attempt, 5);
    let reopened_circuit = restart_ledger(directory.path())["circuit_opened_at_unix_ms"]
        .as_u64()
        .unwrap();
    assert!(reopened_circuit >= now);
    thread::sleep(Duration::from_millis(10));
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 1);
    state.shutdown();
}

#[test]
fn corrupt_restart_ledger_blocks_daemon_but_keeps_bounded_diagnostics_available() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join("desktop-daemon-restart-window.v1.json");
    fs::write(&path, b"not-json").unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    let fake = Arc::new(FakeState::default());
    let state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        durable_test_timing(),
        directory.path(),
    )
    .unwrap();

    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Blocked);
    let snapshot = state.snapshot().unwrap();
    assert_eq!(
        snapshot.blocked_reason,
        Some(DaemonBlockedReason::RestartLedgerInvalid)
    );
    assert_eq!(
        snapshot.restart_ledger_reason,
        Some(RestartLedgerReason::InvalidFormat)
    );
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 0);
    assert_eq!(state.retry().unwrap().state, DaemonLifecycleKind::Blocked);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 0);
    wait_until(|| {
        state.diagnostics(None).is_ok_and(|body| {
            body.windows(b"restart_ledger_invalid".len())
                .any(|window| window == b"restart_ledger_invalid")
                && body
                    .windows(b"invalid_format".len())
                    .any(|window| window == b"invalid_format")
        })
    });
    assert!(state.diagnostics(None).unwrap().len() < MAX_DIAGNOSTICS_EXPORT_BYTES);
    state.shutdown();
}

#[test]
fn manual_half_open_does_not_clear_durable_restart_history() {
    let directory = tempfile::tempdir().unwrap();
    let fake = Arc::new(FakeState::default());
    fake.transient_spawn_failure.store(true, Ordering::Release);
    let state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        durable_test_timing(),
        directory.path(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen);
    assert_eq!(
        restart_ledger(directory.path())["restart_attempts_unix_ms"]
            .as_array()
            .unwrap()
            .len(),
        5
    );

    fake.transient_spawn_failure.store(false, Ordering::Release);
    assert_eq!(
        state.retry().unwrap().state,
        DaemonLifecycleKind::Recovering
    );
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    assert_eq!(
        restart_ledger(directory.path())["restart_attempts_unix_ms"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    state.shutdown();
}

#[test]
fn stable_ready_clears_durable_history_only_after_the_threshold() {
    let directory = tempfile::tempdir().unwrap();
    let now = wall_now_ms();
    write_restart_ledger(directory.path(), vec![now], None);
    let fake = Arc::new(FakeState::default());
    let mut timing = durable_test_timing();
    timing.policy.stable_reset = Duration::from_millis(60);
    let state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        timing,
        directory.path(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    assert_eq!(
        restart_ledger(directory.path())["restart_attempts_unix_ms"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    wait_until(|| {
        restart_ledger(directory.path())["restart_attempts_unix_ms"]
            .as_array()
            .is_some_and(Vec::is_empty)
            && state
                .snapshot()
                .is_ok_and(|snapshot| snapshot.restart_attempt == 0)
    });
    state.shutdown();
}

#[test]
fn failed_manual_half_open_cannot_spawn_again_before_cooldown() {
    let fake = Arc::new(FakeState::default());
    fake.transient_spawn_failure.store(true, Ordering::Release);
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen);

    let attempts_before_manual_retry = fake.spawn_count.load(Ordering::Acquire);
    assert_eq!(
        state.retry().unwrap().state,
        DaemonLifecycleKind::Recovering
    );
    wait_until(|| {
        state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen
            && fake.spawn_count.load(Ordering::Acquire) > attempts_before_manual_retry
    });
    let attempts_after_failed_half_open = fake.spawn_count.load(Ordering::Acquire);

    assert_eq!(
        state.retry().unwrap().state,
        DaemonLifecycleKind::CircuitOpen
    );
    thread::sleep(Duration::from_millis(10));
    assert_eq!(
        fake.spawn_count.load(Ordering::Acquire),
        attempts_after_failed_half_open
    );
    state.shutdown();
}

#[test]
fn an_open_circuit_makes_one_automatic_half_open_attempt_after_the_cooldown() {
    let fake = Arc::new(FakeState::default());
    fake.transient_spawn_failure.store(true, Ordering::Release);
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen);

    fake.transient_spawn_failure.store(false, Ordering::Release);
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    assert_eq!(state.snapshot().unwrap().restart_attempt, 5);
    state.shutdown();
}

#[test]
fn a_reachable_process_not_owned_by_the_actor_is_never_adopted() {
    let fake = Arc::new(FakeState::default());
    fake.alive.store(true, Ordering::Release);
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Blocked);
    assert_eq!(
        state.snapshot().unwrap().blocked_reason,
        Some(DaemonBlockedReason::OwnershipConflict)
    );
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 0);
    state.shutdown();
}

#[test]
fn protocol_mismatch_remains_blocked_after_explicit_retry() {
    let fake = Arc::new(FakeState::default());
    fake.protocol_mismatch.store(true, Ordering::Release);
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Blocked);
    assert_eq!(
        state.snapshot().unwrap().blocked_reason,
        Some(DaemonBlockedReason::ProtocolMismatch)
    );
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 0);

    fake.protocol_mismatch.store(false, Ordering::Release);
    let retry = state.retry().unwrap();
    assert_eq!(retry.state, DaemonLifecycleKind::Blocked);
    assert_eq!(
        retry.blocked_reason,
        Some(DaemonBlockedReason::ProtocolMismatch)
    );
    thread::sleep(Duration::from_millis(10));
    assert_eq!(state.snapshot().unwrap(), retry);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 0);
    state.shutdown();
}

#[test]
fn lifecycle_snapshot_is_bounded_and_contains_no_process_details() {
    let encoded = serde_json::to_string(&DaemonLifecycleSnapshot::initial()).unwrap();
    assert_eq!(
        encoded,
        r#"{"schema_version":"resume-ir.desktop-daemon-lifecycle.v1","state":"starting","generation":0,"restart_attempt":0,"restart_budget":5,"retry_delay_ms":null,"consecutive_heartbeat_failures":0,"blocked_reason":null,"last_exit":null,"restart_ledger_reason":null}"#
    );
    assert!(!encoded.contains("pid"));
    assert!(!encoded.contains("path"));
    assert!(!encoded.contains("token"));
}
