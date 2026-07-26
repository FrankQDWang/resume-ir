use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::metrics::{process_metrics, IpcMetrics};
use super::{ConnectionOutcome, RequestFailure};

/// The exactly-once terminal capability for one accepted connection.
///
/// Synchronous connections are completed by the connection handler. Search
/// connections move a clone into their asynchronous response owner. If every
/// owner disappears without an explicit outcome, `Drop` records a request
/// failure.
#[derive(Clone)]
pub(crate) struct ConnectionCompletion {
    inner: Arc<ConnectionCompletionInner>,
}

struct ConnectionCompletionInner {
    metrics: &'static IpcMetrics,
    finished: AtomicBool,
    deferred: AtomicBool,
}

impl ConnectionCompletion {
    pub(crate) fn accepted() -> Self {
        let metrics = process_metrics();
        metrics.record_accepted();
        Self {
            inner: Arc::new(ConnectionCompletionInner {
                metrics,
                finished: AtomicBool::new(false),
                deferred: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn defer(&self) -> Self {
        self.inner.deferred.store(true, Ordering::Release);
        self.clone()
    }

    pub(crate) fn was_deferred(&self) -> bool {
        self.inner.deferred.load(Ordering::Acquire)
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.inner.finished.load(Ordering::Acquire)
    }

    pub(crate) fn finish(&self, outcome: ConnectionOutcome) {
        if outcome == ConnectionOutcome::Deferred
            || self.inner.finished.swap(true, Ordering::AcqRel)
        {
            return;
        }
        self.inner.metrics.record_connection_outcome(outcome);
    }
}

impl Drop for ConnectionCompletionInner {
    fn drop(&mut self) {
        if !self.finished.swap(true, Ordering::AcqRel) {
            self.metrics
                .record_connection_outcome(ConnectionOutcome::RequestFailed(
                    RequestFailure::Handler,
                ));
        }
    }
}
