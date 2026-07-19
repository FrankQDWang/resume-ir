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

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_millis(250);
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
fn protocol_mismatch_blocks_automatic_start_but_explicit_retry_rechecks_it() {
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
    state.retry().unwrap();
    wait_until(|| state.snapshot().unwrap().state == DaemonLifecycleKind::Ready);
    assert_eq!(fake.spawn_count.load(Ordering::Acquire), 1);
    state.shutdown();
}

#[test]
fn lifecycle_snapshot_is_bounded_and_contains_no_process_details() {
    let encoded = serde_json::to_string(&DaemonLifecycleSnapshot::initial()).unwrap();
    assert_eq!(
        encoded,
        r#"{"schema_version":"resume-ir.desktop-daemon-lifecycle.v1","state":"starting","generation":0,"restart_attempt":0,"restart_budget":5,"retry_delay_ms":null,"consecutive_heartbeat_failures":0,"blocked_reason":null,"last_exit":null}"#
    );
    assert!(!encoded.contains("pid"));
    assert!(!encoded.contains("path"));
    assert!(!encoded.contains("token"));
}
