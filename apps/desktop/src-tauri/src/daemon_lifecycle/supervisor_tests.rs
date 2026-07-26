use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;

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
    launch_id: String,
}

impl SupervisedChild for FakeChild {
    fn launch_id(&self) -> &str {
        &self.launch_id
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
        let attempt = self.state.spawn_count.fetch_add(1, Ordering::AcqRel) + 1;
        if self.state.transient_spawn_failure.load(Ordering::Acquire) {
            return Err(RuntimeFailure::Transient);
        }
        self.state.alive.store(true, Ordering::Release);
        Ok(FakeChild {
            state: Arc::clone(&self.state),
            launch_id: format!("{attempt:064x}"),
        })
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

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !condition() {
        assert!(Instant::now() < deadline, "condition did not become true");
        thread::yield_now();
    }
}

#[test]
fn actor_owns_restart_and_binds_ready_identity_to_each_launch() {
    let fake = Arc::new(FakeState::default());
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Running);
    let first = state.ready_identity().unwrap();
    assert_eq!(first.supervisor_generation, 1);
    assert_eq!(first.launch_id, format!("{:064x}", 1));

    fake.alive.store(false, Ordering::Release);
    wait_until(|| state.snapshot().unwrap().generation == 2);
    let second = state.ready_identity().unwrap();
    assert_eq!(second.supervisor_generation, 2);
    assert_ne!(second.launch_id, first.launch_id);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 2);
    state.shutdown();
}

#[test]
fn startup_spawns_without_preprobing_stale_reachable_state() {
    let fake = Arc::new(FakeState::default());
    fake.alive.store(true, Ordering::Release);
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Running);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 1);
    state.shutdown();
}

#[test]
fn exact_launch_protocol_mismatch_blocks_immediately() {
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
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 1);
    assert_eq!(
        state.snapshot().unwrap().transition_reason,
        DaemonTransitionReason::ProtocolMismatch
    );
    state.shutdown();
}

#[test]
fn heartbeat_requires_three_consecutive_failures_before_restart() {
    let fake = Arc::new(FakeState::default());
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Running);
    fake.unavailable_probes.store(2, Ordering::Release);
    wait_until(|| state.snapshot().unwrap().heartbeat_failures == 2);
    wait_until(|| state.snapshot().unwrap().heartbeat_failures == 0);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 1);

    fake.unavailable_probes.store(3, Ordering::Release);
    wait_until(|| state.snapshot().unwrap().generation == 2);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 2);
    assert_eq!(
        state.snapshot().unwrap().last_exit,
        Some(DaemonExitClass::HeartbeatTimeout)
    );
    state.shutdown();
}

#[test]
fn blocked_state_allows_exactly_one_explicit_new_launch() {
    let fake = Arc::new(FakeState::default());
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Running);
    *fake.fatal_exit.lock().unwrap() = Some(ChildExitOutcome::Blocked(
        DaemonBlockedReason::RuntimeIntegrity,
    ));
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Blocked);
    assert_eq!(
        state.snapshot().unwrap().transition_reason,
        DaemonTransitionReason::RuntimeIntegrity
    );

    let retry = state.retry().unwrap();
    assert_eq!(retry.state, DaemonLifecycleKind::Starting);
    assert_eq!(retry.transition_reason, DaemonTransitionReason::ManualRetry);
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Running);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 2);
    assert_eq!(state.retry().unwrap_err().code(), "retry_not_allowed");
    state.shutdown();
}

#[test]
fn circuit_open_enforces_expiry_allows_once_and_reopens_after_failure() {
    let fake = Arc::new(FakeState::default());
    fake.transient_spawn_failure.store(true, Ordering::Release);
    let mut timing = test_timing();
    timing.policy.circuit_open = Duration::from_millis(100);
    let state = DaemonLifecycleState::launch_with_timing(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        timing,
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen);
    let opened = state.snapshot().unwrap();
    assert_eq!(opened.automatic_restart_attempt, 5);
    assert!(opened.retry_after_ms.is_some_and(|remaining| remaining > 0));
    assert_eq!(state.retry().unwrap_err().code(), "retry_not_allowed");

    wait_until(|| state.snapshot().unwrap().retry_after_ms == Some(0));
    assert_eq!(state.retry().unwrap().state, DaemonLifecycleKind::Starting);
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::CircuitOpen);
    assert!(state
        .snapshot()
        .unwrap()
        .retry_after_ms
        .is_some_and(|remaining| remaining > 0));
    assert_eq!(state.retry().unwrap_err().code(), "retry_not_allowed");

    wait_until(|| state.snapshot().unwrap().retry_after_ms == Some(0));
    fake.transient_spawn_failure.store(false, Ordering::Release);
    assert_eq!(state.retry().unwrap().state, DaemonLifecycleKind::Starting);
    assert_eq!(state.retry().unwrap_err().code(), "retry_not_allowed");
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Running);
    state.shutdown();
}

#[test]
fn app_reopen_has_a_fresh_budget_and_ignores_legacy_restart_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = directory.path().join(LEGACY_RESTART_LEDGER);
    fs::write(&legacy, b"not-json").unwrap();

    let fake = Arc::new(FakeState::default());
    let state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
        directory.path(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Running);
    assert_eq!(state.snapshot().unwrap().automatic_restart_attempt, 0);
    assert!(!legacy.exists());
    state.shutdown();

    let reopened = Arc::new(FakeState::default());
    let reopened_state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&reopened),
        },
        test_timing(),
        directory.path(),
    )
    .unwrap();
    wait_until(|| reopened_state.snapshot().unwrap().state == DaemonLifecycleKind::Running);
    assert_eq!(
        reopened_state.snapshot().unwrap().automatic_restart_attempt,
        0
    );
    assert_eq!(reopened.spawn_count.load(Ordering::Acquire), 1);
    reopened_state.shutdown();
}

#[cfg(unix)]
#[test]
fn unsafe_legacy_restart_ledger_is_never_followed_or_removed() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("private-target");
    fs::write(&target, b"private").unwrap();
    let legacy = directory.path().join(LEGACY_RESTART_LEDGER);
    symlink(&target, &legacy).unwrap();
    let fake = Arc::new(FakeState::default());
    let state = DaemonLifecycleState::launch_with_timing_and_data_dir(
        FakeRuntime {
            state: Arc::clone(&fake),
        },
        test_timing(),
        directory.path(),
    )
    .unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Running);
    assert_eq!(fs::read(&target).unwrap(), b"private");
    assert!(fs::symlink_metadata(&legacy)
        .unwrap()
        .file_type()
        .is_symlink());
    state.shutdown();
}

#[test]
fn lifecycle_snapshot_is_exact_bounded_and_contains_no_process_details() {
    let encoded = serde_json::to_string(&DaemonLifecycleSnapshot::initial()).unwrap();
    assert_eq!(
        encoded,
        r#"{"schema_version":"resume-ir.desktop-daemon-lifecycle.v2","state":"starting","transition_reason":"initial_start","generation":0,"automatic_restart_attempt":0,"automatic_restart_limit":5,"retry_after_ms":null,"heartbeat_failures":0,"last_exit":null}"#
    );
    for forbidden in ["pid", "path", "token", "launch_id", "restart_ledger"] {
        assert!(!encoded.contains(forbidden));
    }
}
