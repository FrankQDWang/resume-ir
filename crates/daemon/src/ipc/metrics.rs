use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

use super::{ConnectionOutcome, RequestFailure};

static PROCESS_METRICS: LazyLock<IpcMetrics> = LazyLock::new(IpcMetrics::default);

#[derive(Default)]
pub(crate) struct IpcMetrics {
    accepted: AtomicU64,
    completed: AtomicU64,
    client_disconnect: AtomicU64,
    request_failure: AtomicU64,
    response_failure: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct IpcMetricsSnapshot {
    pub(crate) accepted: u64,
    pub(crate) completed: u64,
    pub(crate) client_disconnect: u64,
    pub(crate) request_failure: u64,
    pub(crate) response_failure: u64,
}

pub(crate) fn process_metrics() -> &'static IpcMetrics {
    &PROCESS_METRICS
}

impl IpcMetrics {
    pub(crate) fn record_accepted(&self) {
        saturating_increment(&self.accepted);
    }

    pub(crate) fn record_connection_outcome(&self, outcome: ConnectionOutcome) {
        match outcome {
            ConnectionOutcome::Completed => saturating_increment(&self.completed),
            ConnectionOutcome::RequestFailed(RequestFailure::Handler) => {
                saturating_increment(&self.request_failure);
            }
            ConnectionOutcome::ClientDisconnected(error)
            | ConnectionOutcome::RequestFailed(RequestFailure::ResponseSink(error)) => {
                saturating_increment(&self.response_failure);
                if error.client_disconnected() {
                    saturating_increment(&self.client_disconnect);
                }
            }
            ConnectionOutcome::Deferred => {}
        }
    }

    pub(crate) fn snapshot(&self) -> IpcMetricsSnapshot {
        IpcMetricsSnapshot {
            accepted: self.accepted.load(Ordering::Relaxed),
            completed: self.completed.load(Ordering::Relaxed),
            client_disconnect: self.client_disconnect.load(Ordering::Relaxed),
            request_failure: self.request_failure.load(Ordering::Relaxed),
            response_failure: self.response_failure.load(Ordering::Relaxed),
        }
    }
}

fn saturating_increment(counter: &AtomicU64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        Some(value.saturating_add(1))
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::IpcMetrics;
    use crate::ipc::{ConnectionOutcome, RequestFailure, ResponseSinkError};

    #[test]
    fn terminal_outcomes_are_bounded_and_classified() {
        let metrics = IpcMetrics::default();
        metrics.record_accepted();
        metrics
            .record_connection_outcome(ConnectionOutcome::RequestFailed(RequestFailure::Handler));
        metrics.record_accepted();
        metrics.record_connection_outcome(ConnectionOutcome::ClientDisconnected(
            ResponseSinkError::ClientDisconnected,
        ));
        assert_eq!(metrics.snapshot().accepted, 2);
        assert_eq!(metrics.snapshot().request_failure, 1);
        assert_eq!(metrics.snapshot().response_failure, 1);
        assert_eq!(metrics.snapshot().client_disconnect, 1);
    }

    #[test]
    fn counters_saturate_instead_of_wrapping() {
        let metrics = IpcMetrics::default();
        metrics.accepted.store(u64::MAX, Ordering::Relaxed);
        metrics.record_accepted();
        assert_eq!(metrics.snapshot().accepted, u64::MAX);
    }
}
