mod error;
mod generation;
mod metrics;
pub(crate) mod response;
mod service;

pub(crate) use error::{ConnectionOutcome, RequestFailure, ResponseSinkError};
pub(crate) use generation::IPC_PROTOCOL_VERSION;
pub(crate) use generation::{DaemonGenerationOwner, GenerationError, OwnerMode};
pub(crate) use metrics::{process_metrics, IpcMetricsSnapshot};
pub(crate) use service::{ServiceErrorCode, ServiceHealth, ServiceState};
