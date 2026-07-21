use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

#[cfg(test)]
use std::path::Path;

#[path = "restart_ledger.rs"]
mod restart_ledger;

use super::policy::{RecoveryDecision, RestartPolicy, RestartPolicyConfig};
use super::receipt::LifecycleReceiptRecorder;
use crate::daemon_client::DesktopError;
pub(super) use restart_ledger::RestartLedgerReason;
use restart_ledger::{unix_time_ms, RestartWindowLedger};

const SCHEMA_VERSION: &str = "resume-ir.desktop-daemon-lifecycle.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DaemonLifecycleKind {
    Starting,
    Ready,
    Recovering,
    CircuitOpen,
    Blocked,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DaemonBlockedReason {
    ConfigurationInvalid,
    RuntimeIntegrity,
    ProtocolMismatch,
    OwnershipConflict,
    SupervisorUnavailable,
    RestartLedgerInvalid,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DaemonExitClass {
    ChildExited,
    StartupTimeout,
    HeartbeatTimeout,
    StartFailed,
    ControlPlaneFailure,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DaemonLifecycleSnapshot {
    pub(super) schema_version: &'static str,
    pub(super) state: DaemonLifecycleKind,
    pub(super) generation: u64,
    pub(super) restart_attempt: u8,
    pub(super) restart_budget: u8,
    pub(super) retry_delay_ms: Option<u64>,
    pub(super) consecutive_heartbeat_failures: u8,
    pub(super) blocked_reason: Option<DaemonBlockedReason>,
    pub(super) last_exit: Option<DaemonExitClass>,
    pub(super) restart_ledger_reason: Option<RestartLedgerReason>,
}

impl DaemonLifecycleSnapshot {
    fn initial() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            state: DaemonLifecycleKind::Starting,
            generation: 0,
            restart_attempt: 0,
            restart_budget: 5,
            retry_delay_ms: None,
            consecutive_heartbeat_failures: 0,
            blocked_reason: None,
            last_exit: None,
            restart_ledger_reason: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DaemonProbe {
    Ready,
    Unavailable,
    ProtocolMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimeFailure {
    Blocked(DaemonBlockedReason),
    Transient,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChildExitOutcome {
    Running,
    Exited,
    RestartableFatal,
    Blocked(DaemonBlockedReason),
}

pub(super) trait SupervisedChild: Send + 'static {
    fn poll_exit(&mut self) -> ChildExitOutcome;
    fn stop(self);
}

pub(super) trait DaemonRuntime: Send + 'static {
    type Child: SupervisedChild;

    fn spawn(&mut self) -> Result<Self::Child, RuntimeFailure>;
    fn probe(&mut self, timeout: Duration) -> DaemonProbe;
}

#[derive(Clone, Copy)]
pub(super) struct SupervisorTiming {
    tick: Duration,
    startup_deadline: Duration,
    heartbeat_interval: Duration,
    heartbeat_timeout: Duration,
    heartbeat_failure_limit: u8,
    policy: RestartPolicyConfig,
}

impl SupervisorTiming {
    pub(super) const fn production() -> Self {
        Self {
            tick: Duration::from_millis(100),
            startup_deadline: Duration::from_secs(10),
            heartbeat_interval: Duration::from_secs(5),
            heartbeat_timeout: Duration::from_secs(2),
            heartbeat_failure_limit: 3,
            policy: RestartPolicyConfig::production(),
        }
    }
}

enum SupervisorCommand {
    Retry(mpsc::Sender<DaemonLifecycleSnapshot>),
    Stop(mpsc::Sender<()>),
}

pub(crate) struct DaemonLifecycleState {
    commands: mpsc::SyncSender<SupervisorCommand>,
    snapshot: Arc<Mutex<DaemonLifecycleSnapshot>>,
    receipt: Arc<LifecycleReceiptRecorder>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl DaemonLifecycleState {
    pub(super) fn launch<R: DaemonRuntime>(
        runtime: R,
        receipt: LifecycleReceiptRecorder,
    ) -> Result<Self, DesktopError> {
        Self::launch_with_timing_and_receipt(runtime, SupervisorTiming::production(), receipt)
    }

    #[cfg(test)]
    fn launch_with_timing<R: DaemonRuntime>(
        runtime: R,
        timing: SupervisorTiming,
    ) -> Result<Self, DesktopError> {
        Self::launch_with_components(
            runtime,
            timing,
            LifecycleReceiptRecorder::disabled(),
            RestartWindowLedger::disabled(timing.policy),
        )
    }

    #[cfg(test)]
    fn launch_with_timing_and_data_dir<R: DaemonRuntime>(
        runtime: R,
        timing: SupervisorTiming,
        data_dir: &Path,
    ) -> Result<Self, DesktopError> {
        Self::launch_with_components(
            runtime,
            timing,
            LifecycleReceiptRecorder::initialize(data_dir),
            RestartWindowLedger::initialize(data_dir, unix_time_ms(), timing.policy),
        )
    }

    fn launch_with_timing_and_receipt<R: DaemonRuntime>(
        runtime: R,
        timing: SupervisorTiming,
        receipt: LifecycleReceiptRecorder,
    ) -> Result<Self, DesktopError> {
        let restart_ledger = receipt
            .data_dir()
            .map(|data_dir| {
                RestartWindowLedger::initialize(data_dir, unix_time_ms(), timing.policy)
            })
            .unwrap_or_else(|| RestartWindowLedger::disabled(timing.policy));
        Self::launch_with_components(runtime, timing, receipt, restart_ledger)
    }

    fn launch_with_components<R: DaemonRuntime>(
        runtime: R,
        timing: SupervisorTiming,
        receipt: LifecycleReceiptRecorder,
        restart_ledger: RestartWindowLedger,
    ) -> Result<Self, DesktopError> {
        let (commands, receiver) = mpsc::sync_channel(8);
        let snapshot = Arc::new(Mutex::new(DaemonLifecycleSnapshot::initial()));
        let actor_snapshot = Arc::clone(&snapshot);
        let receipt = Arc::new(receipt);
        let actor_receipt = Arc::clone(&receipt);
        let thread = thread::Builder::new()
            .name("resume-daemon-supervisor".to_string())
            .spawn(move || {
                SupervisorActor::new(
                    runtime,
                    timing,
                    actor_snapshot,
                    actor_receipt,
                    restart_ledger,
                )
                .run(receiver)
            })
            .map_err(|_| {
                DesktopError::new(
                    "daemon_supervisor_unavailable",
                    "本地 daemon 监督器无法启动",
                )
            })?;
        Ok(Self {
            commands,
            snapshot,
            receipt,
            thread: Mutex::new(Some(thread)),
        })
    }

    pub(crate) fn snapshot(&self) -> Result<DaemonLifecycleSnapshot, DesktopError> {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| DesktopError::internal())
    }

    pub(crate) fn retry(&self) -> Result<DaemonLifecycleSnapshot, DesktopError> {
        let (sender, receiver) = mpsc::channel();
        self.commands
            .try_send(SupervisorCommand::Retry(sender))
            .map_err(|_| DesktopError::new("daemon_supervisor_busy", "本地 daemon 监督器繁忙"))?;
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| DesktopError::internal())
    }

    pub(crate) fn diagnostics(
        &self,
        daemon_diagnostics: Option<&crate::daemon_response::DiagnosticsBody>,
    ) -> Result<Vec<u8>, DesktopError> {
        self.receipt.diagnostics(daemon_diagnostics)
    }

    pub(crate) fn shutdown(&self) {
        let handle = self.thread.lock().ok().and_then(|mut handle| handle.take());
        let Some(handle) = handle else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let _ = self.commands.send(SupervisorCommand::Stop(sender));
        let _ = receiver.recv_timeout(Duration::from_secs(3));
        let _ = handle.join();
        self.receipt.shutdown();
    }
}

impl Drop for DaemonLifecycleState {
    fn drop(&mut self) {
        let handle = self.thread.get_mut().ok().and_then(Option::take);
        let Some(handle) = handle else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let _ = self.commands.send(SupervisorCommand::Stop(sender));
        let _ = receiver.recv_timeout(Duration::from_secs(3));
        let _ = handle.join();
        self.receipt.shutdown();
    }
}

#[derive(Clone, Copy)]
enum ActorPhase {
    Waiting {
        start_at: Instant,
    },
    Starting {
        deadline: Instant,
    },
    Ready {
        stable_since: Instant,
        heartbeat_at: Instant,
        heartbeat_failures: u8,
    },
    CircuitOpen {
        retry_at: Instant,
    },
    Blocked,
}

struct SupervisorActor<R: DaemonRuntime> {
    runtime: R,
    timing: SupervisorTiming,
    started_at: Instant,
    policy: RestartPolicy,
    phase: ActorPhase,
    child: Option<R::Child>,
    snapshot: Arc<Mutex<DaemonLifecycleSnapshot>>,
    receipt: Arc<LifecycleReceiptRecorder>,
    restart_ledger: RestartWindowLedger,
}

impl<R: DaemonRuntime> SupervisorActor<R> {
    fn new(
        runtime: R,
        timing: SupervisorTiming,
        snapshot: Arc<Mutex<DaemonLifecycleSnapshot>>,
        receipt: Arc<LifecycleReceiptRecorder>,
        restart_ledger: RestartWindowLedger,
    ) -> Self {
        let started_at = Instant::now();
        let attempt_ages = restart_ledger.restart_attempt_ages(unix_time_ms());
        let mut actor = Self {
            runtime,
            timing,
            started_at,
            policy: RestartPolicy::with_restart_attempt_ages(timing.policy, attempt_ages),
            phase: ActorPhase::Waiting {
                start_at: started_at,
            },
            child: None,
            snapshot,
            receipt,
            restart_ledger,
        };
        actor.restore_persisted_phase(started_at);
        actor
    }

    fn run(mut self, receiver: mpsc::Receiver<SupervisorCommand>) {
        loop {
            match receiver.recv_timeout(self.timing.tick) {
                Ok(SupervisorCommand::Retry(reply)) => {
                    self.manual_retry();
                    let _ = reply.send(self.current_snapshot());
                }
                Ok(SupervisorCommand::Stop(reply)) => {
                    self.authorize_clean_restart_if_ready();
                    self.stop_child();
                    let _ = reply.send(());
                    return;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.stop_child();
                    return;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            self.tick();
        }
    }

    fn restore_persisted_phase(&mut self, now: Instant) {
        if self.restart_ledger.reason().is_some() {
            self.block(DaemonBlockedReason::RestartLedgerInvalid);
            return;
        }
        if let Some(remaining) = self
            .restart_ledger
            .circuit_remaining(unix_time_ms(), self.timing.policy.circuit_open)
        {
            if !remaining.is_zero() {
                self.policy.restore_circuit_open();
                self.phase = ActorPhase::CircuitOpen {
                    retry_at: now + remaining,
                };
                self.publish_state(DaemonLifecycleKind::CircuitOpen, Some(remaining), None, 0);
            } else {
                self.begin_durable_half_open(now);
            }
            return;
        }
        if let Some(remaining) = self
            .restart_ledger
            .scheduled_restart_remaining(unix_time_ms())
        {
            self.phase = ActorPhase::Waiting {
                start_at: now + remaining,
            };
            self.publish_state(DaemonLifecycleKind::Recovering, Some(remaining), None, 0);
            return;
        }
        self.phase = ActorPhase::Waiting { start_at: now };
        self.publish_state(DaemonLifecycleKind::Starting, None, None, 0);
    }

    fn tick(&mut self) {
        let now = Instant::now();
        match self.phase {
            ActorPhase::Waiting { start_at } if now >= start_at => self.attempt_start(now),
            ActorPhase::Starting { deadline } => self.observe_starting(now, deadline),
            ActorPhase::Ready {
                stable_since,
                heartbeat_at,
                heartbeat_failures,
            } => self.observe_ready(now, stable_since, heartbeat_at, heartbeat_failures),
            ActorPhase::CircuitOpen { retry_at } if now >= retry_at => {
                self.begin_durable_half_open(now);
            }
            ActorPhase::Waiting { .. } | ActorPhase::CircuitOpen { .. } | ActorPhase::Blocked => {}
        }
    }

    fn attempt_start(&mut self, now: Instant) {
        match self.runtime.probe(self.timing.heartbeat_timeout) {
            DaemonProbe::Ready => {
                self.block(DaemonBlockedReason::OwnershipConflict);
                return;
            }
            DaemonProbe::ProtocolMismatch => {
                self.block(DaemonBlockedReason::ProtocolMismatch);
                return;
            }
            DaemonProbe::Unavailable => {}
        }
        if self
            .restart_ledger
            .consume_start_authority(unix_time_ms())
            .is_err()
        {
            self.block(DaemonBlockedReason::RestartLedgerInvalid);
            return;
        }
        match self.runtime.spawn() {
            Ok(child) => {
                self.child = Some(child);
                self.phase = ActorPhase::Starting {
                    deadline: now + self.timing.startup_deadline,
                };
                let state = if self.current_snapshot().generation == 0 {
                    DaemonLifecycleKind::Starting
                } else {
                    DaemonLifecycleKind::Recovering
                };
                self.publish_state(state, None, None, 0);
            }
            Err(RuntimeFailure::Blocked(reason)) => self.block(reason),
            Err(RuntimeFailure::Transient) => self.recover(DaemonExitClass::StartFailed),
        }
    }

    fn observe_starting(&mut self, now: Instant, deadline: Instant) {
        if self.handle_child_exit() {
            return;
        }
        match self.runtime.probe(self.timing.heartbeat_timeout) {
            DaemonProbe::Ready => {
                if self
                    .restart_ledger
                    .record_probation_ready(unix_time_ms())
                    .is_err()
                {
                    self.stop_child();
                    self.block(DaemonBlockedReason::RestartLedgerInvalid);
                    return;
                }
                self.policy.on_ready(self.elapsed(now));
                self.phase = ActorPhase::Ready {
                    stable_since: now,
                    heartbeat_at: now + self.timing.heartbeat_interval,
                    heartbeat_failures: 0,
                };
                self.update_snapshot(|snapshot| {
                    snapshot.state = DaemonLifecycleKind::Ready;
                    snapshot.generation = snapshot.generation.saturating_add(1);
                    snapshot.retry_delay_ms = None;
                    snapshot.consecutive_heartbeat_failures = 0;
                    snapshot.blocked_reason = None;
                });
                self.record_snapshot();
            }
            DaemonProbe::ProtocolMismatch => {
                self.stop_child();
                self.block(DaemonBlockedReason::ProtocolMismatch);
            }
            DaemonProbe::Unavailable if now >= deadline => {
                self.stop_child();
                self.recover(DaemonExitClass::StartupTimeout);
            }
            DaemonProbe::Unavailable => {}
        }
    }

    fn observe_ready(
        &mut self,
        now: Instant,
        stable_since: Instant,
        heartbeat_at: Instant,
        heartbeat_failures: u8,
    ) {
        if self.handle_child_exit() {
            return;
        }
        if now < heartbeat_at {
            return;
        }
        match self.runtime.probe(self.timing.heartbeat_timeout) {
            DaemonProbe::Ready => {
                self.phase = ActorPhase::Ready {
                    stable_since,
                    heartbeat_at: now + self.timing.heartbeat_interval,
                    heartbeat_failures: 0,
                };
                self.publish_heartbeat_failures(0);
                self.clear_restart_budget_if_stable(now);
            }
            DaemonProbe::ProtocolMismatch => {
                self.stop_child();
                self.block(DaemonBlockedReason::ProtocolMismatch);
            }
            DaemonProbe::Unavailable => {
                let failures = heartbeat_failures.saturating_add(1);
                if failures >= self.timing.heartbeat_failure_limit {
                    self.stop_child();
                    self.recover(DaemonExitClass::HeartbeatTimeout);
                } else {
                    self.phase = ActorPhase::Ready {
                        stable_since,
                        heartbeat_at: now + self.timing.heartbeat_interval,
                        heartbeat_failures: failures,
                    };
                    self.publish_heartbeat_failures(failures);
                }
            }
        }
    }

    fn recover(&mut self, exit: DaemonExitClass) {
        self.child = None;
        self.update_snapshot(|snapshot| snapshot.last_exit = Some(exit));
        let decision_at = Instant::now();
        match self.policy.on_failure(self.elapsed(decision_at)) {
            RecoveryDecision::RetryAfter(delay) => {
                if self
                    .restart_ledger
                    .record_restart_attempt(unix_time_ms(), delay)
                    .is_err()
                {
                    self.block(DaemonBlockedReason::RestartLedgerInvalid);
                    return;
                }
                let scheduled_at = Instant::now();
                self.phase = ActorPhase::Waiting {
                    start_at: scheduled_at + delay,
                };
                self.publish_state(DaemonLifecycleKind::Recovering, Some(delay), None, 0);
            }
            RecoveryDecision::OpenCircuit(delay) => {
                if self
                    .restart_ledger
                    .record_circuit_open(unix_time_ms())
                    .is_err()
                {
                    self.block(DaemonBlockedReason::RestartLedgerInvalid);
                    return;
                }
                let scheduled_at = Instant::now();
                self.phase = ActorPhase::CircuitOpen {
                    retry_at: scheduled_at + delay,
                };
                self.publish_state(DaemonLifecycleKind::CircuitOpen, Some(delay), None, 0);
            }
        }
    }

    fn block(&mut self, reason: DaemonBlockedReason) {
        self.child = None;
        self.phase = ActorPhase::Blocked;
        self.publish_state(DaemonLifecycleKind::Blocked, None, Some(reason), 0);
    }

    fn manual_retry(&mut self) {
        match self.phase {
            ActorPhase::CircuitOpen { .. } if !self.policy.begin_manual_half_open() => return,
            ActorPhase::CircuitOpen { .. } => {
                if self
                    .restart_ledger
                    .record_circuit_open(unix_time_ms())
                    .is_err()
                {
                    self.block(DaemonBlockedReason::RestartLedgerInvalid);
                    return;
                }
            }
            ActorPhase::Blocked
            | ActorPhase::Waiting { .. }
            | ActorPhase::Starting { .. }
            | ActorPhase::Ready { .. } => {
                return;
            }
        }
        let now = Instant::now();
        self.phase = ActorPhase::Waiting { start_at: now };
        self.publish_state(
            DaemonLifecycleKind::Recovering,
            Some(Duration::ZERO),
            None,
            0,
        );
    }

    fn begin_durable_half_open(&mut self, now: Instant) {
        if self
            .restart_ledger
            .record_circuit_open(unix_time_ms())
            .is_err()
        {
            self.block(DaemonBlockedReason::RestartLedgerInvalid);
            return;
        }
        self.policy.begin_half_open();
        self.phase = ActorPhase::Waiting { start_at: now };
        self.publish_state(
            DaemonLifecycleKind::Recovering,
            Some(Duration::ZERO),
            None,
            0,
        );
    }

    fn clear_restart_budget_if_stable(&mut self, now: Instant) {
        if !self.policy.stable_reset_due(self.elapsed(now)) {
            return;
        }
        if self.restart_ledger.clear_after_stable_ready().is_ok() {
            self.policy.complete_stable_reset(self.elapsed(now));
            self.update_snapshot(|snapshot| {
                snapshot.restart_attempt = 0;
                snapshot.restart_ledger_reason = None;
            });
            self.record_snapshot();
        } else {
            let reason = self.restart_ledger.reason();
            let changed = self.current_snapshot().restart_ledger_reason != reason;
            self.update_snapshot(|snapshot| snapshot.restart_ledger_reason = reason);
            if changed {
                self.record_snapshot();
            }
        }
    }

    fn authorize_clean_restart_if_ready(&mut self) {
        if !matches!(self.phase, ActorPhase::Ready { .. })
            || !self
                .child
                .as_mut()
                .is_some_and(|child| child.poll_exit() == ChildExitOutcome::Running)
        {
            return;
        }
        if self
            .restart_ledger
            .authorize_clean_restart(unix_time_ms())
            .is_err()
        {
            let reason = self.restart_ledger.reason();
            self.update_snapshot(|snapshot| snapshot.restart_ledger_reason = reason);
            self.record_snapshot();
        }
    }

    fn handle_child_exit(&mut self) -> bool {
        let outcome = self
            .child
            .as_mut()
            .map_or(ChildExitOutcome::Exited, SupervisedChild::poll_exit);
        match outcome {
            ChildExitOutcome::Running => false,
            ChildExitOutcome::Exited => {
                self.recover(DaemonExitClass::ChildExited);
                true
            }
            ChildExitOutcome::RestartableFatal => {
                self.recover(DaemonExitClass::ControlPlaneFailure);
                true
            }
            ChildExitOutcome::Blocked(reason) => {
                self.block(reason);
                true
            }
        }
    }

    fn stop_child(&mut self) {
        if let Some(child) = self.child.take() {
            child.stop();
        }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started_at)
    }

    fn publish_state(
        &mut self,
        state: DaemonLifecycleKind,
        retry: Option<Duration>,
        blocked_reason: Option<DaemonBlockedReason>,
        heartbeat_failures: u8,
    ) {
        let attempts = self.policy.restart_attempts(self.elapsed(Instant::now()));
        let restart_ledger_reason = self.restart_ledger.reason();
        self.update_snapshot(|snapshot| {
            snapshot.state = state;
            snapshot.restart_attempt = attempts;
            snapshot.retry_delay_ms = retry.map(duration_millis);
            snapshot.consecutive_heartbeat_failures = heartbeat_failures;
            snapshot.blocked_reason = blocked_reason;
            snapshot.restart_ledger_reason = restart_ledger_reason;
        });
        self.record_snapshot();
    }

    fn publish_heartbeat_failures(&self, failures: u8) {
        self.update_snapshot(|snapshot| snapshot.consecutive_heartbeat_failures = failures);
    }

    fn current_snapshot(&self) -> DaemonLifecycleSnapshot {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_else(|_| {
                let mut snapshot = DaemonLifecycleSnapshot::initial();
                snapshot.state = DaemonLifecycleKind::Blocked;
                snapshot.blocked_reason = Some(DaemonBlockedReason::SupervisorUnavailable);
                snapshot
            })
    }

    fn update_snapshot(&self, update: impl FnOnce(&mut DaemonLifecycleSnapshot)) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            update(&mut snapshot);
        }
    }

    fn record_snapshot(&self) {
        self.receipt.record(&self.current_snapshot());
    }
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "supervisor_tests.rs"]
mod tests;
