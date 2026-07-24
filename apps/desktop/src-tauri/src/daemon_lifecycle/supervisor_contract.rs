use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: &str = "resume-ir.desktop-daemon-lifecycle.v2";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DaemonLifecycleKind {
    Starting,
    Running,
    RetryWait,
    CircuitOpen,
    Blocked,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DaemonTransitionReason {
    InitialStart,
    AutomaticRetry,
    ManualRetry,
    ControlPlaneReady,
    ChildExited,
    StartupTimeout,
    HeartbeatTimeout,
    StartFailed,
    ControlPlaneFailure,
    RestartBudgetExhausted,
    HalfOpenRetry,
    ConfigurationInvalid,
    RuntimeIntegrity,
    ProtocolMismatch,
    OwnershipConflict,
    SupervisorUnavailable,
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

impl From<DaemonBlockedReason> for DaemonTransitionReason {
    fn from(value: DaemonBlockedReason) -> Self {
        match value {
            DaemonBlockedReason::ConfigurationInvalid => Self::ConfigurationInvalid,
            DaemonBlockedReason::RuntimeIntegrity => Self::RuntimeIntegrity,
            DaemonBlockedReason::ProtocolMismatch => Self::ProtocolMismatch,
            DaemonBlockedReason::OwnershipConflict => Self::OwnershipConflict,
            DaemonBlockedReason::SupervisorUnavailable => Self::SupervisorUnavailable,
        }
    }
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

impl From<DaemonExitClass> for DaemonTransitionReason {
    fn from(value: DaemonExitClass) -> Self {
        match value {
            DaemonExitClass::ChildExited => Self::ChildExited,
            DaemonExitClass::StartupTimeout => Self::StartupTimeout,
            DaemonExitClass::HeartbeatTimeout => Self::HeartbeatTimeout,
            DaemonExitClass::StartFailed => Self::StartFailed,
            DaemonExitClass::ControlPlaneFailure => Self::ControlPlaneFailure,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct DaemonLifecycleSnapshot {
    pub(super) schema_version: &'static str,
    pub(super) state: DaemonLifecycleKind,
    pub(super) transition_reason: DaemonTransitionReason,
    pub(super) generation: u64,
    pub(super) automatic_restart_attempt: u8,
    pub(super) automatic_restart_limit: u8,
    pub(super) retry_after_ms: Option<u64>,
    pub(super) heartbeat_failures: u8,
    pub(super) last_exit: Option<DaemonExitClass>,
}

impl DaemonLifecycleSnapshot {
    pub(super) fn initial() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            state: DaemonLifecycleKind::Starting,
            transition_reason: DaemonTransitionReason::InitialStart,
            generation: 0,
            automatic_restart_attempt: 0,
            automatic_restart_limit: 5,
            retry_after_ms: None,
            heartbeat_failures: 0,
            last_exit: None,
        }
    }

    pub(super) fn supervisor_unavailable() -> Self {
        Self {
            state: DaemonLifecycleKind::Blocked,
            transition_reason: DaemonTransitionReason::SupervisorUnavailable,
            ..Self::initial()
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadyDaemonIdentity {
    pub(crate) supervisor_generation: u64,
    pub(crate) launch_id: String,
}
