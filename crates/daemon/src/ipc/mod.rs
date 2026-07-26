mod capability;
mod completion;
pub(crate) mod connection;
mod diagnostics;
mod error;
mod generation;
mod metrics;
pub(crate) mod protocol;
pub(crate) mod response;
pub(crate) mod routes;
mod runtime_health;
mod runtime_state;
pub(crate) mod search_service;
pub(crate) mod server;
mod service;
mod status_updater;

pub(crate) use capability::{
    CapabilityHealth, CapabilityMatrix, CapabilityState, CoreHealth, CoreReason, CoreState,
    OptionalRuntimeHealth, OptionalRuntimeMatrix, OptionalRuntimeReason, OptionalRuntimeState,
};
pub(crate) use completion::ConnectionCompletion;
pub(crate) use error::{
    ConnectionOutcome, DaemonFatalError, RequestFailure, ResponseSinkError, RuntimeEvent,
};
pub(crate) use generation::IPC_PROTOCOL_VERSION;
pub(crate) use generation::{DaemonGenerationOwner, GenerationError, OwnerMode};
pub(crate) use metrics::process_metrics;
pub(crate) use runtime_health::{
    runtime_health_channel, RuntimeHealthReceiver, RuntimeHealthReporter, RuntimeHealthUpdate,
};
pub(crate) use runtime_state::{ControlPlanePublisher, ControlPlaneState};
pub(crate) use service::{repair_progress_json, ServiceErrorCode};
