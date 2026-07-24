use std::fs;
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::policy::{RecoveryDecision, RestartPolicy, RestartPolicyConfig};
use super::receipt::LifecycleReceiptRecorder;
pub(crate) use super::supervisor_contract::{
    DaemonBlockedReason, DaemonExitClass, DaemonLifecycleKind, DaemonLifecycleSnapshot,
    DaemonTransitionReason, ReadyDaemonIdentity,
};
use crate::daemon_client::DesktopError;

const LEGACY_RESTART_LEDGER: &str = "desktop-daemon-restart-window.v1.json";

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
    fn launch_id(&self) -> &str;
    fn probe(&mut self, timeout: Duration) -> DaemonProbe;
    fn poll_exit(&mut self) -> ChildExitOutcome;
    fn stop(self);
}

pub(super) trait DaemonRuntime: Send + 'static {
    type Child: SupervisedChild;

    fn spawn(&mut self) -> Result<Self::Child, RuntimeFailure>;
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
    Retry(mpsc::Sender<Result<DaemonLifecycleSnapshot, DesktopError>>),
    Stop(mpsc::Sender<()>),
}

pub(crate) struct DaemonLifecycleState {
    commands: mpsc::SyncSender<SupervisorCommand>,
    snapshot: Arc<Mutex<DaemonLifecycleSnapshot>>,
    ready_identity: Arc<Mutex<Option<ReadyDaemonIdentity>>>,
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

    #[cfg(test)]
    fn launch_with_timing_and_data_dir<R: DaemonRuntime>(
        runtime: R,
        timing: SupervisorTiming,
        data_dir: &std::path::Path,
    ) -> Result<Self, DesktopError> {
        Self::launch_with_timing_and_receipt(
            runtime,
            timing,
            LifecycleReceiptRecorder::initialize(data_dir),
        )
    }

