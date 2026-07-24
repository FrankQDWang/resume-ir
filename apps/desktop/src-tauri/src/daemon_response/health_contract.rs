use serde::{Deserialize, Serialize};

pub(super) use daemon_contract::{
    CapabilityMatrix as Capabilities, CoreError, CoreHealth as CoreStatus, CoreState,
    OptionalRuntimeMatrix as OptionalRuntimes, StatusState,
};

use super::{ensure, DesktopError, SafeCount};

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProcessState {
    Ready,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RepairProgressPhase {
    Queued,
    MigrationRebuild,
    SourceUnavailable,
    Rebuilding,
    RetryWait,
    Blocked,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RepairErrorKind {
    FulltextPublicationBusy,
    FulltextFailure,
    VectorPublicationBusy,
    VectorFailure,
    MetadataFailure,
    Interrupted,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RepairProgress {
    pub(super) phase: RepairProgressPhase,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub(super) attempt: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub(super) max_attempts: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub(super) retry_after_ms: Option<SafeCount>,
    #[serde(deserialize_with = "deserialize_required_nullable")]
    pub(super) last_error_kind: Option<RepairErrorKind>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IpcMetrics {
    pub(super) accepted: SafeCount,
    pub(super) completed: SafeCount,
    pub(super) client_disconnect: SafeCount,
    pub(super) request_failure: SafeCount,
    pub(super) response_failure: SafeCount,
}

pub(super) fn validate_health_contract(
    status: StatusState,
    core: &CoreStatus,
    runtimes: &OptionalRuntimes,
    capabilities: &Capabilities,
    error: Option<&CoreError>,
) -> Result<(), DesktopError> {
    daemon_contract::validate_health_contract(
        status,
        *core,
        *runtimes,
        *capabilities,
        error.copied(),
    )
    .map_err(|_| DesktopError::new("daemon_protocol", "daemon 响应合同无效"))
}

pub(super) fn status_for_core(core: CoreState) -> StatusState {
    match core {
        CoreState::Initializing => StatusState::Initializing,
        CoreState::Ready => StatusState::Ok,
        CoreState::Repairing => StatusState::Repairing,
        CoreState::Degraded => StatusState::Degraded,
        CoreState::Blocked => StatusState::Blocked,
    }
}

pub(super) fn validate_repair_progress(
    core: CoreState,
    progress: Option<&RepairProgress>,
) -> Result<(), DesktopError> {
    if core == CoreState::Ready {
        return ensure(progress.is_none());
    }
    let Some(progress) = progress else {
        return ensure(matches!(
            core,
            CoreState::Initializing | CoreState::Degraded | CoreState::Blocked
        ));
    };
    ensure(progress.attempt.is_none_or(|value| value.value() <= 5))?;
    ensure(progress.max_attempts.is_none_or(|value| value.value() == 5))?;
    ensure(
        progress
            .retry_after_ms
            .is_none_or(|value| value.value() <= 60_000),
    )
}

pub(super) fn validate_latency(
    p50: Option<f64>,
    p95: Option<f64>,
    p99: Option<f64>,
) -> Result<(), DesktopError> {
    for value in [p50, p95, p99].into_iter().flatten() {
        ensure(value.is_finite() && (0.0..=3_600_000.0).contains(&value))?;
    }
    Ok(())
}

pub(super) fn validate_counts<const N: usize>(
    has_epoch: bool,
    has_counts: [bool; N],
) -> Result<(), DesktopError> {
    ensure(has_counts.into_iter().all(|present| present == has_epoch))
}

pub(super) fn deserialize_required_nullable<'de, D, T>(
    deserializer: D,
) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
