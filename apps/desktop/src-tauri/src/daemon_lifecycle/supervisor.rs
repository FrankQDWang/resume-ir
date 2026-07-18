use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::policy::{RecoveryDecision, RestartPolicy, RestartPolicyConfig};
use super::receipt::LifecycleReceiptRecorder;
use crate::daemon_client::DesktopError;

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
        Self::launch_with_timing_and_receipt(runtime, timing, LifecycleReceiptRecorder::disabled())
    }

    fn launch_with_timing_and_receipt<R: DaemonRuntime>(
        runtime: R,
        timing: SupervisorTiming,
        receipt: LifecycleReceiptRecorder,
    ) -> Result<Self, DesktopError> {
        let (commands, receiver) = mpsc::sync_channel(8);
        let snapshot = Arc::new(Mutex::new(DaemonLifecycleSnapshot::initial()));
        let actor_snapshot = Arc::clone(&snapshot);
        let receipt = Arc::new(receipt);
        let actor_receipt = Arc::clone(&receipt);
        let thread = thread::Builder::new()
            .name("resume-daemon-supervisor".to_string())
            .spawn(move || {
                SupervisorActor::new(runtime, timing, actor_snapshot, actor_receipt).run(receiver)
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
}

impl<R: DaemonRuntime> SupervisorActor<R> {
    fn new(
        runtime: R,
        timing: SupervisorTiming,
        snapshot: Arc<Mutex<DaemonLifecycleSnapshot>>,
        receipt: Arc<LifecycleReceiptRecorder>,
    ) -> Self {
        let started_at = Instant::now();
        Self {
            runtime,
            timing,
            started_at,
            policy: RestartPolicy::new(timing.policy),
            phase: ActorPhase::Waiting {
                start_at: started_at,
            },
            child: None,
            snapshot,
            receipt,
        }
    }

    fn run(mut self, receiver: mpsc::Receiver<SupervisorCommand>) {
        loop {
            match receiver.recv_timeout(self.timing.tick) {
                Ok(SupervisorCommand::Retry(reply)) => {
                    self.manual_retry();
                    let _ = reply.send(self.current_snapshot());
                }
                Ok(SupervisorCommand::Stop(reply)) => {
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
                self.policy.begin_half_open();
                self.phase = ActorPhase::Waiting { start_at: now };
                self.publish_state(
                    DaemonLifecycleKind::Recovering,
                    Some(Duration::ZERO),
                    None,
                    0,
                );
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
            Err(RuntimeFailure::Transient) => self.recover(now, DaemonExitClass::StartFailed),
        }
    }

    fn observe_starting(&mut self, now: Instant, deadline: Instant) {
        if self.handle_child_exit(now) {
            return;
        }
        match self.runtime.probe(self.timing.heartbeat_timeout) {
            DaemonProbe::Ready => {
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
                self.recover(now, DaemonExitClass::StartupTimeout);
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
        if self.handle_child_exit(now) {
            return;
        }
        if self.policy.observe_ready(self.elapsed(now)) {
            self.update_snapshot(|snapshot| snapshot.restart_attempt = 0);
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
            }
            DaemonProbe::ProtocolMismatch => {
                self.stop_child();
                self.block(DaemonBlockedReason::ProtocolMismatch);
            }
            DaemonProbe::Unavailable => {
                let failures = heartbeat_failures.saturating_add(1);
                if failures >= self.timing.heartbeat_failure_limit {
                    self.stop_child();
                    self.recover(now, DaemonExitClass::HeartbeatTimeout);
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

    fn recover(&mut self, now: Instant, exit: DaemonExitClass) {
        self.child = None;
        self.update_snapshot(|snapshot| snapshot.last_exit = Some(exit));
        match self.policy.on_failure(self.elapsed(now)) {
            RecoveryDecision::RetryAfter(delay) => {
                self.phase = ActorPhase::Waiting {
                    start_at: now + delay,
                };
                self.publish_state(DaemonLifecycleKind::Recovering, Some(delay), None, 0);
            }
            RecoveryDecision::OpenCircuit(delay) => {
                self.phase = ActorPhase::CircuitOpen {
                    retry_at: now + delay,
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
        if !matches!(
            self.phase,
            ActorPhase::CircuitOpen { .. } | ActorPhase::Blocked
        ) {
            return;
        }
        self.policy.begin_half_open();
        let now = Instant::now();
        self.phase = ActorPhase::Waiting { start_at: now };
        self.publish_state(
            DaemonLifecycleKind::Recovering,
            Some(Duration::ZERO),
            None,
            0,
        );
    }

    fn handle_child_exit(&mut self, now: Instant) -> bool {
        let outcome = self
            .child
            .as_mut()
            .map_or(ChildExitOutcome::Exited, SupervisedChild::poll_exit);
        match outcome {
            ChildExitOutcome::Running => false,
            ChildExitOutcome::Exited => {
                self.recover(now, DaemonExitClass::ChildExited);
                true
            }
            ChildExitOutcome::RestartableFatal => {
                self.recover(now, DaemonExitClass::ControlPlaneFailure);
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
        self.update_snapshot(|snapshot| {
            snapshot.state = state;
            snapshot.restart_attempt = attempts;
            snapshot.retry_delay_ms = retry.map(duration_millis);
            snapshot.consecutive_heartbeat_failures = heartbeat_failures;
            snapshot.blocked_reason = blocked_reason;
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