    fn launch_with_timing_and_receipt<R: DaemonRuntime>(
        runtime: R,
        timing: SupervisorTiming,
        receipt: LifecycleReceiptRecorder,
    ) -> Result<Self, DesktopError> {
        if let Some(data_dir) = receipt.data_dir() {
            ignore_legacy_restart_ledger(data_dir);
        }
        let (commands, receiver) = mpsc::sync_channel(8);
        let snapshot = Arc::new(Mutex::new(DaemonLifecycleSnapshot::initial()));
        let ready_identity = Arc::new(Mutex::new(None));
        let receipt = Arc::new(receipt);
        let thread_snapshot = Arc::clone(&snapshot);
        let thread_identity = Arc::clone(&ready_identity);
        let thread_receipt = Arc::clone(&receipt);
        let thread = thread::Builder::new()
            .name("resume-daemon-supervisor".to_string())
            .spawn(move || {
                SupervisorActor::new(
                    runtime,
                    timing,
                    thread_snapshot,
                    thread_identity,
                    thread_receipt,
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
            ready_identity,
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

    pub(crate) fn ready_identity(&self) -> Option<ReadyDaemonIdentity> {
        let lifecycle_is_running = self
            .snapshot
            .lock()
            .is_ok_and(|snapshot| snapshot.state == DaemonLifecycleKind::Running);
        lifecycle_is_running
            .then(|| {
                self.ready_identity
                    .lock()
                    .ok()
                    .and_then(|value| value.clone())
            })
            .flatten()
    }

    pub(crate) fn retry(&self) -> Result<DaemonLifecycleSnapshot, DesktopError> {
        let (sender, receiver) = mpsc::channel();
        self.commands
            .try_send(SupervisorCommand::Retry(sender))
            .map_err(|_| DesktopError::new("daemon_supervisor_busy", "本地 daemon 监督器繁忙"))?;
        receiver
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| DesktopError::internal())?
    }

    pub(crate) fn diagnostics(
        &self,
        daemon_diagnostics: Option<&crate::daemon_response::DiagnosticsBody>,
    ) -> Result<Vec<u8>, DesktopError> {
        self.receipt.diagnostics(daemon_diagnostics)
    }

    pub(crate) fn shutdown(&self) {
        let handle = self.thread.lock().ok().and_then(|mut handle| handle.take());
        let Some(handle) = handle else { return };
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
        let Some(handle) = handle else { return };
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
    Running {
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
    ready_identity: Arc<Mutex<Option<ReadyDaemonIdentity>>>,
    receipt: Arc<LifecycleReceiptRecorder>,
    next_start_reason: DaemonTransitionReason,
}

impl<R: DaemonRuntime> SupervisorActor<R> {
    fn new(
        runtime: R,
        timing: SupervisorTiming,
        snapshot: Arc<Mutex<DaemonLifecycleSnapshot>>,
        ready_identity: Arc<Mutex<Option<ReadyDaemonIdentity>>>,
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
            ready_identity,
            receipt,
            next_start_reason: DaemonTransitionReason::InitialStart,
        }
    }

    fn run(mut self, receiver: mpsc::Receiver<SupervisorCommand>) {
        loop {
            match receiver.recv_timeout(self.timing.tick) {
                Ok(SupervisorCommand::Retry(reply)) => {
                    let result = self.manual_retry().map(|()| self.current_snapshot());
                    let _ = reply.send(result);
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
            ActorPhase::Running {
                heartbeat_at,
                heartbeat_failures,
            } => self.observe_running(now, heartbeat_at, heartbeat_failures),
            ActorPhase::CircuitOpen { retry_at } => {
                let remaining = self
                    .policy
                    .manual_half_open_retry_after(self.elapsed(now))
                    .unwrap_or_else(|| retry_at.saturating_duration_since(now));
                self.update_snapshot(|snapshot| {
                    snapshot.retry_after_ms =
                        Some(duration_millis(remaining).max(u64::from(!remaining.is_zero())));
                });
            }
            ActorPhase::Waiting { .. } | ActorPhase::Blocked => {}
        }
    }

    fn attempt_start(&mut self, now: Instant) {
        match self.runtime.spawn() {
            Ok(child) => {
                self.child = Some(child);
                self.phase = ActorPhase::Starting {
                    deadline: now + self.timing.startup_deadline,
                };
                self.publish_state(
                    DaemonLifecycleKind::Starting,
                    self.next_start_reason,
                    None,
                    0,
                );
            }
            Err(RuntimeFailure::Blocked(reason)) => self.block(reason),
            Err(RuntimeFailure::Transient) => self.recover(DaemonExitClass::StartFailed),
        }
    }

    fn observe_starting(&mut self, now: Instant, deadline: Instant) {
        if self.handle_child_exit() {
            return;
        }
        let probe = self
            .child
            .as_mut()
            .map_or(DaemonProbe::Unavailable, |child| {
                child.probe(self.timing.heartbeat_timeout)
            });
        match probe {
            DaemonProbe::Ready => self.enter_running(now),
            // The child probe already maps foreign and wrong-launch discovery to
            // Unavailable. A protocol mismatch here is therefore attributable to
            // this exact launch and must fail closed immediately.
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

    fn enter_running(&mut self, now: Instant) {
        self.policy.on_ready(self.elapsed(now));
        let launch_id = self
            .child
            .as_ref()
            .map(|child| child.launch_id().to_owned())
            .unwrap_or_default();
        self.phase = ActorPhase::Running {
            heartbeat_at: now + self.timing.heartbeat_interval,
            heartbeat_failures: 0,
        };
        self.update_snapshot(|snapshot| {
            snapshot.state = DaemonLifecycleKind::Running;
            snapshot.transition_reason = DaemonTransitionReason::ControlPlaneReady;
            snapshot.generation = snapshot.generation.saturating_add(1);
            snapshot.retry_after_ms = None;
            snapshot.heartbeat_failures = 0;
        });
        let generation = self.current_snapshot().generation;
        if let Ok(mut identity) = self.ready_identity.lock() {
            *identity = Some(ReadyDaemonIdentity {
                supervisor_generation: generation,
                launch_id,
            });
        }
        self.record_snapshot();
    }

    fn observe_running(&mut self, now: Instant, heartbeat_at: Instant, heartbeat_failures: u8) {
        if self.handle_child_exit() || now < heartbeat_at {
            return;
        }
        let probe = self
            .child
            .as_mut()
            .map_or(DaemonProbe::Unavailable, |child| {
                child.probe(self.timing.heartbeat_timeout)
            });
        match probe {
            DaemonProbe::Ready => {
                self.phase = ActorPhase::Running {
                    heartbeat_at: now + self.timing.heartbeat_interval,
                    heartbeat_failures: 0,
                };
                self.update_snapshot(|snapshot| snapshot.heartbeat_failures = 0);
                if self.policy.stable_reset_due(self.elapsed(now)) {
                    self.policy.complete_stable_reset(self.elapsed(now));
                    let attempts = self.policy.restart_attempts(self.elapsed(now));
                    self.update_snapshot(|snapshot| snapshot.automatic_restart_attempt = attempts);
                    self.record_snapshot();
                }
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
                    self.phase = ActorPhase::Running {
                        heartbeat_at: now + self.timing.heartbeat_interval,
                        heartbeat_failures: failures,
                    };
                    self.update_snapshot(|snapshot| snapshot.heartbeat_failures = failures);
                    self.record_snapshot();
                }
            }
        }
    }

    fn recover(&mut self, exit: DaemonExitClass) {
        self.child = None;
        self.clear_ready_identity();
        self.update_snapshot(|snapshot| snapshot.last_exit = Some(exit));
        let now = Instant::now();
        match self.policy.on_failure(self.elapsed(now)) {
            RecoveryDecision::RetryAfter(delay) => {
                self.next_start_reason = DaemonTransitionReason::AutomaticRetry;
                self.phase = ActorPhase::Waiting {
                    start_at: now + delay,
                };
                self.publish_state(DaemonLifecycleKind::RetryWait, exit.into(), Some(delay), 0);
            }
            RecoveryDecision::OpenCircuit(delay) => {
                self.phase = ActorPhase::CircuitOpen {
                    retry_at: now + delay,
                };
                self.publish_state(
                    DaemonLifecycleKind::CircuitOpen,
                    DaemonTransitionReason::RestartBudgetExhausted,
                    Some(delay),
                    0,
                );
            }
        }
    }

    fn block(&mut self, reason: DaemonBlockedReason) {
        self.child = None;
        self.clear_ready_identity();
        self.phase = ActorPhase::Blocked;
        self.publish_state(DaemonLifecycleKind::Blocked, reason.into(), None, 0);
    }

    fn manual_retry(&mut self) -> Result<(), DesktopError> {
        let now = Instant::now();
        match self.phase {
            ActorPhase::Blocked => {}
            ActorPhase::CircuitOpen { retry_at }
                if now >= retry_at && self.policy.begin_manual_half_open(self.elapsed(now)) => {}
            ActorPhase::CircuitOpen { .. } => {
                return Err(retry_not_allowed());
            }
            ActorPhase::Waiting { .. }
            | ActorPhase::Starting { .. }
            | ActorPhase::Running { .. } => {
                return Err(retry_not_allowed());
            }
        }
        self.next_start_reason = DaemonTransitionReason::ManualRetry;
        self.phase = ActorPhase::Waiting { start_at: now };
        self.publish_state(
            DaemonLifecycleKind::Starting,
            DaemonTransitionReason::ManualRetry,
            None,
            0,
        );
        Ok(())
    }

    fn handle_child_exit(&mut self) -> bool {
        let outcome = self
            .child
            .as_mut()
            .map_or(ChildExitOutcome::Exited, SupervisedChild::poll_exit);
        match outcome {
            ChildExitOutcome::Running => false,
            ChildExitOutcome::Exited => {
                self.child = None;
                self.recover(DaemonExitClass::ChildExited);
                true
            }
            ChildExitOutcome::RestartableFatal => {
                self.child = None;
                self.recover(DaemonExitClass::ControlPlaneFailure);
                true
            }
            ChildExitOutcome::Blocked(reason) => {
                self.child = None;
                self.block(reason);
                true
            }
        }
    }

    fn stop_child(&mut self) {
        if let Some(child) = self.child.take() {
            child.stop()
        }
        self.clear_ready_identity();
    }

    fn clear_ready_identity(&self) {
        if let Ok(mut identity) = self.ready_identity.lock() {
            *identity = None
        }
    }

    fn elapsed(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started_at)
    }

    fn publish_state(
        &mut self,
        state: DaemonLifecycleKind,
        reason: DaemonTransitionReason,
        retry_after: Option<Duration>,
        heartbeat_failures: u8,
    ) {
        if state != DaemonLifecycleKind::Running {
            self.clear_ready_identity()
        }
        let now = Instant::now();
        let attempts = self.policy.restart_attempts(self.elapsed(now));
        self.update_snapshot(|snapshot| {
            snapshot.state = state;
            snapshot.transition_reason = reason;
            snapshot.automatic_restart_attempt = attempts;
            snapshot.retry_after_ms = retry_after.map(duration_millis);
            snapshot.heartbeat_failures = heartbeat_failures;
        });
        self.record_snapshot();
    }

    fn update_snapshot(&self, update: impl FnOnce(&mut DaemonLifecycleSnapshot)) {
        if let Ok(mut snapshot) = self.snapshot.lock() {
            update(&mut snapshot)
        }
    }

    fn current_snapshot(&self) -> DaemonLifecycleSnapshot {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_else(|_| DaemonLifecycleSnapshot::supervisor_unavailable())
    }

    fn record_snapshot(&self) {
        self.receipt.record(&self.current_snapshot())
    }
}

fn ignore_legacy_restart_ledger(data_dir: &std::path::Path) {
    let path = data_dir.join(LEGACY_RESTART_LEDGER);
    if fs::symlink_metadata(&path)
        .is_ok_and(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
    {
        let _ = fs::remove_file(path);
    }
}

fn retry_not_allowed() -> DesktopError {
    DesktopError::new(
        "retry_not_allowed",
        "当前 daemon 生命周期状态不允许人工重试",
    )
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "supervisor_tests.rs"]
mod tests;
