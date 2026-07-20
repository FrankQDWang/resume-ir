mod completion;
pub(crate) mod connection;
mod diagnostics;
mod error;
mod generation;
mod metrics;
pub(crate) mod protocol;
pub(crate) mod response;
pub(crate) mod routes;
pub(crate) mod search_service;
pub(crate) mod server;
mod service;

pub(crate) use completion::ConnectionCompletion;
pub(crate) use error::{
    ConnectionOutcome, DaemonFatalError, RequestFailure, ResponseSinkError, RuntimeEvent,
};
pub(crate) use generation::IPC_PROTOCOL_VERSION;
pub(crate) use generation::{DaemonGenerationOwner, GenerationError, OwnerMode};
pub(crate) use metrics::{process_metrics, IpcMetricsSnapshot};
pub(crate) use service::{
    projection_service_health, repair_progress_json, search_repair_reason_label,
    service_error_json, ServiceErrorCode, ServiceHealth, ServiceState,
};
